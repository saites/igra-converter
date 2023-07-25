use eddie::DamerauLevenshtein;
use phf::{phf_map, phf_set};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{hash_map, HashMap};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::iter::zip;
use std::ops::Deref;

use crate::bktree;
use crate::bktree::BKTree;
use crate::robin::EventID::Known;
use crate::robin::{Event, EventID, Registration};
use crate::xbase::{DBaseRecord, DBaseResult, Decimal, Field, Header, TableReader, FieldDescriptor, FieldType};

pub fn read_reg<P: AsRef<std::path::Path>>(path: P) -> Result<Vec<Registration>, Box<dyn Error>> {
    Ok(serde_json::from_reader(BufReader::new(File::open(path)?))?)
}

/// Implements a Damerau-Levenshtein metric on new-type versions of borrowed PersonRecords,
/// allowing the code below to generate metric trees from the database data using different fields.
/// Those trees are used to find and rank nearby records when an exact target cannot be found.
///
/// The syntax is `damlev_metric_impl! { MyNewType (some_record_property) }`,
/// which creates the wrapper struct around `&'a PersonRecord`
/// with a `Metric::dist` function that returns the distance between instances of the property.
macro_rules! damlev_metric_impl {
    (
        $name:ident (
            $field:ident
        )
    ) => {
        pub struct $name<'a>(&'a PersonRecord);

        impl<'a> bktree::Metric for $name<'a> {
            type Output = usize;

            fn dist(&self, x: &Self) -> usize {
                let damlev = DamerauLevenshtein::new();
                damlev.distance(&self.0.$field, &x.0.$field)
            }
        }
    };
}

damlev_metric_impl! { ByIGRANum (igra_number) }
damlev_metric_impl! { ByFirstName (legal_first) }
damlev_metric_impl! { ByLastName (legal_last) }
damlev_metric_impl! { ByPerformanceFirst(first_name) }
damlev_metric_impl! { ByPerformanceLast(last_name) }

// TODO: store a full name field on PersonRecord
//  and create a metric for that.

/// Counts the number of times a key is inserted and tracks the sum of their distances.
struct DistCounter<T>(HashMap<T, (u64, usize)>);

impl<T> DistCounter<T>
    where
        T: std::hash::Hash + Eq,
{
    fn new() -> Self {
        DistCounter(HashMap::<T, (u64, usize)>::new())
    }

    /// Insert T with the given distance.
    /// If T is already present, adds the distance to its sum.
    fn insert(&mut self, dist: usize, pr: T) {
        self.0
            .entry(pr)
            .and_modify(|(hits, dist_sum)| {
                *hits = hits.saturating_add(1);
                *dist_sum = dist_sum.saturating_add(dist);
            })
            .or_insert((1, dist));
    }

    /// Consume the map and extract the best values.
    ///
    /// This filters out anything with fewer than min_hits
    /// and anything farther than max_dist_sum.
    /// Entries that appear more often appear first,
    /// and when tied for number of hits, closer values are first.
    ///
    /// The returned vector holds tuples of the form `(match, (hits, total distance))`.
    fn best(self, min_hits: u64, max_dist_sum: Option<usize>) -> Vec<(T, (u64, usize))> {
        let mut best: Vec<_> = if let Some(max_dist_sum) = max_dist_sum {
            self.0
                .into_iter()
                .filter(|(_, (hits, d_sum))| *hits >= min_hits && *d_sum <= max_dist_sum)
                .collect()
        } else {
            self.0
                .into_iter()
                .filter(|(_, (hits, _))| *hits >= min_hits)
                .collect()
        };

        best.sort_by(|(_, (hits0, d_sum0)), (_, (hits1, d_sum1))| {
            let h_cmp = hits1.cmp(hits0); // prefer more hits
            if h_cmp.is_eq() {
                d_sum0.cmp(d_sum1)
            } else {
                h_cmp
            } // and less distance
        });

        best
    }
}

impl<T> Deref for DistCounter<T> {
    type Target = HashMap<T, (u64, usize)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Performs validations on event entries using the current person database.
///
/// Does the registrant claim to be a member?
/// - If no, look for records with similar names, DoB, SSN
///   & highlight them as possible matches.
/// - If yes, look for a matching record based on all of
///   IGRA # + First Name + Last Name + DoB + SSN
///   - If we have a match, check for updates to
///     Association, Performance name, Address details.
///   - If no, measure distance from existing entries,
///     and propose entries within some bounds, ordered by distance
///     (same as if they claim not to be a member, but includes IGRA #).
/// - Have they registered for at least 2 events?
/// - For events involving partners
///   - Did they enter the correct number of values?
///   - For each value entered, can we find a matching member
///     based on IGRA #, name, or a combination?
///   - For found listed partners, did that partner register & list this person?
pub struct EntryValidator<'a> {
    by_igra_num: BKTree<ByIGRANum<'a>, usize>,
    by_first_name: BKTree<ByFirstName<'a>, usize>,
    by_last_name: BKTree<ByLastName<'a>, usize>,
    by_perf_first: BKTree<ByPerformanceFirst<'a>, usize>,
    by_perf_last: BKTree<ByPerformanceLast<'a>, usize>,

    damlev: DamerauLevenshtein,
}

/// This is the report structure returned from validation.
#[derive(Debug, Serialize)]
pub struct Report<'a> {
    /// These are the processed entries, in the same order as the incoming entries.
    pub results: Vec<Processed<'a>>,
    /// This is a collection of IGRA number -> record for relevant records.
    ///
    /// Relevant records include entries matched to a single record
    /// as well as close matches to information given in entries.
    pub relevant: HashMap<&'a str, &'a PersonRecord>,
}

/// The interface asks for "PARTNER NAME | IGRA #",
/// so this split a partner string s into (Some(igra_number), name)
/// if the string starts or ends with digits, or (None, name) otherwise.
///
/// The name portion will hold the (possibly empty) remaining string,
/// stripped of whitespace and '|'.
pub fn split_partner(s: &str) -> (Option<&str>, &str) {
    fn ignored(c: char) -> bool {
        c == '|' || c.is_whitespace()
    }
    
    let s = s.trim_matches(ignored);
    if s.is_empty() {
        return (None, s);
    }

    let name_start = s.find(|c: char| !c.is_ascii_digit());
    let name_end = s.rfind(|c: char| !c.is_ascii_digit());
    match (name_start, name_end) {
        (Some(start), _) if start > 0 => {
            let (num, name) = s.split_at(start);
            (Some(num.trim()), name.trim_matches(ignored))
        }
        (_, Some(end)) if end < s.len() - 1 => {
            let (name, num) = s.split_at(end);
            (Some(num.trim()), name.trim_matches(ignored))
        }
        (None, None) => (Some(s), &""),
        _ => (None, s.trim_matches(ignored)),
    }
}

