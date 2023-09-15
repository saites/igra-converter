use std::clone::Clone;
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
use std::iter::{Iterator, zip};
use std::ops::Deref;
use chrono::NaiveDate;
use memchr::memchr;

use crate::bktree;
use crate::bktree::BKTree;
use crate::robin::EventID::Known;
use crate::robin::{Event, EventID, Registration};
use crate::xbase::{DBaseRecord, DBaseResult, Decimal, Field, Header, TableReader, FieldDescriptor, FieldType};

/// Read registration data from the JSON file at the given path.
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

/// Checks if two strings are equal ignoring ascii case and leading/trailing whitespace.
fn str_eq(s1: &str, s2: &str) -> bool {
    s1.trim().eq_ignore_ascii_case(s2.trim())
}

/// The registration form asks for "PARTNER NAME | IGRA #",
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
    /// Which inputs are non-empty determines how we decide the input matches a record.
    ///
    /// If we have an IGRA number, a perfect match must match that matching number,
    /// and if all the name fields are empty, then we'll simply return that record.
    ///
    /// Without an IGRA number, a perfect match depends on what names are non-empty,
    /// what they match in the database, and whether we have multiple potential hits.
    ///
    /// If `performance` is non-empty, we try to extract a first and last name portion as follows:
    /// - If it has a comma `,` we split it there assume we have something like "LastName, FirstName".
    /// - Otherwise, if it has a space ` ` we split it there and assume its like "FirstName LastName".
    /// - Otherwise, we just call it "FirstName".
    /// Finally, we trim any leading whitespace or additional commas from each part,
    /// and if this leaves the first part empty, their values are swapped.
    /// Note that even if `performance` is be non-empty, these resulting parts might be (e.g. ",, ,").
    ///
    /// When `first` and `last` are both empty, they are ignored for matching purposes.
    /// If either is non-empty, `first` must match `legal_first` and `last` must match `legal_last`.
    ///
    /// When both are set, they each must match their respective fields.
    /// When only first and last are set, they must match the legal first/last names.
    /// If only performance is set, it must match _either_ legal or performance names.
    /// If we're only given two-part performance name P (e.g. likely a partner field),
    /// and we're matching against a record R that has an empty last_name or first_name,
    /// we'll accept `P == "R.first_name R.legal_last"` or `P == R.legal_first R.last_name`.
    pub fn find_person<'b>(&'b self, igra_num: Option<&str>, first: &str, last: &str, performance: &str)
                           -> (bool, Vec<&'a PersonRecord>) {
        let ignore_chars: &[_] = &[' ', ','];

        let first = first.trim_matches(ignore_chars);
        let last = last.trim_matches(ignore_chars);
        let (p_first, p_last) = performance.split_once(',')
            .map(|(l, f)| { (f, l) })
            .or_else(|| performance.split_once(' '))
            .map(|(f, l)| { (f.trim_matches(ignore_chars), l.trim_matches(ignore_chars)) })
            .map(|(f, l)| { if f.is_empty() { (l, f) } else { (f, l) } })
            .unwrap_or((performance.trim_matches(ignore_chars), ""));

        let have_legal_input = !(first.is_empty() && last.is_empty());
        let have_perf_input = !(p_first.is_empty() && p_last.is_empty());
        let two_part_perf = !p_first.is_empty() && !p_last.is_empty();

        // This function intentionally excludes things that are reasonable, but not specific enough.
        // In fact, it's likely a bit too broad, but that's the sort of thing we can edit in post :)
        let is_perfect = |rec: &PersonRecord| {
            let l_lf_match = str_eq(&rec.legal_first, first);
            let l_ll_match = str_eq(&rec.legal_last, last);
            let p_pf_match = str_eq(&rec.first_name, p_first);
            let p_pl_match = str_eq(&rec.last_name, p_last);
            let p_lf_match = str_eq(&rec.legal_first, p_first);
            let p_ll_match = str_eq(&rec.legal_last, p_last);

            // When we don't need to match the performance name, things are easier.
            match (have_legal_input, have_perf_input) {
                (false, false) => { igra_num.is_some_and(|s| str_eq(s, &rec.igra_number)) }
                (true, false) => { l_lf_match && l_ll_match }
                (true, true) => { l_lf_match && l_ll_match && p_pf_match && p_pl_match }
                (false, true) => {
                    (p_pf_match && p_pl_match) || (p_lf_match && p_ll_match) ||
                        (two_part_perf &&
                            (rec.last_name.is_empty() && p_pf_match && p_ll_match) || // "FirstName LegalLast"
                            (rec.first_name.is_empty() && p_lf_match && p_pl_match)   // "LegalFirst LastName"
                        )
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
                        event: db_event,
                        round: event.round,
                        index: i,
                    },
                    fix: Fix::ContactRegistrant,
                })
            }

            proc.push_all(
                Problem::UnknownPartner {
                    event: db_event,
                    round: event.round,
                    index: i,
                },
                possible.into_iter().take(30),
                relevant,
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
                problem: Problem::NoValue { field: RegF::IGRANumber },
                fix: Fix::ContactRegistrant,
            })
        }

        if first_name.is_empty() {
            proc.issues.push(Suggestion {
                problem: Problem::NoValue { field: RegF::LegalFirst },
                fix: Fix::ContactRegistrant,
            })
        }

        if last_name.is_empty() {
            proc.issues.push(Suggestion {
                problem: Problem::NoValue { field: RegF::LegalLast },
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
            str_eq(&member.legal_first, &first_name)
                && str_eq(&member.legal_last, &last_name)
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
                    problem: Problem::NotAMember,
                    fix: Fix::AddNewMember,
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
                    fix: Fix::UseThisRecord(IGRANumber(m.igra_number.clone())),
                });
            }
        }

        proc.found = Some(m.igra_number.as_str());
        relevant.insert(&m.igra_number, m);

        /// Checks if two strings are equal ignoring ascii case,
        /// and if not, adds an issue noting the database field should be updated
        /// (or that the registrant made a typo when they filled out the form).
        fn check(proc: &mut Processed, field: RegF, s1: &str, s2: &str) {
            if !str_eq(s1, s2) {
                proc.issues.push(Suggestion {
                    problem: Problem::DbMismatch { field },
                    fix: Fix::UpdateDatabase,
                })
            }
        }

        /// Compare phone numbers by stripping all non-digit characters.
        fn check_phone(proc: &mut Processed, field: RegF, lphone: &str, rphone: &str) {
            let mut lphone = lphone.to_string();
            let mut rphone = rphone.to_string();
            lphone.retain(|c| c.is_ascii_digit());
            rphone.retain(|c| c.is_ascii_digit());

            // If given, strip a likely country prefix.
            let lphone = if lphone.len() == 11 && lphone.starts_with("1") { &lphone[1..] } else { &lphone };
            let rphone = if rphone.len() == 11 && rphone.starts_with("1") { &rphone[1..] } else { &rphone };
            check(proc, field, lphone, rphone);
        }

        check(proc, RegF::Email, &m.email, &who.address.email);
        check(proc, RegF::DateOfBirth, &m.birthdate, &who.dob.dos());

        if let Some(assn) = who.association.member_assn.split_whitespace().next() {
            log::debug!("Association: {assn}");
            check(proc, RegF::Association, &m.association, &assn);
        } else {
            log::debug!("Association: {}", who.association.member_assn);
            check(proc, RegF::Association, &m.association, &who.association.member_assn);
        }

        if let Some((_, ssn)) = m.ssn.rsplit_once('-') {
            check(proc, RegF::SSN, &ssn, &who.ssn)
        } else {
            check(proc, RegF::SSN, &m.ssn, &who.ssn)
        }

        check(proc, RegF::LegalFirst, &m.legal_first, &who.first_name);
        check(proc, RegF::LegalLast, &m.legal_last, &who.last_name);

        // In the database, most people's performance names match their legal names.
        // If the user left it blank, we probably should should ignore it.
        // Otherwise, we compare the given value against the concatenated "First Last" DB values.
        if !who.performance_name.trim().is_empty() {
            let db_perf_name = format!("{} {}", m.first_name, m.last_name);
            check(proc, RegF::PerformanceName, &db_perf_name, &who.performance_name);
        }

        // Address in the database use only a single line.
        // This needs a bit of work to handle common abbreviations and such.
        let addr = format!("{} {}", who.address.address_line_1, who.address.address_line_2);
        check(proc, RegF::AddressLine, &m.address, &addr);
        check(proc, RegF::City, &m.city, &who.address.city);

        // Postal codes in the database often have a suffix, but users usually don't put them.
        // If only one has a suffix, just compare their prefixes; otherwise compare them as usual.
        match (m.zip.split_once('-'), who.address.zip_code.split_once('-')) {
            (Some((m_prefix, _)), None) => { check(proc, RegF::PostalCode, m_prefix, &who.address.zip_code); }
            (None, Some((r_prefix, _))) => { check(proc, RegF::PostalCode, &m.zip, r_prefix); }
            _ => { check(proc, RegF::PostalCode, &m.zip, &who.address.zip_code); }
        };

        check_phone(proc, RegF::CellPhone, &m.cell_phone, &who.address.cell_phone_no);
        // If they put the same number in twice, just ignore the second.
        if !str_eq(&who.address.cell_phone_no, &who.address.home_phone_no) {
            check_phone(proc, RegF::HomePhone, &m.home_phone, &who.address.home_phone_no);
        }

        // The DB uses two letter abbreviations for states,
        // and it uses the field for Canadian provinces,
        // and calls everything else "FC" for "Foreign Country".
        let is_us_or_can =
            str_eq(&who.address.country, "United States")
                || str_eq(&who.address.country, "US")
                || str_eq(&who.address.country, "USA")
                || str_eq(&who.address.country, "Canada")
                || str_eq(&who.address.country, "CA")
                || str_eq(&who.address.country, "CAN");
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

            // Most of the regions are 'normalized' to a full name,
            // but sometimes we just have a two-letter state abbreviation.
            let region_matches = m.region().map_or(false, |db_region| {
                str_eq(db_region, &who.address.region)
            });
            if !(region_matches || str_eq(&m.state, &who.address.region)) {
                proc.issues.push(Suggestion {
                    problem: Problem::DbMismatch {
                        field: RegF::Region,
                    },
                    fix: Fix::UpdateDatabase,
                });
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

fn at_most(s: &str, n: usize) -> String {
    if s.len() >= n { &s[..n] } else { s }.to_string()
}

impl<'a> Report<'a> {
    /// Turn the processed records into their dBASE equivalent.
    ///
    /// Note that the dBASE records aren't necessarily valid,
    /// either internally, across records, or with some underlying database.
    pub fn online_to_dbase(&self) -> Vec<RegistrationRecord> {
        let today = chrono::Utc::now().naive_utc().date();

        self.results.iter().map(|processed| {
            let reg = &processed.registration;
            let stalls = Decimal::from(reg.stalls.min(9) as i64);

            let (prepaid_amount, prepaid_date) = if reg.payment.total > 0 {
                (
                    Some(Decimal::from_parts(((&reg.payment.total) / 100) as i32, (&reg.payment.total % 100) as u32)),
                    Some(reg.estimate_payment_date().unwrap_or(today)),
                )
            } else {
                (None, None)
            };


            let events = reg.events.iter().filter_map(|e| {
                if let EventID::Known(eid) = e.id {
                    eid.construct_name(e.round).map(|name| {
                        let partners = if processed.partners.is_empty() {
                            None
                        } else {
                            let ids: Vec<_> = processed.partners.iter().filter_map(|p| {
                                if p.event == eid && p.round == e.round {
                                    Some(p.igra_number.to_string())
                                } else {
                                    None
                                }
                            }).collect();

                            if ids.is_empty() { None } else { Some(ids) }
                        };

                        EventRecord {
                            name,
                            partners,
                            ..Default::default()
                        }
                    })
                } else {
                    None
                }
            }).collect();

            if let Some(db) = processed.found.and_then(|num| self.relevant.get(num)) {
                RegistrationRecord {
                    igra_number: db.igra_number.clone(),
                    association: db.association.clone(),
                    ssn: db.ssn.clone(),
                    division: db.division.clone(),
                    last_name: db.last_name.clone(),
                    first_name: db.first_name.clone(),
                    city: db.city.clone(),
                    state: db.state.clone(),
                    sex: db.sex.clone(),
                    // rodeo_association: at_most(rodeo_association, 2),
                    events,
                    stalls,
                    prepaid_amount,
                    prepaid_date,

                    ..Default::default()
                }
            } else {
                let c = &reg.contestant;
                let association = memchr(b' ', c.association.member_assn.as_bytes())
                    .map_or_else(|| at_most(&c.association.member_assn, 5),
                                 |i| c.association.member_assn[0..i.min(5)].to_string());
                let division = IGRA_DIVISIONS.get(&association).unwrap_or(&" ").to_string();

                let (first_name, last_name) = if c.performance_name.is_empty() {
                    (c.first_name.as_str(), c.last_name.as_str())
                } else {
                    memchr(b' ', c.performance_name.as_bytes())
                        .map_or((c.performance_name.as_str(), ""),
                                |i| c.performance_name.split_at(i))
                };

                RegistrationRecord {
                    igra_number: at_most(&c.association.igra, 4),
                    ssn: at_most(&c.ssn, 11),
                    last_name: at_most(last_name, 17),
                    first_name: at_most(first_name, 10),
                    city: at_most(&c.address.city, 18),
                    sex: if c.gender == "Cowboys" { "M" } else { "F" }.to_string(),
                    // rodeo_association: at_most(rodeo_association, 2),
                    state: at_most(STATES.get(&c.address.region).unwrap_or(&"  "), 2),
                    association,
                    division,
                    events,
                    stalls,
                    prepaid_amount,
                    prepaid_date,

                    ..Default::default()
                }
            }
        }).collect()
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
        let b_to_a = entry_b.and_then(|b| b.confirmed_partners.get(person_a));
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
///
/// To better reflect how the table is actually used,
/// this doesn't exactly match the table layout, nor hold all the same fields.
///
/// When reading the table, `RODEO_NUM` is ignored.
/// During writing, it's filled with `igra_number`, matching the 2020 rule change.
///
/// Event information is converted to its own `EventRecord` struct.
/// During writing, events in that collection fill the relevant fields, and the rest are left blank.
/// During reading, events for which a person is registered are collected in the `events` collection.
/// A "T" or "S" column is converted to an `EventMetric::Time` or `EventMetric::Score`, respectively.
/// That value is stored in the `outcome` field, though its initialized as `None`.
#[allow(unused)]
#[derive(Debug, Default)]
pub struct RegistrationRecord {
    igra_number: String,
    association: String,
    ssn: String,
    division: String,
    last_name: String,
    first_name: String,
    city: String,
    state: String,
    sex: String,

    events: Vec<EventRecord>,

    // I think these are either completely unused or used as scratch fields by the clipper app.
    rodeo_score: Option<Decimal>,
    rodeo_time: Option<Decimal>,
    rodeo_association: String,
    flag_1: String,
    flag_2: String,

    stalls: Decimal,
    extra_flag: String, // also seems unused

    sat_points: Decimal,
    sun_points: Decimal,
    ext_points: Decimal,
    tot_points: Decimal,

    prepaid_amount: Option<Decimal>,
    prepaid_date: Option<NaiveDate>,

    sat_dollars: Decimal,
    sun_dollars: Decimal,
    ext_dollars: Decimal,
    tot_dollars: Decimal,
}

impl RegistrationRecord {
    /// Return the event record matching the event name, if we have it.
    fn get_event(&self, name: &str) -> Option<&EventRecord> {
        self.events.iter().find(|e| e.name == name)
    }

    fn add_fields_for(&self, name: &str, entered_first: bool, n_partners: usize, data: &mut Vec<Field>) {
        if let Some(e) = self.get_event(name) {
            e.add_fields(entered_first, n_partners, data);
        } else {
            EventRecord::add_empty_fields(entered_first, n_partners, data);
        }
    }
}


/// An event result record from the current (old, DOS-based) registration database.
#[derive(Debug, Default)]
pub struct EventRecord {
    /// The name of the event, which actually encodes the round information, too.
    /// TODO: parse out the event and round info to make this more ergonomic.
    name: String,
    /// IGRA numbers of registered partners, if known.
    partners: Option<Vec<String>>,
    outcome: Option<EventMetric>,
    dollars: Decimal,
    points: Decimal,
    world: Decimal,
}

impl EventRecord {
    /// Add data fields for this event, indicating it is entered.
    fn add_fields(&self, entered_first: bool, n_partners: usize, data: &mut Vec<Field>) {
        if entered_first {
            data.push(Field::Character("X".to_string())); // entered
        }

        // For partner events, emit the IGRA number of partners,
        // up to the expected number of partners.
        // If the event requires more partners than recorded,
        // emit an empty string for each of those fields.
        let emitted = if let Some(partners) = &self.partners {
            partners.iter().take(n_partners).for_each(|p| {
                data.push(Field::Character(p.clone()))
            });
            partners.len()
        } else {
            0
        };
        (0..(n_partners.saturating_sub(emitted))).for_each(|_| {
            data.push(Field::Character("".to_string()))
        });

        if !entered_first {
            data.push(Field::Character("X".to_string())); // entered
        }

        if let Some(o) = self.outcome {
            data.push(Field::Numeric(Some(
                match o {
                    EventMetric::Time(t) => t,
                    EventMetric::Score(s) => s,
                }
            )));

            data.push(Field::Numeric(Some(self.points)));
            data.push(Field::Numeric(Some(self.dollars)));
            data.push(Field::Numeric(Some(self.world)));
        } else {
            data.push(Field::Numeric(None)); // outcome
            data.push(Field::Numeric(None)); // points
            data.push(Field::Numeric(None)); // dollars
            data.push(Field::Numeric(None)); // world
        }
    }

    /// Add data fields for an event that was not entered.
    fn add_empty_fields(entered_first: bool, n_partners: usize, data: &mut Vec<Field>) {
        if entered_first {
            data.push(Field::Character("".to_string())); // not entered
            (0..n_partners).for_each(|_| { data.push(Field::Character("".to_string())) });
        } else {
            (0..n_partners).for_each(|_| { data.push(Field::Character("".to_string())) });
            data.push(Field::Character("".to_string())); // not entered
        }
        data.push(Field::Numeric(None)); // outcome
        data.push(Field::Numeric(None)); // points
        data.push(Field::Numeric(None)); // dollars
        data.push(Field::Numeric(None)); // world
    }
}

/// An event is scored using either Time or Score.
#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
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
                // Ignore RODEO_NUM, which now must match IGRA_NUM.
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
            FieldDescriptor {
                name: "IGRA_NUM".to_string(),
                field_type: FieldType::Character,
                length: 4,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "STATE_ASSN".to_string(),
                field_type: FieldType::Character,
                length: 5,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "BIRTH_DATE".to_string(),
                field_type: FieldType::Character,
                length: 8,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "SSN".to_string(),
                field_type: FieldType::Character,
                length: 11,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "DIVISION".to_string(),
                field_type: FieldType::Character,
                length: 1,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "LAST_NAME".to_string(),
                field_type: FieldType::Character,
                length: 17,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "FIRST_NAME".to_string(),
                field_type: FieldType::Character,
                length: 10,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "LEGAL_LAST".to_string(),
                field_type: FieldType::Character,
                length: 17,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "LEGALFIRST".to_string(),
                field_type: FieldType::Character,
                length: 10,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "ID_CHECKED".to_string(),
                field_type: FieldType::Character,
                length: 1,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "SEX".to_string(),
                field_type: FieldType::Character,
                length: 1,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "ADDRESS".to_string(),
                field_type: FieldType::Character,
                length: 30,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "CITY".to_string(),
                field_type: FieldType::Character,
                length: 18,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "STATE".to_string(),
                field_type: FieldType::Character,
                length: 2,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "ZIP".to_string(),
                field_type: FieldType::Character,
                length: 10,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "HOME_PHONE".to_string(),
                field_type: FieldType::Character,
                length: 13,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "CELL_PHONE".to_string(),
                field_type: FieldType::Character,
                length: 13,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "E_MAIL".to_string(),
                field_type: FieldType::Character,
                length: 50,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "STATUS".to_string(),
                field_type: FieldType::Character,
                length: 1,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "FIRSTRODEO".to_string(),
                field_type: FieldType::Character,
                length: 8,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "LASTUPDATE".to_string(),
                field_type: FieldType::Character,
                length: 8,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "SORT_DATE".to_string(),
                field_type: FieldType::Character,
                length: 8,
                decimal_count: 0,
                work_area_id: 0,
                example: 1,
            },
            FieldDescriptor {
                name: "EXT_DOLLAR".to_string(),
                field_type: FieldType::Numeric,
                length: 7,
                decimal_count: 2,
                work_area_id: 0,
                example: 1,
            },
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

impl DBaseRecord for RegistrationRecord {
    /// Describe the layout of a registration table.
    ///
    /// The events each have a series of properties, designated by a letter, applied to each day.
    /// It's assumed they have the following meaning, though that's not totally clear:
    /// - E: "Entered" -- This person entered this event.
    /// - S: "Score" -- score received
    /// - T: "Time" -- time taken
    /// - P: "Points" -- points received
    /// - D: "Dollars" -- dollars won
    /// - W: "World" -- world points earned
    fn describe(&self) -> Vec<FieldDescriptor> {
        vec![
            // General details
            FieldDescriptor { name: "IGRA_NUM".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RODEO_NUM".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "STATE_ASSN".to_string(), field_type: FieldType::Character, length: 5, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "SSN".to_string(), field_type: FieldType::Character, length: 11, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DIVISION".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "LAST_NAME".to_string(), field_type: FieldType::Character, length: 17, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FIRST_NAME".to_string(), field_type: FieldType::Character, length: 10, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CITY".to_string(), field_type: FieldType::Character, length: 18, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "STATE".to_string(), field_type: FieldType::Character, length: 2, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "SEX".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },

            // Bull Riding
            FieldDescriptor { name: "BULL_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_S_SAT".to_string(), field_type: FieldType::Numeric, length: 4, decimal_count: 1, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_S_SUN".to_string(), field_type: FieldType::Numeric, length: 4, decimal_count: 1, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BULL_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Bronc Riding
            FieldDescriptor { name: "BRON_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_S_SAT".to_string(), field_type: FieldType::Numeric, length: 4, decimal_count: 1, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_S_SUN".to_string(), field_type: FieldType::Numeric, length: 4, decimal_count: 1, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRON_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Steer Riding (used to be "Wild Cow Riding")
            FieldDescriptor { name: "WCOW_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_S_SAT".to_string(), field_type: FieldType::Numeric, length: 4, decimal_count: 1, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_S_SUN".to_string(), field_type: FieldType::Numeric, length: 4, decimal_count: 1, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "WCOW_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Chute Dogging
            FieldDescriptor { name: "CHUT_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_T_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_T_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CHUT_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Calf Roping on Foot
            FieldDescriptor { name: "CALF_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_T_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_T_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "CALF_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Break-away
            FieldDescriptor { name: "BRAK_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_T_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_T_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BRAK_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Barrel Racing
            FieldDescriptor { name: "BARR_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_T_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_T_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "BARR_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Pole Bending
            FieldDescriptor { name: "POLE_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_T_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_T_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "POLE_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Flag Racing
            FieldDescriptor { name: "FLAG_E_SAT".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_T_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_P_SAT".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_D_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_W_SAT".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_E_SUN".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_T_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_P_SUN".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_D_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG_W_SUN".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // ?? I think these are some sort of scratch fields used by the Clipper program.
            FieldDescriptor { name: "RODEO_SCOR".to_string(), field_type: FieldType::Numeric, length: 5, decimal_count: 1, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RODEO_TIME".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RODEO_ASSO".to_string(), field_type: FieldType::Character, length: 2, decimal_count: 0, work_area_id: 0, example: 1 },

            // Team Roping
            // This event is handled so weirdly to work around how other events are recorded
            // combined with the fact you can participate twice per go, once as header and again as heeler.
            // From what I can tell, HD1E is "X" if the person entered as Header, HD2E is the Heeler's IGRA #,
            // and TIM1/PTS1/DOL1/WOR1 are time/points/dollars/world values when they were heading.
            // Similarly, HL2E is "X" if  they enter as Heeler, HD2E is the Header's IGRA #,
            // and TIM2/PTS2/DOL2/WOR2 are time/points/dollars/world values when they were heeling.
            //
            // NOTE: The "entered" and "partner" fields are swapped between the two entry types!
            FieldDescriptor { name: "TR_HD1E_SA".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_HL1E_SA".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_TIM1_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_PTS1_SA".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_DOL1_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_WOR1_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            FieldDescriptor { name: "TR_HD2E_SA".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_HL2E_SA".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_TIM2_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_PTS2_SA".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_DOL2_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_WOR2_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            FieldDescriptor { name: "TR_HD1E_SU".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_HL1E_SU".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_TIM1_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_PTS1_SU".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_DOL1_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_WOR1_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            FieldDescriptor { name: "TR_HD2E_SU".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_HL2E_SU".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_TIM2_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_PTS2_SU".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_DOL2_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TR_WOR2_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Steer Decorating
            FieldDescriptor { name: "ST_EVNT_SA".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_PART_SA".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_TIME_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_POIN_SA".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_DOLL_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_WORL_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_EVNT_SU".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_PART_SU".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_TIME_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_POIN_SU".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_DOLL_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "ST_WORL_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Wild Drag Race
            FieldDescriptor { name: "DR_EVNT_SA".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_PAR1_SA".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_PAR2_SA".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_TIME_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_POIN_SA".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_DOLL_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_WORL_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_EVNT_SU".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_PAR1_SU".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_PAR2_SU".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_TIME_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_POIN_SU".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_DOLL_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "DR_WORL_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Goat Dressing
            FieldDescriptor { name: "GO_EVNT_SA".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_PART_SA".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_TIME_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_POIN_SA".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_DOLL_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_WORL_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_EVNT_SU".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_PART_SU".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_TIME_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_POIN_SU".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_DOLL_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "GO_WORL_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // ?? From the Clipper program files, I think this is "Ribbon Roping".
            // Maybe an old team event we don't do anymore?
            FieldDescriptor { name: "RR_EVNT_SA".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_PART_SA".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_TIME_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_POIN_SA".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_DOLL_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_WORL_SA".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_EVNT_SU".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_PART_SU".to_string(), field_type: FieldType::Character, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_TIME_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_POIN_SU".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_DOLL_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "RR_WORL_SU".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // ?? In the few rodeo files I have, I see FLAG1 sometimes 'X', but not any instances of FLAG2 set.
            // They might be another scratch space field used by the clipper application.
            FieldDescriptor { name: "FLAG1".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "FLAG2".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 19525, example: 1 },

            // Number of stalls they requested.
            FieldDescriptor { name: "STALL_FLAG".to_string(), field_type: FieldType::Numeric, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "EXTRA_FLAG".to_string(), field_type: FieldType::Character, length: 1, decimal_count: 0, work_area_id: 0, example: 1 },

            // Total points. "EXT" seems unused.
            FieldDescriptor { name: "SAT_POINTS".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "SUN_POINTS".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "EXT_POINTS".to_string(), field_type: FieldType::Numeric, length: 3, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TOT_POINTS".to_string(), field_type: FieldType::Numeric, length: 4, decimal_count: 0, work_area_id: 0, example: 1 },

            // Payment info.
            FieldDescriptor { name: "PRE_DATE".to_string(), field_type: FieldType::Date, length: 8, decimal_count: 0, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "PRE_PAID".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },

            // Total winnings. "EXT" seems unused.
            FieldDescriptor { name: "SAT_DOLLAR".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "SUN_DOLLAR".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "EXT_DOLLAR".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
            FieldDescriptor { name: "TOT_DOLLAR".to_string(), field_type: FieldType::Numeric, length: 7, decimal_count: 2, work_area_id: 0, example: 1 },
        ]
    }

    fn to_record(&self) -> Vec<Field> {
        let mut data = Vec::with_capacity(191);

        data.push(Field::Character(self.igra_number.clone()));
        data.push(Field::Character(self.igra_number.clone())); // RODEO_NUM is IGRA_NUM
        data.push(Field::Character(self.association.clone()));
        data.push(Field::Character(self.ssn.clone()));
        data.push(Field::Character(self.division.clone()));
        data.push(Field::Character(self.last_name.clone()));
        data.push(Field::Character(self.first_name.clone()));
        data.push(Field::Character(self.city.clone()));
        data.push(Field::Character(self.state.clone()));
        data.push(Field::Character(self.sex.clone()));

        for event in [
            "BULL_E_SAT", "BULL_E_SUN",
            "BRON_E_SAT", "BRON_E_SUN",
            "WCOW_E_SAT", "WCOW_E_SUN",
            "CHUT_E_SAT", "CHUT_E_SUN",
            "CALF_E_SAT", "CALF_E_SUN",
            "BRAK_E_SAT", "BRAK_E_SUN",
            "BARR_E_SAT", "BARR_E_SUN",
            "POLE_E_SAT", "POLE_E_SUN",
            "FLAG_E_SAT", "FLAG_E_SUN",
        ] {
            self.add_fields_for(event, true, 0, &mut data);
        }

        // Because the fields come in between, we need to split apart the logic for writing events.
        data.push(Field::Numeric(self.rodeo_score));
        data.push(Field::Numeric(self.rodeo_time));
        data.push(Field::Character(self.rodeo_association.clone()));

        // The "2nd" instances of Team Roping swap the order of entered and partner.
        self.add_fields_for("TR_HD1E_SA", true, 1, &mut data);
        self.add_fields_for("TR_HD2E_SA", false, 1, &mut data);
        self.add_fields_for("TR_HD1E_SU", true, 1, &mut data);
        self.add_fields_for("TR_HD2E_SU", false, 1, &mut data);

        for (event, n_partners) in [
            ("ST_EVNT_SA", 1), ("ST_EVNT_SU", 1),
            ("DR_EVNT_SA", 2), ("DR_EVNT_SU", 2),
            ("GO_EVNT_SA", 1), ("GO_EVNT_SU", 1),
            ("RR_EVNT_SA", 1), ("RR_EVNT_SU", 1),
        ] {
            self.add_fields_for(event, true, n_partners, &mut data);
        }

        data.push(Field::Character(self.flag_1.clone()));
        data.push(Field::Character(self.flag_2.clone()));
        data.push(Field::Numeric(Some(self.stalls)));
        data.push(Field::Character(self.extra_flag.clone()));

        data.push(Field::Numeric(Some(self.sat_points)));
        data.push(Field::Numeric(Some(self.sun_points)));
        data.push(Field::Numeric(Some(self.ext_points)));
        data.push(Field::Numeric(Some(self.tot_points)));

        data.push(Field::Date(self.prepaid_date));
        data.push(Field::Numeric(self.prepaid_amount));

        data.push(Field::Numeric(Some(self.sat_dollars)));
        data.push(Field::Numeric(Some(self.sun_dollars)));
        data.push(Field::Numeric(Some(self.ext_dollars)));
        data.push(Field::Numeric(Some(self.tot_dollars)));

        data
    }
}

/// Read registration/event records from a DBF table.
pub fn read_registrations<R: io::Read>(
    table: TableReader<Header<R>>,
) -> DBaseResult<Vec<RegistrationRecord>> {
    let mut registrations = Vec::<RegistrationRecord>::with_capacity(table.n_records());
    let mut records = table.records();

    while let Some(record) = records.next() {
        let record = record?;

        let mut entrant = RegistrationRecord::default();
        for field in record {
            let f = field?;

            match (f.name, f.value) {
                ("IGRA_NUM", Field::Character(s)) => entrant.igra_number = s,
                ("RODEO_NUM", _) => {} // ignored
                ("STATE_ASSN", Field::Character(s)) => entrant.association = s,
                ("SSN", Field::Character(s)) => entrant.ssn = s,
                ("DIVISION", Field::Character(s)) => entrant.division = s,
                ("LAST_NAME", Field::Character(s)) => entrant.last_name = s,
                ("FIRST_NAME", Field::Character(s)) => entrant.first_name = s,
                ("CITY", Field::Character(s)) => entrant.city = s,
                ("STATE", Field::Character(s)) => entrant.state = s,
                ("SEX", Field::Character(s)) => entrant.sex = s,
                // <individual events appear here in the table layout>
                // ??
                ("RODEO_SCOR", Field::Numeric(n)) => entrant.rodeo_score = n,
                ("RODEO_TIME", Field::Numeric(n)) => entrant.rodeo_time = n,
                ("RODEO_ASSO", Field::Character(s)) => entrant.rodeo_association = s,
                // <team events appear here in the table layout>
                // ??
                ("FLAG1", Field::Character(s)) => entrant.flag_1 = s,
                ("FLAG2", Field::Character(s)) => entrant.flag_2 = s,
                // horse stalls
                ("STALL_FLAG", Field::Numeric(Some(n))) => entrant.stalls = n,
                ("STALL_FLAG", Field::Numeric(None)) => entrant.stalls = Decimal::from(0),
                ("EXTRA_FLAG", Field::Character(s)) => entrant.extra_flag = s,
                // points
                ("SAT_POINTS", Field::Numeric(Some(n))) => entrant.sat_points = n,
                ("SUN_POINTS", Field::Numeric(Some(n))) => entrant.sun_points = n,
                ("EXT_POINTS", Field::Numeric(Some(n))) => entrant.ext_points = n,
                ("TOT_POINTS", Field::Numeric(Some(n))) => entrant.tot_points = n,
                // prepaid amount
                ("PRE_DATE", Field::Date(d)) => entrant.prepaid_date = d,
                ("PRE_PAID", Field::Numeric(val)) => entrant.prepaid_amount = val,
                // winnings
                ("SAT_DOLLAR", Field::Numeric(Some(n))) => entrant.sat_dollars = n,
                ("SUN_DOLLAR", Field::Numeric(Some(n))) => entrant.sun_dollars = n,
                ("EXT_DOLLAR", Field::Numeric(Some(n))) => entrant.ext_dollars = n,
                ("TOT_DOLLAR", Field::Numeric(Some(n))) => entrant.tot_dollars = n,

                // Peel apart other fields identified by pattern matching.
                (event_field, val) => {
                    let (abbrev, field, day) = event_field.split_once('_')
                        .and_then(|(name, rest)| {
                            match rest.split_once('_') {
                                Some((field, day)) => Some((name, field, day)),
                                _ => None,
                            }
                        })
                        .expect(&*format!("Unknown field: '{event_field}' with value '{val:?}'"));


                    // Extract the event name, if its a recognized form.
                    let event = match day {
                        "SAT" | "SUN" => {
                            entrant.events.iter_mut()
                                .find(|e| &e.name[..4] == abbrev && &e.name[7..] == day)
                        }
                        "SA" | "SU" => {
                            match field {
                                // Team Roping doesn't fit the pattern of the rest of the events.
                                // Obnoxiously, 2 of the team roping events list partners before entry.
                                // So, when we encounter HD2E, we don't have an event entry for it yet.
                                // The next block will create the event if they listed a partner,
                                // and we'll see that event when we reach HL2E
                                // We assume that if they had a partner listed, they entered the event.
                                // If they _do_ enter the event _without_ listing a partner,
                                // we'll add the event instance when we see the "X" for entry.
                                // Thankfully, the other fields all come after that point anyway.
                                "HD2E" => { None }
                                "HL2E" | "TIM2" | "PTS2" | "DOL2" | "WOR2" => {
                                    entrant.events.iter_mut()
                                        .find(|e| &e.name[..2] == abbrev
                                            && &e.name[3..7] == "HD2E"
                                            && &e.name[8..] == day
                                        )
                                }
                                _ => {
                                    entrant.events.iter_mut()
                                        .find(|e| &e.name[..2] == abbrev && &e.name[8..] == day)
                                }
                            }
                        }
                        _ => None,
                    };

                    match (field, val, event) {
                        ("E" | "EVNT" | "HD1E" | "HL2E", Field::Character(ref x), None) => {
                            if x == "X" {
                                entrant.events.push(EventRecord {
                                    // TODO: translate the name into a KnownEvent
                                    name: f.name.into(),
                                    ..EventRecord::default()
                                });
                            }
                        }
                        // Create an event for HD2E if they listed a partner.
                        ("HD2E", Field::Character(p), None) => {
                            if !p.is_empty() {
                                entrant.events.push(EventRecord {
                                    name: f.name.into(),
                                    partners: Some(vec![p]),
                                    ..EventRecord::default()
                                });
                            }
                        }
                        ("HL2E", Field::Character(_), Some(_)) => {
                            // See notes above about the weirdness of Team Roping.
                        }
                        (_, _, None) => {} // TODO: make this work better
                        // Score or Time: distinguish whether one is recorded.
                        ("S", Field::Numeric(Some(n)), Some(evnt)) => evnt.outcome = Some(EventMetric::Score(n)),
                        ("T" | "TIME" | "TIM1" | "TIM2", Field::Numeric(Some(n)), Some(e)) => e.outcome = Some(EventMetric::Time(n)),
                        // If the value is None, don't set the outcome field.
                        ("S", Field::Numeric(None), Some(_)) => {}
                        ("T" | "TIME" | "TIM1" | "TIM2", Field::Numeric(None), Some(_)) => {}
                        // Extract points/dollars/world points if set
                        ("P" | "POIN" | "PTS1" | "PTS2", Field::Numeric(Some(n)), Some(e)) => e.points = n,
                        ("P" | "POIN" | "PTS1" | "PTS2", Field::Numeric(None), Some(_)) => {}
                        ("D" | "DOLL" | "DOL1" | "DOL2", Field::Numeric(Some(n)), Some(e)) => e.dollars = n,
                        ("D" | "DOLL" | "DOL1" | "DOL2", Field::Numeric(None), Some(_)) => {}
                        ("W" | "WORL" | "WOR1" | "WOR2", Field::Numeric(Some(n)), Some(e)) => e.world = n,
                        ("W" | "WORL" | "WOR1" | "WOR2", Field::Numeric(None), Some(_)) => {}
                        // Grab partner IGRA numbers.
                        ("PART" | "PAR1" | "PAR2" | "HL1E", Field::Character(p), Some(e)) => {
                            if let Some(ref mut partners) = e.partners {
                                partners.push(p);
                            } else {
                                e.partners = Some(vec![p]);
                            }
                        }
                        (field, val, _) => {
                            panic!("Unknown field: '{field}' with value '{val:?}' for event '{abbrev}' ('{event_field}')");
                        }
                    }
                }
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
            num = self.igra_number,
            gender = self.sex,
            bday = self.birthdate,
            legal = format!("{} {}", self.legal_first, self.legal_last),
            perf = format!("{} {}", self.first_name, self.last_name),
            addr = format!("{} {}, {}", self.address, self.city, self.state),
            assoc = self.association,
        )
    }
}

impl Display for RegistrationRecord {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:4} {:6} {:10} {:17} {cat:7} {:18} {:2}  #={:2}  sat={:5}  sun={:5}  tot={:5}  ext={:5}  pnl={pnl:10.02}",
            self.igra_number,
            self.association,
            self.first_name,
            self.last_name,
            self.city,
            self.state,
            self.events.len(),
            self.sat_points,
            self.sun_points,
            self.tot_points,
            self.ext_points,
            pnl = self.tot_dollars.to_f64_lossy() - (self.events.len() as f64 * 30.0),
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

static STATES: phf::Map<&'static str, &'static str> = phf_map! {
    "Alaska" => "AK",
    "Alabama" => "AL",
    "Arkansas" => "AR",
    "Arizona" => "AZ",
    "California" => "CA",
    "Colorado" => "CO",
    "Connecticut" => "CT",
    "Delaware" => "DE",
    "Florida" => "FL",
    "Georgia" => "GA",
    "Hawaii" => "HI",
    "Iowa" => "IA",
    "Idaho" => "ID",
    "Illinois" => "IL",
    "Indiana" => "IN",
    "Kansas" => "KS",
    "Kentucky" => "KY",
    "Louisiana" => "LA",
    "Massachusetts" => "MA",
    "Maryland" => "MD",
    "Maine" => "ME",
    "Michigan" => "MI",
    "Minnesota" => "MN",
    "Missouri" => "MO",
    "Mississippi" => "MS",
    "Montana" => "MT",
    "North Carolina" => "NC",
    "North Dakota" => "ND",
    "Nebraska" => "NE",
    "New Hampshire" => "NH",
    "New Jersey" => "NJ",
    "New Mexico" => "NM",
    "Nevada" => "NV",
    "New York" => "NY",
    "Ohio" => "OH",
    "Oklahoma" => "OK",
    "Ontario" => "ON",
    "Oregon" => "OR",
    "Pennsylvania" => "PA",
    "Rhode Island" => "RI",
    "South Carolina" => "SC",
    "South Dakota" => "SD",
    "Tennessee" => "TN",
    "Texas" => "TX",
    "Utah" => "UT",
    "Virginia" => "VA",
    "Vermont" => "VT",
    "Washington" => "WA",
    "Wisconsin" => "WI",
    "West Virginia" => "WV",
    "Wyoming" => "WY",

    "District Of Columbia" => "DC",
    "Guam" => "GU",
    "Puerto Rico" => "PR",
    "Virgin Islands" => "VI",

    "Alberta" => "AB",
    "British Columbia" => "BC",
    "Newfoundland and Labrador" => "NF",
    "Manitoba" => "MB",
    "New Brunswick" => "NB",
    "Nova Scotia" => "NS",
    "Northwest Territories" => "NT",
    "Prince Edward Island" => "PE",
    "Quebec" => "PQ",
    "Saskatchewan" => "SK",
    "Yukon Territory" => "YT",

    "Army Europe" => "AE",
    "Canal Zone" => "CZ",
    "Foreign Country" => "FC",
};

impl PersonRecord {
    pub fn region(&self) -> Option<&&'static str> {
        REGIONS.get(&self.state.to_ascii_uppercase())
    }
}

pub static CANADIAN_REGIONS: phf::Set<&'static str> = phf_set! {
    "AB", "BC", "LB", "MB", "NB", "NF", "NS", "NT", "PE", "PQ", "SK", "YT",
};

pub static IGRA_DIVISIONS: phf::Map<&'static str, &'static str> = phf_map! {
    "CRGRA" => "1",
    "DSRA" =>  "3",
    "AGRA" =>  "2",
    "GSGRA" => "1",
    "CGRA" =>  "2",
    "ASGRA" => "4",
    "MIGRA" => "4",
    "NSGRA" => "4",
    "MGRA" => "3",
    "NMGRA" => "2",
    "NGRA" => "1",
    "GPRA" => "3",
    "TGRA" => "3",
    "RRRA" => "3",
    "UGRA" => "2",
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
        let event = match id {
            1 => RodeoEvent::BullRiding,
            2 => RodeoEvent::RanchSaddleBroncRiding,
            // There isn't a 3.
            4 => RodeoEvent::SteerRiding,
            5 => RodeoEvent::ChuteDogging,
            6 => RodeoEvent::CalfRopingOnFoot,
            7 => RodeoEvent::MountedBreakaway,
            8 => RodeoEvent::BarrelRacing,
            9 => RodeoEvent::PoleBending,
            10 => RodeoEvent::FlagRacing,
            11 => RodeoEvent::TeamRopingHeader, // I am not 100% about this and 12.
            12 => RodeoEvent::TeamRopingHeeler,
            13 => RodeoEvent::SteerDecorating,
            14 => RodeoEvent::WildDragRace,
            15 => RodeoEvent::GoatDressing,
            _ => {
                return None;
            }
        };

        Some(event)
    }

    fn event_record_prefix(self) -> &'static str {
        match self {
            RodeoEvent::CalfRopingOnFoot => { "CALF_E" }
            RodeoEvent::MountedBreakaway => { "BRAK_E" }
            RodeoEvent::TeamRopingHeader => { "TR_HD1E" }
            RodeoEvent::TeamRopingHeeler => { "TR_HL2E" }
            RodeoEvent::PoleBending => { "POLE_E" }
            RodeoEvent::BarrelRacing => { "BARR_E" }
            RodeoEvent::FlagRacing => { "FLAG_E" }
            RodeoEvent::ChuteDogging => { "CHUT_E" }
            RodeoEvent::RanchSaddleBroncRiding => { "BRON_E" }
            RodeoEvent::SteerRiding => { "WCOW_E" }
            RodeoEvent::BullRiding => { "BULL_E" }
            RodeoEvent::GoatDressing => { "GO_EVNT" }
            RodeoEvent::SteerDecorating => { "ST_EVNT" }
            RodeoEvent::WildDragRace => { "DR_EVNT" }
        }
    }

    /// Given a round, what should the name be?
    ///
    /// Returns `None` if the round is not 1 or 2,
    /// as the original system only considered Saturday and Sunday.
    fn construct_name(self, round: u64) -> Option<String> {
        match self {
            RodeoEvent::TeamRopingHeader
                | RodeoEvent::TeamRopingHeeler
                | RodeoEvent::SteerDecorating
                | RodeoEvent::WildDragRace
                | RodeoEvent::GoatDressing => {
                  if round == 1 {
                      return Some(format!("{}_SA", self.event_record_prefix()));
                  } else if round == 2 {
                      return Some(format!("{}_SU", self.event_record_prefix()));
                  } else {
                      return None;
                  }
            },
            RodeoEvent::CalfRopingOnFoot 
                | RodeoEvent::MountedBreakaway 
                | RodeoEvent::PoleBending 
                | RodeoEvent::BarrelRacing 
                | RodeoEvent::FlagRacing 
                | RodeoEvent::ChuteDogging 
                | RodeoEvent::RanchSaddleBroncRiding 
                | RodeoEvent::SteerRiding 
                | RodeoEvent::BullRiding => { 
                  if round == 1 {
                      return Some(format!("{}_SAT", self.event_record_prefix()));
                  } else if round == 2 {
                      return Some(format!("{}_SUN", self.event_record_prefix()));
                  } else {
                      return None;
                  }
             },
        }
    }
}

#[cfg(test)]
mod test {
    use super::RodeoEvent;
    #[test]
    fn name_from_event() {
        let name = RodeoEvent::TeamRopingHeader.construct_name(1);
        assert_eq!(name, Some("TR_HD1E_SA".into()));
    }
}