impl<'a> EntryValidator<'a> {
    pub(crate) fn new(people: &'a Vec<PersonRecord>) -> Self {
        let mut ev = EntryValidator {
            by_igra_num: BKTree::new(),
            by_first_name: BKTree::new(),
            by_last_name: BKTree::new(),
            by_perf_first: BKTree::new(),
            by_perf_last: BKTree::new(),
            damlev: DamerauLevenshtein::new(),
        };

        for p in people {
            ev.by_igra_num.insert(ByIGRANum(&p));
            ev.by_first_name.insert(ByFirstName(&p));
            ev.by_last_name.insert(ByLastName(&p));
            ev.by_perf_first.insert(ByPerformanceFirst(&p));
            ev.by_perf_last.insert(ByPerformanceLast(&p));
        }

        ev
    }

    /// Validates the registration entries against the people database.
    pub fn validate_entries(&self, entries: &'a Vec<Registration>) -> Report<'a> {
        let today = chrono::Utc::now().naive_utc().date();

        let mut results: Vec<Processed> = Vec::with_capacity(entries.len());
        let mut relevant = HashMap::<&str, &PersonRecord>::new();

        for r in entries {
            let mut p = Processed::new(&r);

            // Validate their age is at least 18.
            if r.contestant
                .dob
                .naive_date()
                .and_then(|d| today.years_since(d))
                .map_or(true, |age| age < 18)
            {
                p.issues.push(Suggestion {
                    problem: Problem::NotOldEnough,
                    fix: Fix::ContactRegistrant,
                });
            }

            // Make sure they registered for at least two go-rounds.
            if r.events.len() < 2 {
                p.issues.push(Suggestion {
                    problem: Problem::NotEnoughRounds,
                    fix: Fix::ContactRegistrant,
                });
            }

            self.validate_events(&mut p, &mut relevant);
            self.find_registrant(&mut p, &mut relevant);

            // Collect known partners into a list to make them easier to display.
            p.partners = p
                .confirmed_partners
                .iter()
                .flat_map(|(person, events)| {
                    events.iter().map(|(event, round, index)| Partner {
                        igra_number: &person.igra_number,
                        event: *event,
                        round: *round,
                        index: *index,
                    })
                })
                .collect();

            results.push(p);
        }

        // Find cross-registration issues.
        let mut more_issues: Vec<Vec<Suggestion>> = results
            .iter()
            .filter_map(|result| result.found.and_then(|f| relevant.get(f)).zip(Some(result)))
            .map(|(person_a, entry_a)| validate_cross_reg(&results, person_a, entry_a))
            .collect();

        // We can't mutate the results in the above code
        // because we need to borrow them again to find other records;
        // hence, we need another iteration to insert the found issues.
        for (v, mi) in zip(&mut results.iter_mut().filter(|r| r.found.is_some()), &mut more_issues) {
            // If we're going to recommend adding/using non-registered people,
            // add their data to the relevance collection.
            for sugg in mi.iter() {
                let other = match &sugg.fix {
                    Fix::UseThisRecord(igra_num) | Fix::AddRegistration(igra_num) => self
                        .by_igra_num
                        .find_closest(0, |r| self.damlev.distance(&r.0.igra_number, &igra_num.0)),
                    _ => None,
                };

                if let Some((0, o)) = other {
                    relevant.insert(&o.0.igra_number, o.0);
                }
            }

            v.issues.append(mi);
        }

        Report { results, relevant }
    }

    fn validate_events(
        &self,
        proc: &mut Processed<'a>,
        relevant: &mut HashMap<&'a str, &'a PersonRecord>,
    ) {
        for event in &proc.registration.events {
            if event.round > 2 {
                proc.issues.push(Suggestion {
                    problem: Problem::InvalidRoundID {
                        event: event.id,
                        round: event.round,
                    },
                    fix: Fix::ContactDevelopers,
                });
            }

            let db_event = if let Known(expected) = event.id {
                expected
            } else {
                // We don't have this event mapping.
                proc.issues.push(Suggestion {
                    problem: Problem::UnknownEventID { event: event.id },
                    fix: Fix::ContactDevelopers,
                });
                continue;
            };

            self.validate_partners(proc, &event, db_event, relevant);
        }
    }

    /// Attempt to find close matches based on basic information we have.
    ///
    /// Returns a boolean representing a "perfect" match and a collection of best matches.
    /// If perfect is true, the collection will have exactly one record.
    /// The converse is not true in general: a single match may not be perfect.
    /// Note that the collection may be empty.
    ///
    /// With an IGRA number, a perfect match must have a matching number,
    /// and if any of the name fields are non-empty, further consideration is necessary.
    /// With all empty names, a matching IGRA number is considered a perfect match.
    /// Otherwise, potential matches must also satisfy the name requirements below.
    ///
    /// Without an IGRA number, a perfect match depends on what names are non-empty,
    /// what they match in the database, and whether we have multiple potential hits.
    ///
    /// Which names are non-empty determines whether we can distinguish legal vs performance names.
    /// If performance is non-empty, we attempt to split it at the first space and trim its parts;
    /// if it can't be split, then it's taken as the first name with the last name empty.
    /// When both are set, they each must match their respective fields.
    /// When only first and last are set, they must match the legal first/last names.
    /// If only performance is set, it must match _either_ legal or performance names.
    pub fn find_person<'b>(&'b self, igra_num: Option<&str>, first: &str, last: &str, performance: &str)
                   -> (bool, Vec<&'a PersonRecord>) {
        let first = first.trim();
        let last = last.trim();
        let (p_first, p_last) = performance.split_once(' ')
            .map(|(f, l)| (f.trim(), l.trim()))
            .unwrap_or((performance.trim(), &""));

        let is_perfect = |found: &PersonRecord| {
            match (first.is_empty() && last.is_empty(), performance.is_empty()) {
                (true, true) => {
                    // If we don't have any names, only match against IGRA number.
                    // If we don't even have that, then what are you searching for?
                    igra_num.is_some_and(|s| s.trim() == found.igra_number)
                },
                (false, true) => {
                    // Only have a separated first and last name: try to match legal.
                    first.eq_ignore_ascii_case(&found.legal_first)
                        && last.eq_ignore_ascii_case(&found.legal_last)
                }
                (true, false) => {
                    // Only have a combined name: try to match either field.
                    (p_first.eq_ignore_ascii_case(&found.legal_first)
                        && p_last.eq_ignore_ascii_case(&found.legal_last)) 
                    || (p_first.eq_ignore_ascii_case(&found.first_name)
                        && p_last.eq_ignore_ascii_case(&found.last_name))
                }
                (false, false) => {
                    // Have both, so require both field sets to match.
                    first.eq_ignore_ascii_case(&found.legal_first)
                        && last.eq_ignore_ascii_case(&found.legal_last)
                        && p_first.eq_ignore_ascii_case(&found.first_name)
                        && p_last.eq_ignore_ascii_case(&found.last_name)
                }
            }
        };

        let mut exp_hits = 0;

        // When we have an IGRA number, try to take the fast path if possible.
        // With a search distance of 0, we'll expand very few nodes,
        // so an exact match can be verified very quickly.
        let mut p_finder = if let Some(ref igra_num) = igra_num {
            if let Some((_, found)) = self.by_igra_num.find_closest(
                0, |x| self.damlev.distance(igra_num, &x.0.igra_number)) {

                // Return early if we consider this a perfect match.
                if is_perfect(found.0) {
                    return (true, vec![found.0]);
                }
            }

            // Otherwise, we'll need to make a suggestion.
            let mut p_finder = DistCounter::<&PersonRecord>::new();
            self.by_igra_num
                .find_by(1, |x| self.damlev.distance(igra_num, &x.0.igra_number))
                .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
            exp_hits += 1;
            p_finder
        } else {
            DistCounter::<&PersonRecord>::new()
        };

        let search_dist = 3;
        if !first.is_empty() {
            let first = first.to_ascii_uppercase();
            self.by_first_name
                .find_by(search_dist, |x| self.damlev.distance(&first, &x.0.legal_first))
                .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
            exp_hits += 1;
        }

        if !last.is_empty() {
            let last = last.to_ascii_uppercase();
            self.by_last_name
                .find_by(search_dist, |x| self.damlev.distance(&last, &x.0.legal_last))
                .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
            exp_hits += 1;
        }

        if !performance.is_empty() {
            let p_first = p_first.to_ascii_uppercase();
            let p_last = if !p_last.is_empty() {
                p_last.to_ascii_uppercase()
            } else {
                p_first.clone()
            };

            self.by_perf_first
                .find_by(search_dist, |x| self.damlev.distance(&p_first, &x.0.first_name))
                .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
            self.by_perf_last
                .find_by(search_dist, |x| self.damlev.distance(&p_last, &x.0.last_name))
                .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));

            if first.is_empty() && last.is_empty() {
                self.by_first_name
                    .find_by(search_dist, |x| self.damlev.distance(&p_first, &x.0.legal_first))
                    .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
                self.by_last_name
                    .find_by(search_dist, |x| self.damlev.distance(&p_last, &x.0.legal_last))
                    .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
                exp_hits += 2;
            }
        }

        let mut possible: Vec<_> = p_finder.best(exp_hits, None)
            .into_iter()
            .map(|(p, _)| p)
            .collect();

        // When we were only using a name to search,
        // if we have exactly one perfect match, consider it the correct one.
        // Note that it's important to check for other people with the same name.
        if igra_num.is_none() {
            let mut perfection = possible.iter().filter(|p| is_perfect(p));
            match (perfection.next(), perfection.next()) {
                (Some(p), None) => return (true, vec![p]),
                (Some(_), Some(_)) => {
                    // If we have _multiple_ matches,
                    // then limit the results to just them.
                    possible.retain(|p| is_perfect(p));
                }
                _ => {}
            }
        }

        (false, possible)
    }

    fn validate_partners(
        &self,
        proc: &mut Processed<'a>,
        event: &Event,
        db_event: RodeoEvent,
        relevant: &mut HashMap<&'a str, &'a PersonRecord>,
    ) {
        // Remove empty partner strings.
        let partners: Vec<_> = event
            .partners
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();

        match (partners.len() as u64).cmp(&(db_event.num_partners() as u64)) {
            Ordering::Less => {
                proc.issues.push(Suggestion {
                    problem: Problem::TooFewPartners {
                        event: db_event,
                        round: event.round,
                    },
                    fix: Fix::ContactRegistrant,
                });
            }
            Ordering::Greater => {
                proc.issues.push(Suggestion {
                    problem: Problem::TooManyPartners {
                        event: event.id,
                        round: event.round,
                    },
                    fix: Fix::ContactDevelopers,
                });
            }
            Ordering::Equal => {}
        }


        for (i, p) in partners.iter().enumerate() {
            let (part_num, part_name) = split_partner(p);
            log::info!("Partner: {p:?} - Num: {:?} Name: {:?}", part_num, part_name);

            let (perfect, possible) = self.find_person(part_num, "", "", part_name);
            if perfect {
                proc.confirm(possible[0], db_event, event.round, i);
                continue;
            }

            if possible.is_empty() {
                proc.issues.push(Suggestion {
                    problem: Problem::UnknownPartner {
                        event: db_event, round: event.round, index: i,
                    },
                    fix: Fix::ContactRegistrant,
                })
            }

            proc.push_all(
                Problem::UnknownPartner {
                    event: db_event, round: event.round, index: i,
                },
                possible.into_iter().take(30),
                relevant
            );
        }
    }

    fn find_registrant(
        &self,
        proc: &mut Processed<'a>,
        relevant: &mut HashMap<&'a str, &'a PersonRecord>,
    ) {
        let who = &proc.registration.contestant;
        let first_name = who.first_name.trim();
        let last_name = who.last_name.trim();
        let is_member = who.is_member == "yes";
        let igra_num = who.association.igra.trim();
        let dob = who.dob.dos();
        let ssn = who.dos_ssn();

        if is_member && igra_num.is_empty() {
            proc.issues.push(Suggestion {
                problem: Problem::NoValue { field: RegF::IGRANumber, },
                fix: Fix::ContactRegistrant,
            })
        }

        if first_name.is_empty() {
            proc.issues.push(Suggestion {
                problem: Problem::NoValue { field: RegF::LegalFirst, },
                fix: Fix::ContactRegistrant,
            })
        }

        if last_name.is_empty() {
            proc.issues.push(Suggestion {
                problem: Problem::NoValue { field: RegF::LegalLast, },
                fix: Fix::ContactRegistrant,
            })
        }

        // Search for members that closely match the registration.
        let (_, mut candidates) = self.find_person(
            if igra_num.is_empty() { None } else { Some(igra_num) },
            &who.first_name, &who.last_name, &who.performance_name,
        );

        log::debug!("Found {} candidates for '{} {}' aka '{}' with num '{:?}'",
            candidates.len(),
            who.first_name, who.last_name, who.performance_name,
            igra_num,
        );

        // Now that we've got a (possibly empty) list of potential matches,
        // we apply validation rules based on if they say they're a member/gave an IGRA number:
        //
        // - If so, and we their other details match, we choose that record and validate their info.
        //   If we didn't find a match, we can at least list close matches for someone to evaluate.
        //   If we didn't even find a close match, then someone needs to contact them
        //   or add add the missing information to the database.
        //
        // - If they say they aren't yet a member, we check for matching personal info anyway.
        //   If we find any, then perhaps this person was a member, but somehow forgot.
        //   In that case, we can list out those suggestions for someone to review.
        //   Otherwise, we report that their personal details need to be added to the database.
        //
        // The personal details we match against are first and last name, birthday, DoB, and SSN.
        // Technically, two people might have the same values
        // since SSN is only the last 4 (and we don't really know if they used their own),
        // but this only matters if they don't list an IGRA identifier.
        // If we have their IGRA number, this just gives us certainty it's not a typo.
        let exact = |member: &PersonRecord| {
            member.legal_first.eq_ignore_ascii_case(&first_name)
                && member.legal_last.eq_ignore_ascii_case(&last_name)
                && member.birthdate == dob
                && member.ssn == ssn
        };

        // TODO: clean this up, as it's hard to follow the returns.
        let m;
        if !is_member {
            candidates.retain(|p| exact(p));
            if candidates.is_empty() {
                // They say they're not a member, and they're probably right.
                proc.issues.push(Suggestion {
                    problem: Problem::NotAMember, fix: Fix::AddNewMember
                });
                return;
            } else {
                // They say they're not a member, but we found really close matches.
                if candidates.len() == 1 {
                    // Since there's only a single match,
                    // mark them found to highlight field differences.
                    m = candidates[0];
                    proc.push_person(Problem::MaybeAMember, m, relevant);
                } else {
                    proc.push_all(Problem::MaybeAMember, candidates, relevant);
                    return;
                }
            }
        } else if candidates.is_empty() {
            // They say they're a member, but there aren't even close matches.
            proc.issues.push(Suggestion {
                problem: Problem::NoPerfectMatch,
                fix: Fix::ContactRegistrant,
            });
            return;
        } else {
            let mut filtered = candidates.iter()
                .filter(|member| exact(member) && member.igra_number == igra_num);
            let perfect = filtered.next();
            let maybe = filtered.next();

            if maybe.is_some() {
                // We don't have a single, exact match, so add close matches.
                // TODO: Treat the "found" field to mean "very highly likely",
                //   and go ahead and fill it in with a non-perfect match
                //   when other signals point to the right person.
                proc.push_all(Problem::NoPerfectMatch, candidates.into_iter().take(30), relevant);
                return;
            }

            if let Some(p) = perfect {
                m = p
            } else {
                // Even though we don't have a perfect match,
                // we only have a single probable match.
                assert!(candidates.len() >= 1, "candidates should not be empty");
                m = candidates[0];

                proc.issues.push(Suggestion {
                    problem: Problem::NoPerfectMatch,
                    fix: Fix::UseThisRecord(IGRANumber(m.igra_number.clone()))
                });
            }
        }

        proc.found = Some(m.igra_number.as_str());
        relevant.insert(&m.igra_number, m);

        // This macro checks if two strings are equal ignoring ascii case,
        // and if not, adds an issue noting the database field should be updated
        // (or that the registrant made a typo when they filled out the form).
        macro_rules! check (
                ($field:expr, $lval:expr, $rval:expr) => (
                    if !$lval.trim().eq_ignore_ascii_case(&$rval.trim()) {
                        proc.issues.push(Suggestion{
                            problem: Problem::DbMismatch{field: $field},
                            fix: Fix::UpdateDatabase,
                        })
                    }
                );
            );

        // Compare phone numbers by stripping all non-digit characters.
        macro_rules! check_phone (
            ($field:expr, $lval:expr, $rval:expr) => (
                let mut lphone = $lval.clone();
                lphone.retain(|c| c.is_ascii_digit());
                let mut rphone = $rval.clone();
                rphone.retain(|c| c.is_ascii_digit());
                check!($field, lphone, rphone);
            );
        );

        check!(RegF::Email, m.email, who.address.email);
        check!(RegF::Association, m.association, who.association.member_assn);
        check!(RegF::DateOfBirth, m.birthdate, who.dob.dos());

        if let Some((_, ssn)) = m.ssn.rsplit_once('-') {
            check!(RegF::SSN, ssn, who.ssn)
        }

        // In the database, most people performance names match their legal names.
        // If the user left it blank, we probably should should ignore it.
        // Otherwise, we compare the given value against the concatenated "First Last" DB values.
        if !who.performance_name.trim().is_empty() {
            let db_perf_name = format!("{} {}", m.first_name, m.last_name);
            check!(RegF::PerformanceName, db_perf_name, who.performance_name);
        }

        // Address in the database use only a single line.
        let addr = format!("{} {}", who.address.address_line_1, who.address.address_line_2);
        check!(RegF::AddressLine, m.address, addr);
        check!(RegF::City, m.city, who.address.city);
        check!(RegF::PostalCode, m.zip, who.address.zip_code);

        check_phone!(RegF::CellPhone, m.cell_phone, who.address.cell_phone_no);
        check_phone!(RegF::HomePhone, m.home_phone, who.address.home_phone_no);

        // The DB uses two letter abbreviations for states,
        // and it uses the field for Canadian provinces,
        // and calls everything else "FC" for "Foreign Country".
        let is_us_or_can =
            who.address.country == "United States" || who.address.country == "Canada";
        if m.state == "FC" {
            if is_us_or_can {
                proc.issues.push(Suggestion {
                    problem: Problem::DbMismatch {
                        field: RegF::Country,
                    },
                    fix: Fix::UpdateDatabase,
                });
            }
        } else {
            if !is_us_or_can {
                proc.issues.push(Suggestion {
                    problem: Problem::DbMismatch {
                        field: RegF::Country,
                    },
                    fix: Fix::UpdateDatabase,
                });
            }
            match m.region() {
                Some(db_region) => {
                    check!(RegF::Region, db_region, who.address.region)
                }
                _ => proc.issues.push(Suggestion {
                    problem: Problem::DbMismatch {
                        field: RegF::Region,
                    },
                    fix: Fix::UpdateDatabase,
                }),
            }
        }

        // The DB stores "sex", the form reports "gender",
        // but what we actually care about who you're competing with.
        match (m.sex.as_str(), who.gender.as_str()) {
            ("M", "Cowboys") | ("F", "Cowgirls") => {}
            _ => proc.issues.push(Suggestion {
                problem: Problem::DbMismatch {
                    field: RegF::CompetitionCategory,
                },
                fix: Fix::UpdateDatabase,
            }),
        }
    }

}

/// This is the result of processing registration data.
#[derive(Debug, Serialize)]
pub struct Processed<'a> {
    /// This is the original registration information they provided.
    pub registration: &'a Registration,
    /// When we have very high confidence that we found the correct person,
    /// this will hold the associated IGRA number.
    pub found: Option<&'a str>,
    /// These are issues we found with their registration.
    pub issues: Vec<Suggestion>,
    /// For partners they list that we can match to a record,
    /// this holds the associated IGRA information.
    pub partners: Vec<Partner<'a>>,

    /// For partners they list that we can associate with a person,
    /// this maps them to the events/rounds they say they're partnered.
    /// After processing all registrations, we use these to cross-validate entries.
    #[serde(skip)]
    confirmed_partners: HashMap<&'a PersonRecord, Vec<(RodeoEvent, RoundID, usize)>>,
}

impl<'a> Processed<'a> {
    fn new(registration: &'a Registration) -> Self {
        Processed {
            registration,
            found: None,
            issues: vec![],
            partners: vec![],
            confirmed_partners: HashMap::default(),
        }
    }

    /// Add a confirmed partner.
    ///
    /// This updates the confirmed_partners map,
    /// which is used only while working on validating a record.
    fn confirm(&mut self, partner: &'a PersonRecord, event: RodeoEvent, round: u64, index: usize) {
        match self.confirmed_partners.entry(partner) {
            hash_map::Entry::Occupied(mut e) => {
                e.get_mut().push((event, round, index));
            }
            hash_map::Entry::Vacant(e) => {
                e.insert(vec![(event, round, index)]);
            }
        }
    }

    /// For each person in people,
    /// insert an issue of the given problem
    /// with the suggested fix to use that person's record.
    /// In addition, insure those people are the relevancy collection.
    #[inline]
    fn push_all<I>(&mut self,
                problem: Problem,
                people: I,
                relevant: &mut HashMap<&'a str, &'a PersonRecord>,
    )
    where I: IntoIterator<Item=&'a PersonRecord>
    {
        for p in people.into_iter() {
            self.push_person(problem.clone(), p, relevant);
        }
    }

    #[inline]
    fn push_person(&mut self,
                   problem: Problem,
                   person: &'a PersonRecord,
                   relevant: &mut HashMap<&'a str, &'a PersonRecord>,
    )
    {
        self.issues.push(Suggestion {
            problem,
            fix: Fix::UseThisRecord(IGRANumber(person.igra_number.clone())),
        });
        relevant.insert(&person.igra_number, person);
    }
}

/// Validates an entry against all other entries and returns a possibly-emtpy list of problems.
///
/// The validation rules only apply to entries which have a "found" record,
/// as to avoid extra work and more false positives when we're not sure about the actual person.
/// As a consequence, it may have more false negatives, but generally these are less likely anyway.
///
/// The validations this does:
///
/// - This person is only registered once.
/// - For each partner event:
///   - If Person A says Person B is their partner, Person B should be registered.
///   - Person B should list Person A as their partner for the same event.
fn validate_cross_reg(
    entries: &Vec<Processed>,
    person_a: &PersonRecord,
    entry_a: &Processed,
) -> Vec<Suggestion> {
    let mut issues = Vec::<Suggestion>::new();

    // TODO: Check if a person appears to list themself as partner.
    // TODO: Check if a person is registered more than once.

    for (person_b, a_events_with_b) in &entry_a.confirmed_partners {
        // Try to find the entry for B, the partner of A.
        let entry_b = entries.iter().find(|other| {
            other.found.map_or(false, |other_igra_num| {
                other_igra_num == person_b.igra_number
            })
        });

        // For every event and round A claims to partner with B,
        // make sure B claims to partner with A.
        let b_to_a = entry_b
            .map(|b| b.confirmed_partners.get(person_a))
            .flatten();
        for (event_a, round_a, index_a) in a_events_with_b {
            if entry_b.is_none() {
                log::debug!("{} says they're partnering with {}, but {} isn't registered",
                    person_a, person_b, person_b
                );
                issues.push(Suggestion {
                    problem: Problem::UnregisteredPartner {
                        event: *event_a,
                        round: *round_a,
                        index: *index_a,
                    },
                    fix: Fix::AddRegistration(IGRANumber(person_b.igra_number.clone())),
                });
                continue;
            }

            let b_listed_a = b_to_a.map_or(false, |listings| {
                listings
                    .iter()
                    .any(|(b_event, b_round, _)| b_event == event_a && b_round == round_a)
            });

            // A listed B, but B didn't list A.
            if !b_listed_a {
                issues.push(Suggestion {
                    problem: Problem::MismatchedPartners {
                        event: *event_a,
                        round: *round_a,
                        index: *index_a,
                        partner: IGRANumber(person_b.igra_number.clone()),
                    },
                    fix: Fix::ContactRegistrant,
                });
            }
        }
    }

    issues
}

/// Registration fields.
#[allow(unused)]
#[derive(Eq, Hash, PartialEq, Debug, Copy, Clone, Serialize, Deserialize)]
pub enum RegF {
    IsMember,
    IGRANumber,
    Association,

    LegalFirst,
    LegalLast,
    DateOfBirth,
    SSN,

    PerformanceName,
    CompetitionCategory,
    Email,
    AddressLine,
    City,
    Region,
    Country,
    PostalCode,

    CellPhone,
    HomePhone,

    EventID,
    NoteToDirector,
}

pub type RoundID = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "name", content = "data")]
pub enum Problem {
    /// The registrant didn't fill in the field.
    NoValue { field: RegF },
    /// The DoB indicates the registrant is too young to participate.
    NotOldEnough,
    /// The registrant lists that they are not a member,
    /// and there's not a database record that likely matches them.
    NotAMember,
    /// The registrant lists that they are not a member,
    /// but we found a database record that very closely matches their information.
    MaybeAMember,
    /// We couldn't find a database record that matches the registration information.
    NoPerfectMatch,
    /// There's a database record considered a match based on static fields,
    /// but non-static fields (e.g., address or phone number) are different.
    DbMismatch { field: RegF },

    /// The registrant didn't register for enough rounds across all events.
    NotEnoughRounds,
    /// They didn't list enough partners.
    TooFewPartners { event: RodeoEvent, round: RoundID },
    /// We can't associate the entered partner data with a database record.
    UnknownPartner { event: RodeoEvent, round: RoundID, index: usize },
    /// We have a matching database record for the partner,
    /// but they haven't registered yet.
    UnregisteredPartner { event: RodeoEvent, round: RoundID, index: usize },
    /// We have a matching database record for the partner,
    /// that person has registered, but they listed someone else or no one at all.
    MismatchedPartners {
        event: RodeoEvent,
        round: RoundID,
        partner: IGRANumber,
        index: usize,
    },

    // The issues below indicate problems with the data itself,
    // due to manual manipulation of the data or programming bugs.
    /// We don't know how to map this Event ID to the actual event.
    UnknownEventID { event: EventID },
    /// This RoundID is too large.
    InvalidRoundID { event: EventID, round: RoundID },
    /// Somehow the registration has more partners listed than we think the event allows.
    ///
    /// This probably indicates an error with the mapping from registration events to DB events,
    /// but if not, then the registration data itself has more partners than it should,
    /// so either the entry form is coded incorrectly, or someone manually edited the data.
    TooManyPartners { event: EventID, round: RoundID },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "name", content = "data")]
pub enum Fix {
    /// The database should be updated to match changed personal details.
    UpdateDatabase,
    /// The database value is correct, and the registration value is wrong.
    UseThisRecord(IGRANumber),
    /// This person is new to IGRA.
    AddNewMember,
    /// This person is listed as a partner, but has not yet registered.
    AddRegistration(IGRANumber),
    /// The registrant needs to clarify the correct value.
    ContactRegistrant,
    /// The problem is associated with the actual registration data
    /// or how this program interprets it.
    ContactDevelopers,
}

#[derive(Debug, Serialize)]
pub struct Suggestion {
    pub problem: Problem,
    pub fix: Fix,
}

#[derive(Debug, Serialize)]
pub struct Partner<'a> {
    pub event: RodeoEvent,
    pub round: RoundID,
    pub index: usize,
    pub igra_number: &'a str,
}

/// Personal data from the current (old, DOS-based) registration database.
#[allow(unused)]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PersonRecord {
    pub igra_number: String,
    pub association: String,
    pub birthdate: String,
    pub ssn: String,
    pub division: String,
    pub last_name: String,
    pub first_name: String,
    pub legal_last: String,
    pub legal_first: String,
    pub id_checked: String,
    pub sex: String,

    pub address: String,
    pub city: String,
    pub state: String,
    pub zip: String,
    pub home_phone: String,
    pub cell_phone: String,
    pub email: String,
    pub status: String,

    pub first_rodeo: String,
    pub last_updated: String,
    pub sort_date: String,
    #[serde(skip)]
    pub ext_dollars: Decimal,
}

/// Hash implementation for PersonRecord: two records are equivalent if they have the same IGRA number.
/// Note that this logic only holds if PersonRecords are indeed unique by this identifier.
impl std::hash::Hash for PersonRecord {
    fn hash<H>(&self, state: &mut H)
        where
            H: std::hash::Hasher,
    {
        self.igra_number.hash(state)
    }
}

impl PartialEq<PersonRecord> for PersonRecord {
    fn eq(&self, other: &PersonRecord) -> bool {
        self.igra_number.eq(&other.igra_number)
    }
}

impl Eq for PersonRecord {}

/// An event registration record from the current (old, DOS-based) registration database.
#[allow(unused)]
#[derive(Debug, Default)]
pub struct RegistrationRecord {
    igra_number: String,
    first_name: String,
    last_name: String,
    sex: String,
    city: String,
    state: String,
    association: String,
    ssn: String,

    events: Vec<EventRecord>,

    sat_points: Decimal,
    sun_points: Decimal,
    ext_points: Decimal,
    tot_points: Decimal,
}

/// An event result record from the current (old, DOS-based) registration database.
#[derive(Debug, Default)]
pub struct EventRecord {
    name: String,
    outcome: Option<EventMetric>,
    dollars: Decimal,
    points: Decimal,
    world: Decimal,
}

/// An event is scored using either Time or Score.
#[allow(dead_code)]
#[derive(Debug)]
pub enum EventMetric {
    Time(Decimal),
    Score(Decimal),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IGRANumber(String);

#[derive(Debug, Clone)]
pub struct LegalLast(String);

impl Display for IGRANumber {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Display for LegalLast {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Read the personnel database (likely named "PERSONEL.DBF").
pub fn read_personnel<R: io::Read>(
    table: TableReader<Header<R>>,
) -> DBaseResult<Vec<PersonRecord>> {
    let mut people = Vec::<PersonRecord>::with_capacity(table.n_records());
    let mut records = table.records();

    while let Some(record) = records.next() {
        let record = record?;

        let mut person = PersonRecord::default();
        for field in record {
            let field = field?;
            match (field.name, field.value) {
                ("IGRA_NUM", Field::Character(s)) => person.igra_number = s,
                ("STATE_ASSN", Field::Character(s)) => person.association = s,
                ("BIRTH_DATE", Field::Character(s)) => person.birthdate = s,
                ("SSN", Field::Character(s)) => person.ssn = s,
                ("DIVISION", Field::Character(s)) => person.division = s,
                ("LAST_NAME", Field::Character(s)) => person.last_name = s,
                ("FIRST_NAME", Field::Character(s)) => person.first_name = s,
                ("LEGAL_LAST", Field::Character(s)) => person.legal_last = s,
                ("LEGALFIRST", Field::Character(s)) => person.legal_first = s,
                ("ID_CHECKED", Field::Character(s)) => person.id_checked = s,
                ("SEX", Field::Character(s)) => person.sex = s,
                ("ADDRESS", Field::Character(s)) => person.address = s,
                ("CITY", Field::Character(s)) => person.city = s,
                ("STATE", Field::Character(s)) => person.state = s,
                ("ZIP", Field::Character(s)) => person.zip = s,
                ("HOME_PHONE", Field::Character(s)) => person.home_phone = s,
                ("CELL_PHONE", Field::Character(s)) => person.cell_phone = s,
                ("E_MAIL", Field::Character(s)) => person.email = s,
                ("STATUS", Field::Character(s)) => person.status = s,
                ("FIRSTRODEO", Field::Character(s)) => person.first_rodeo = s,
                ("LASTUPDATE", Field::Character(s)) => person.last_updated = s,
                ("SORT_DATE", Field::Character(s)) => person.sort_date = s,
                ("EXT_DOLLAR", Field::Numeric(Some(n))) => person.ext_dollars = n,
                ("EXT_DOLLAR", Field::Numeric(None)) => {}
                (n, v) => {
                    panic!("Unknown field: {n} with value '{v:?}'");
                }
            }
        }

        // TODO: add "full name" fields to the record & create them manually.
        people.push(person);
    }

    people.sort_by(|a, b| a.igra_number.cmp(&b.igra_number));
    Ok(people)
}

impl DBaseRecord for PersonRecord {
    fn describe(&self) -> Vec<FieldDescriptor> {
        vec![
            FieldDescriptor{ name: "IGRA_NUM".to_string(), field_type: FieldType::Character, 
                length: 4, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "STATE_ASSN".to_string(),field_type: FieldType::Character, 
                length: 6, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "BIRTH_DATE".to_string(),field_type: FieldType::Character, 
                length: 8, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "SSN".to_string(), field_type: FieldType::Character, 
                length: 11, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "DIVISION".to_string(),field_type: FieldType::Character, 
                length: 1, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "LAST_NAME".to_string(), field_type: FieldType::Character, 
                length: 20, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "FIRST_NAME".to_string(),field_type: FieldType::Character, 
                length: 17, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "LEGAL_LAST".to_string(),field_type: FieldType::Character, 
                length: 20, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "LEGALFIRST".to_string(),field_type: FieldType::Character, 
                length: 17, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "ID_CHECKED".to_string(),field_type: FieldType::Character, 
                length: 1, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "SEX".to_string(), field_type: FieldType::Character, 
                length: 1, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "ADDRESS".to_string(), field_type: FieldType::Character, 
                length: 30, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "CITY".to_string(), field_type: FieldType::Character, 
                length: 30, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "STATE".to_string(), field_type: FieldType::Character, 
                length: 2, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "ZIP".to_string(), field_type: FieldType::Character, 
                length: 10, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "HOME_PHONE".to_string(),field_type: FieldType::Character, 
                length: 10, decimal_count: 13, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "CELL_PHONE".to_string(),field_type: FieldType::Character, 
                length: 10, decimal_count: 13, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "E_MAIL".to_string(), field_type: FieldType::Character, 
                length: 30, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "STATUS".to_string(), field_type: FieldType::Character, 
                length: 1, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "FIRSTRODEO".to_string(),field_type: FieldType::Character, 
                length: 8, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "LASTUPDATE".to_string(),field_type: FieldType::Character, 
                length: 8, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "SORT_DATE".to_string(), field_type: FieldType::Character, 
                length: 8, decimal_count: 10, work_area_id: 0, example: 8 },
            FieldDescriptor{ name: "EXT_DOLLAR".to_string(),field_type: FieldType::Numeric, 
                length: 10, decimal_count: 10, work_area_id: 0, example: 8 },
        ]
    }

    fn to_record(&self) -> Vec<Field> {
        vec![
            Field::Character(self.igra_number.clone()),
            Field::Character(self.association.clone()),
            Field::Character(self.birthdate.clone()),
            Field::Character(self.ssn.clone()),
            Field::Character(self.division.clone()),
            Field::Character(self.last_name.clone()),
            Field::Character(self.first_name.clone()),
            Field::Character(self.legal_last.clone()),
            Field::Character(self.legal_first.clone()),
            Field::Character(self.id_checked.clone()),
            Field::Character(self.sex.clone()),
            Field::Character(self.address.clone()),
            Field::Character(self.city.clone()),
            Field::Character(self.state.clone()),
            Field::Character(self.zip.clone()),
            Field::Character(self.home_phone.clone()),
            Field::Character(self.cell_phone.clone()),
            Field::Character(self.email.clone()),
            Field::Character(self.status.clone()),
            Field::Character(self.first_rodeo.clone()),
            Field::Character(self.last_updated.clone()),
            Field::Character(self.sort_date.clone()),
            Field::Numeric(Some(self.ext_dollars.clone())),
        ]
    }
}

/// Read event records from a DBF table.
fn read_rodeo_events<R: io::Read>(
    table: TableReader<Header<R>>,
) -> DBaseResult<Vec<RegistrationRecord>> {
    let mut registrations = Vec::<RegistrationRecord>::with_capacity(table.n_records());
    let mut records = table.records();

    while let Some(record) = records.next() {
        let record = record?;

        let mut entrant = RegistrationRecord::default();
        for field in record {
            let f = field?;

            if f.name.ends_with("_SAT") || f.name.ends_with("_SUN") {
                let is_x = if let Field::Character(ref x) = f.value {
                    x == "X"
                } else {
                    false
                };

                if &f.name[5..6] == "E" && is_x {
                    let mut evnt = EventRecord::default();
                    evnt.name = f.name.into();
                    entrant.events.push(evnt);
                } else if let Some(evnt) = entrant
                    .events
                    .iter_mut()
                    .find(|e| e.name[..4] == f.name[..4] && e.name[6..] == f.name[6..])
                {
                    match (&f.name[5..6], f.value) {
                        ("S", Field::Numeric(Some(n))) => {
                            evnt.outcome = Some(EventMetric::Score(n))
                        }
                        ("T", Field::Numeric(Some(n))) => evnt.outcome = Some(EventMetric::Time(n)),
                        ("P", Field::Numeric(Some(n))) => evnt.points = n,
                        ("D", Field::Numeric(Some(n))) => evnt.dollars = n,
                        ("W", Field::Numeric(Some(n))) => evnt.world = n,
                        _ => {}
                    }
                }

                continue;
            }

            match (f.name, f.value) {
                ("IGRA_NUM", Field::Character(s)) => entrant.igra_number = s,
                ("FIRST_NAME", Field::Character(s)) => entrant.first_name = s,
                ("LAST_NAME", Field::Character(s)) => entrant.last_name = s,
                ("SEX", Field::Character(s)) => entrant.sex = s,
                ("CITY", Field::Character(s)) => entrant.city = s,
                ("STATE", Field::Character(s)) => entrant.state = s,
                ("STATE_ASSN", Field::Character(s)) => entrant.association = s,
                ("SSN", Field::Character(s)) => entrant.ssn = s,
                ("SAT_POINTS", Field::Numeric(Some(n))) => entrant.sat_points = n,
                ("SUN_POINTS", Field::Numeric(Some(n))) => entrant.sun_points = n,
                ("EXT_POINTS", Field::Numeric(Some(n))) => entrant.ext_points = n,
                ("TOT_POINTS", Field::Numeric(Some(n))) => entrant.tot_points = n,
                _ => {}
            }
        }

        registrations.push(entrant);
    }

    Ok(registrations)
}

impl Display for EventRecord {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.outcome {
            None => {
                write!(f, "{:10}: No Score/No Time", self.name)
            }
            Some(EventMetric::Score(s)) => {
                write!(
                    f,
                    "{:10}: score={s:5}  dollars=${:5}  points={:5}  world={:5}",
                    self.name, self.dollars, self.points, self.world,
                )
            }
            Some(EventMetric::Time(t)) => {
                write!(
                    f,
                    "{:10}:  time={t:5}  dollars=${:5}  points={:5}  world={:5}",
                    self.name, self.dollars, self.points, self.world,
                )
            }
        }
    }
}

impl Display for PersonRecord {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{num:4} {gender:1} {bday:8} {legal:26} {perf:26} {assoc:5} {addr:40} ",
            num=self.igra_number,
            gender=self.sex,
            bday=self.birthdate,
            legal=format!("{} {}", self.legal_first, self.legal_last),
            perf=format!("{} {}", self.first_name, self.last_name),
            addr=format!("{} {}, {}", self.address, self.city, self.state),
            assoc=self.association,
        )
    }
}

impl Display for RegistrationRecord {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:4} {:6} {:10} {:17} {cat:7} {:18} {:2}  sat={:5}  sun={:5}  tot={:5}  ext={:5}",
            self.igra_number,
            self.association,
            self.first_name,
            self.last_name,
            self.city,
            self.state,
            self.sat_points,
            self.sun_points,
            self.tot_points,
            self.ext_points,
            cat = if self.sex == "M" { "COWBOY" } else { "COWGIRL" }
        )
    }
}

/// REGIONS maps the old database identifiers
/// to the region string used by the new registration system.
///
/// Some of these won't ever be returned by the new system,
/// but they're included here for completeness.
static REGIONS: phf::Map<&'static str, &'static str> = phf_map! {
    "AK" => "Alaska",
    "AL" => "Alabama",
    "AR" => "Arkansas",
    "AZ" => "Arizona",
    "CA" => "California",
    "CO" => "Colorado",
    "CT" => "Connecticut",
    "DE" => "Delaware",
    "FL" => "Florida",
    "GA" => "Georgia",
    "HI" => "Hawaii",
    "IA" => "Iowa",
    "ID" => "Idaho",
    "IL" => "Illinois",
    "IN" => "Indiana",
    "KS" => "Kansas",
    "KY" => "Kentucky",
    "LA" => "Louisiana",
    "MA" => "Massachusetts",
    "MD" => "Maryland",
    "ME" => "Maine",
    "MI" => "Michigan",
    "MN" => "Minnesota",
    "MO" => "Missouri",
    "MS" => "Mississippi",
    "MT" => "Montana",
    "NC" => "North Carolina",
    "ND" => "North Dakota",
    "NE" => "Nebraska",
    "NH" => "New Hampshire",
    "NJ" => "New Jersey",
    "NM" => "New Mexico",
    "NV" => "Nevada",
    "NY" => "New York",
    "OH" => "Ohio",
    "OK" => "Oklahoma",
    "ON" => "Ontario",
    "OR" => "Oregon",
    "PA" => "Pennsylvania",
    "RI" => "Rhode Island",
    "SC" => "South Carolina",
    "SD" => "South Dakota",
    "TN" => "Tennessee",
    "TX" => "Texas",
    "UT" => "Utah",
    "VA" => "Virginia",
    "VT" => "Vermont",
    "WA" => "Washington",
    "WI" => "Wisconsin",
    "WV" => "West Virginia",
    "WY" => "Wyoming",

    "DC" => "District Of Columbia",
    "GU" => "Guam",
    "PR" => "Puerto Rico",
    "VI" => "Virgin Islands",

    "AB" => "Alberta",
    "BC" => "British Columbia",
    "LB" => "Newfoundland and Labrador",
    "MB" => "Manitoba",
    "NB" => "New Brunswick",
    "NF" => "Newfoundland and Labrador",
    "NS" => "Nova Scotia",
    "NT" => "Northwest Territories",
    "PE" => "Prince Edward Island",
    "PQ" => "Quebec",
    "SK" => "Saskatchewan",
    "YT" => "Yukon Territory",

    "AE" => "Army Europe",
    "CS" => "Alabama", // not sure what's up with this one
    "CZ" => "Canal Zone",
    "FC" => "Foreign Country",
};

impl PersonRecord {
    pub fn region(&self) -> Option<&&'static str> {
        REGIONS.get(&self.state)
    }
}

pub static CANADIAN_REGIONS: phf::Set<&'static str> = phf_set! {
    "AB", "BC", "LB", "MB", "NB", "NF", "NS", "NT", "PE", "PQ", "SK", "YT",
};

#[allow(unused)]
#[derive(Deserialize, Serialize, Debug, Copy, Clone, Eq, Hash, PartialEq)]
pub enum RodeoEvent {
    CalfRopingOnFoot,
    MountedBreakaway,
    TeamRopingHeader,
    TeamRopingHeeler,
    PoleBending,
    BarrelRacing,
    FlagRacing,

    ChuteDogging,
    RanchSaddleBroncRiding,
    SteerRiding,
    BullRiding,

    GoatDressing,
    SteerDecorating,
    WildDragRace,
}

impl RodeoEvent {
    pub fn num_partners(self) -> u8 {
        match self {
            RodeoEvent::CalfRopingOnFoot => 0,
            RodeoEvent::MountedBreakaway => 0,
            RodeoEvent::TeamRopingHeader => 1,
            RodeoEvent::TeamRopingHeeler => 1,
            RodeoEvent::PoleBending => 0,
            RodeoEvent::BarrelRacing => 0,
            RodeoEvent::FlagRacing => 0,
            RodeoEvent::ChuteDogging => 0,
            RodeoEvent::RanchSaddleBroncRiding => 0,
            RodeoEvent::SteerRiding => 0,
            RodeoEvent::BullRiding => 0,
            RodeoEvent::GoatDressing => 1,
            RodeoEvent::SteerDecorating => 1,
            RodeoEvent::WildDragRace => 2,
        }
    }

    pub fn from_id(id: u64) -> Option<Self> {
        // todo: put this info somewhere else
        let event = match id {
            3 => RodeoEvent::BullRiding,
            // 11 => RodeoEvent::TeamRopingHeader,
            12 => RodeoEvent::TeamRopingHeeler,
            13 => RodeoEvent::BarrelRacing,
            14 => RodeoEvent::PoleBending,
            15 => RodeoEvent::FlagRacing,
            16 => RodeoEvent::SteerDecorating,
            17 => RodeoEvent::WildDragRace,
            18 => RodeoEvent::GoatDressing,
            // todo: determine missing IDs
            _ => {
                return None;
            }
        };

        Some(event)
    }
}
