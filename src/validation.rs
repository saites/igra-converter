use std::fmt::{Display, Formatter};
use serde::{Deserialize, Serialize};
use std::io;
use std::collections::{hash_map, HashMap};
use std::error::Error;
use std::io::BufReader;
use std::fs::File;
use std::ops::Deref;
use eddie::DamerauLevenshtein;
use std::iter::zip;
use std::cmp::Ordering;
use std::rc::Rc;
use phf::phf_map;

use crate::bktree;
use crate::bktree::BKTree;
use crate::robin::{Event, EventID, Registration};
use crate::robin::EventID::Known;
use crate::xbase::{DBaseResult, Decimal, Field, Header, TableReader};

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
    where T: std::hash::Hash + Eq
{
    fn new() -> Self {
        DistCounter(HashMap::<T, (u64, usize)>::new())
    }

    /// Insert T with the given distance.
    /// If T is already present, adds the distance to its sum.
    fn insert(&mut self, dist: usize, pr: T) {
        self.0.entry(pr)
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
    fn best(self, min_hits: u64, max_dist_sum: Option<usize>) -> Vec<(T, (u64, usize))> {
        let mut best: Vec<_> = if let Some(max_dist_sum) = max_dist_sum {
            self.0.into_iter()
                .filter(|(_, (hits, d_sum))| *hits >= min_hits && *d_sum <= max_dist_sum)
                .collect()
        } else {
            self.0.into_iter()
                .filter(|(_, (hits, _))| *hits >= min_hits)
                .collect()
        };

        best.sort_by(
            |(_, (hits0, d_sum0)), (_, (hits1, d_sum1))| {
                let h_cmp = hits1.cmp(hits0);  // prefer more hits
                if h_cmp.is_eq() { d_sum0.cmp(d_sum1) } else { h_cmp } // and less distance
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

impl<'a> EntryValidator<'a> {
    pub(crate) fn new(people: &'a Vec<PersonRecord>) -> Self {
        let mut ev = EntryValidator {
            by_igra_num: BKTree::new(),
            by_first_name: BKTree::new(),
            by_last_name: BKTree::new(),
            by_perf_first: BKTree::new(),
            by_perf_last: BKTree::new(),
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
    ///
    /// TODO: refactor the mess below
    pub fn validate_entries(&self, entries: &'a Vec<Registration>) -> Report<'a> {
        let damlev = DamerauLevenshtein::new();
        let today = chrono::Utc::now().naive_utc().date();

        let mut results: Vec<Processed> = Vec::with_capacity(entries.len());
        let mut relevant = HashMap::<&str, &PersonRecord>::new();

        for r in entries {
            let mut issues = Vec::<Suggestion>::new();
            let mut confirmed_partners = HashMap::<&PersonRecord, Vec<(RodeoEvent, RoundID)>>::new();

            let n = &r.contestant;
            log::debug!("Entry: {first:15} {last:<20} : {id}",
                    first = n.first_name, last = n.last_name, id = n.association.igra);

            // Force this value into a boolean.
            let is_member = n.is_member == "yes";

            // Convert records to match database format.
            let igra_num = n.association.igra.trim();
            let first_name = n.first_name.trim().to_ascii_uppercase();
            let last_name = n.last_name.trim().to_ascii_uppercase();
            let dob = n.dob.dos();
            let ssn = n.dos_ssn();

            // Validate their age is at least 18.
            if n.dob.naive_date().and_then(|d| today.years_since(d)).map_or(true, |age| age < 18) {
                issues.push(Suggestion { problem: Problem::NotOldEnough, fix: Fix::ContactRegistrant });
            }

            // Make sure they registered for at least two go-rounds.
            if r.events.len() < 2 {
                issues.push(Suggestion { problem: Problem::NotEnoughRounds, fix: Fix::ContactRegistrant });
            }

            self.validate_events(&r, &mut issues, &mut confirmed_partners, &mut relevant);

            // Search for members that closely match the registration.
            let mut candidates = DistCounter::<&PersonRecord>::new();

            if is_member {
                if igra_num.is_empty() {
                    issues.push(Suggestion { problem: Problem::NoValue { field: RegF::IGRANumber }, fix: Fix::ContactRegistrant })
                } else {
                    self.by_igra_num
                        .find_by(1, |x| damlev.distance(igra_num, &x.0.igra_number))
                        .into_iter()
                        .for_each(|(d, r)| candidates.insert(d, r.0));
                }
            }

            if first_name.is_empty() {
                issues.push(Suggestion { problem: Problem::NoValue { field: RegF::LegalFirst }, fix: Fix::ContactRegistrant })
            } else {
                self.by_first_name
                    .find_by(2, |x| damlev.distance(&first_name, &x.0.legal_first))
                    .into_iter()
                    .for_each(|(d, r)| candidates.insert(d, r.0));
            }

            if last_name.is_empty() {
                issues.push(Suggestion { problem: Problem::NoValue { field: RegF::LegalLast }, fix: Fix::ContactRegistrant })
            } else {
                self.by_last_name
                    .find_by(2, |x| damlev.distance(&last_name, &x.0.legal_last))
                    .into_iter()
                    .for_each(|(d, r)| candidates.insert(d, r.0));
            }

            // Filter out candidates that don't match on at least 2 fields.
            // Sort them by the number of field matches, then by total distance.
            let mut candidates = candidates.best(2, None);

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

            let mut found = None;

            if !is_member {
                candidates.retain(|(member, _)| exact(member));
                if candidates.is_empty() {
                    // They say they're not a member, and they're probably right.
                    issues.push(Suggestion { problem: Problem::NotAMember, fix: Fix::AddNewMember })
                } else {
                    // They say they're not a member, but these look really close.
                    for (r, _) in candidates {
                        issues.push(Suggestion {
                            problem: Problem::MaybeAMember,
                            fix: Fix::UseThisRecord(IGRANumber(r.igra_number.clone())),
                        });
                        relevant.insert(&r.igra_number, r);
                    }
                }
            } else {
                if candidates.is_empty() {
                    // They say they're a member, but there aren't even close matches.
                    issues.push(Suggestion { problem: Problem::NoPerfectMatch, fix: Fix::ContactRegistrant });
                } else if let Some((m, _)) = candidates.iter().filter(
                    |(member, _)| exact(member) && member.igra_number == igra_num).next() {
                    // This looks to be the right person.

                    // This macro checks if two strings are equal ignoring ascii case,
                    // and if not, adds an issue noting the database field should be updated.
                    macro_rules! check (
                        ($lval:expr, $rval:expr, $field:expr) => (
                            if !$lval.trim().eq_ignore_ascii_case(&$rval.trim()) {
                                issues.push(Suggestion{
                                    problem: Problem::DbMismatch{field: $field},
                                    fix: Fix::UpdateDatabase
                                })
                            }
                        );
                    );

                    // See if other details have changed.
                    // Note that DB fields are (typically) uppercase.
                    check!(m.email, n.address.email, RegF::Email);
                    check!(m.association, n.association.member_assn, RegF::Association);

                    // In the database, most people performance names match their legal names.
                    // If the user left it blank, we probably should should ignore it.
                    // Otherwise, we compare the given value against the concatenated "First Last" DB values.
                    if !n.performance_name.trim().is_empty() {
                        let db_perf_name = format!("{} {}", m.first_name, m.last_name);
                        check!(db_perf_name, n.performance_name, RegF::PerformanceName);
                    }

                    // Address in the database use only a single line.
                    let addr = format!("{} {}", n.address.address_line_1, n.address.address_line_2);
                    check!(m.address, addr, RegF::AddressLine);
                    check!(m.city, n.address.city, RegF::City);
                    check!(m.zip, n.address.zip_code, RegF::PostalCode);

                    // The DB uses two letter abbreviations for states,
                    // and it uses the field for Canadian provinces,
                    // and calls everything else "FC" for "Foreign Country".
                    let is_us_or_can = n.address.country == "United States" || n.address.country == "Canada";
                    if m.state == "FC" {
                        if is_us_or_can {
                            issues.push(Suggestion { problem: Problem::DbMismatch { field: RegF::Country }, fix: Fix::UpdateDatabase });
                        }
                    } else {
                        if !is_us_or_can {
                            issues.push(Suggestion { problem: Problem::DbMismatch { field: RegF::Country }, fix: Fix::UpdateDatabase });
                        }
                        match m.region() {
                            Some(db_region) => check!(db_region, n.address.region, RegF::Region),
                            _ => issues.push(Suggestion { problem: Problem::DbMismatch { field: RegF::Region }, fix: Fix::UpdateDatabase }),
                        }
                    }

                    // The DB formats phone numbers as (xxx)xxx-xxxx,
                    // so we'll strip those extra characters out to compare them.
                    let phone = m.cell_phone.replace(&['(', ')', '-'], "");
                    log::debug!("{phone}");
                    check!(phone, n.address.cell_phone_no, RegF::CellPhone);
                    let phone = m.home_phone.replace(&['(', ')', '-'], "");
                    log::debug!("{phone}");
                    check!(phone, n.address.home_phone_no, RegF::HomePhone);

                    // The DB stores "sex", the form reports "gender",
                    // but what we actually care about who you're competing with.
                    match (m.sex.as_str(), n.gender.as_str()) {
                        ("M", "Cowboys") | ("F", "Cowgirls") => {}
                        _ => {
                            issues.push(Suggestion {
                                problem: Problem::DbMismatch { field: RegF::CompetitionCategory },
                                fix: Fix::UpdateDatabase,
                            })
                        }
                    }

                    found = Some(m.igra_number.as_str());
                    relevant.insert(&m.igra_number, *m);
                } else {
                    // We didn't find them, but maybe we found some close matches.
                    for (r, _) in candidates {
                        issues.push(Suggestion {
                            problem: Problem::NoPerfectMatch,
                            fix: Fix::UseThisRecord(IGRANumber(r.igra_number.clone())),
                        });
                        relevant.insert(&r.igra_number, r);
                    }
                }
            }

            let partners = confirmed_partners.iter().flat_map(|(person, events)| {
                events.iter().map(|(event, round)| {
                    Partner{
                        igra_number: &person.igra_number,
                        event: *event,
                        round: *round,
                    }
                })
            }).collect();

            results.push(Processed {
                registration: r,
                found,
                issues,
                partners,
                confirmed_partners,
            });
        }

        let mut more_issues: Vec<Vec<Suggestion>> = results.iter()
            .filter_map(|result| {
                result.found.and_then(|f| relevant.get(f)).zip(Some(result))
            })
            .map(|(person_a, entry_a)| {
                validate_cross_reg(&results, person_a, entry_a)
            })
            .collect();
        // validate_cross_reg(&results, result)
        for (v, mi) in zip(&mut results, &mut more_issues) {
            for sugg in mi.iter() {
                let other = match &sugg.fix {
                    Fix::AddRegistration(igra_num) => {
                        self.by_igra_num.find_closest(
                            0, |r| damlev.distance(&r.0.igra_number, &igra_num.0),
                        )
                    }
                    Fix::UseThisRecord(igra_num) => {
                        self.by_igra_num.find_closest(
                            0, |r| damlev.distance(&r.0.igra_number, &igra_num.0),
                        )
                    }
                    _ => { None }
                };
                if let Some((0, o)) = other {
                    relevant.insert(&o.0.igra_number, o.0);
                }
            }

            v.issues.append(mi);
        }

        Report {
            results,
            relevant,
        }
    }

    fn validate_events(&self, r: &Registration,
                       issues: &mut Vec<Suggestion>,
                       list_partners: &mut HashMap<&'a PersonRecord, Vec<(RodeoEvent, RoundID)>>,
                       possible_others: &mut HashMap<&'a str, &'a PersonRecord>,
    ) {
        for event in &r.events {
            if event.round > 2 {
                issues.push(Suggestion { problem: Problem::InvalidRoundID { event: event.id, round: event.round }, fix: Fix::ContactDevelopers });
            }

            let db_event = if let Known(expected) = event.id {
                expected
            } else {
                // We don't have this event mapping.
                issues.push(Suggestion { problem: Problem::UnknownEventID { event: event.id }, fix: Fix::ContactDevelopers });
                continue;
            };

            self.validate_partners(&event, db_event, issues, list_partners, possible_others);
        }
    }

    fn validate_partners(&self, event: &Event, db_event: RodeoEvent,
                         issues: &mut Vec<Suggestion>,
                         confirmed_partners: &mut HashMap<&'a PersonRecord, Vec<(RodeoEvent, RoundID)>>,
                         possible_others: &mut HashMap<&'a str, &'a PersonRecord>,
    ) {
        let damlev = DamerauLevenshtein::new();
        let partners: Vec<_> = event.partners.iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();

        match (partners.len() as u64).cmp(&(db_event.num_partners() as u64)) {
            Ordering::Less => {
                issues.push(Suggestion {
                    problem: Problem::TooFewPartners { event: db_event, round: event.round },
                    fix: Fix::ContactRegistrant,
                });
            }
            Ordering::Greater => {
                issues.push(Suggestion {
                    problem: Problem::TooManyPartners { event: event.id, round: event.round },
                    fix: Fix::ContactDevelopers,
                });
            }
            Ordering::Equal => {}
        }

        for p in partners {
            // The interface asks for "PARTNER NAME | IGRA #",
            // intending for people to enter one or the other (if known),
            // but we'll also handle the case where people take it literally
            // and enter both with a a pipe between them.
            //
            // If it doesn't have a pipe, parse it as a number or consider it a name.
            // If it has a pipe, split it at the pipe.
            // If the left is a number, consider the right a name, or try the opposite.
            // If neither looks like a number, then consider the original string the name.
            //
            // Though it's true for now, this logic relies on IGRA identifiers truly being numbers.
            let (part_num, part_name) = match p.split_once('|') {
                None => {
                    // No pipe. Can it be parsed as a number?
                    p.trim().parse::<u64>().map(|num| (Some(num), None))
                        .unwrap_or((None, Some(p)))
                }
                Some((a, b)) => {
                    // If one can be parsed as a number, let the other be the name.
                    let num_first = a.trim().parse::<u64>().map(|num| (Some(num), Some(b)));
                    let name_first = b.trim().parse::<u64>().map(|num| (Some(num), Some(a)));
                    num_first.or(name_first).unwrap_or((None, Some(p)))
                }
            };
            let part_num = part_num.map(|num| format!("{:04}", num));

            // Search for members that closely match.
            let mut p_finder = DistCounter::<&PersonRecord>::new();
            if let Some(ref part_num) = part_num {
                let found = self.by_igra_num.find_by(
                    1, |x| damlev.distance(part_num, &x.0.igra_number));

                // Since they gave a number, we'll break early if we found the right record.
                let mut exact = found.iter().filter(|(d, _)| *d == 0).take(2);
                let perfect = exact.next();
                let too_many = exact.next();
                if perfect.is_some() && too_many.is_none() {
                    // If they gave a name, too, make sure it matches the performance or legal name.
                    let (_, candidate) = perfect.unwrap();
                    let is_match = part_name.map_or(true, |part_name| {
                        let perf_name = format!("{} {}", candidate.0.first_name, candidate.0.last_name);
                        let legal_name = format!("{} {}", candidate.0.legal_first, candidate.0.legal_last);
                        perf_name.eq_ignore_ascii_case(part_name.trim())
                            || legal_name.eq_ignore_ascii_case(part_name.trim())
                    });

                    if is_match {
                        match confirmed_partners.entry(candidate.0) {
                            hash_map::Entry::Occupied(mut e) => { e.get_mut().push((db_event, event.round)); }
                            hash_map::Entry::Vacant(e) => { e.insert(vec![(db_event, event.round)]); }
                        }
                        continue;
                    } else {
                        // Not a name match, but at least the number is right.
                        issues.push(Suggestion {
                            problem: Problem::UnknownPartner { event: db_event, round: event.round },
                            fix: Fix::UseThisRecord(IGRANumber(candidate.0.igra_number.clone())),
                        });
                        possible_others.insert(&candidate.0.igra_number, candidate.0);
                    }
                }

                // Otherwise, we'll need to make a suggestion.
                found.into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
            }

            // Either we didn't have a number to go on, or it didn't match,
            // so search for close matches by comparing the name.
            if let Some(part_name) = part_name {
                let search_dist = 3;

                // Assume we can split it into two parts.
                // If not, we'll search for first names and last names that match the input.
                let (first, last) = part_name.split_once(' ').unwrap_or((part_name, part_name));
                let first = first.trim().to_ascii_uppercase();
                let last = last.trim().to_ascii_uppercase();
                self.by_first_name.find_by(search_dist, |x| damlev.distance(&first, &x.0.legal_first))
                    .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
                self.by_last_name.find_by(search_dist, |x| damlev.distance(&last, &x.0.legal_last))
                    .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
                self.by_perf_first.find_by(search_dist, |x| damlev.distance(&first, &x.0.first_name))
                    .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
                self.by_perf_last.find_by(search_dist, |x| damlev.distance(&last, &x.0.last_name))
                    .into_iter().for_each(|(d, r)| p_finder.insert(d, r.0));
            }

            let possible: Vec<_> = p_finder.best(3, Some(8))
                .into_iter().map(|(p, _)| p).collect();

            // See if we have a single, exact match.
            if let Some(ref part_name) = part_name {
                let mut maybe_partners = possible.iter().filter(|candidate| {
                    let perf_name = format!("{} {}", candidate.first_name, candidate.last_name);
                    let legal_name = format!("{} {}", candidate.legal_first, candidate.legal_last);

                    perf_name.eq_ignore_ascii_case(part_name.trim())
                        || legal_name.eq_ignore_ascii_case(part_name.trim())
                });
                let first = maybe_partners.next();
                let second = maybe_partners.next();

                if first.is_some() && second.is_none() {
                    match confirmed_partners.entry(first.unwrap()) {
                        hash_map::Entry::Occupied(mut e) => { e.get_mut().push((db_event, event.round)); }
                        hash_map::Entry::Vacant(e) => { e.insert(vec![(db_event, event.round)]); }
                    }
                    continue;
                }
            }

            // If not, mark the problem and suggest options, if possible.
            if possible.is_empty() {
                issues.push(Suggestion { problem: Problem::UnknownPartner { event: db_event, round: event.round }, fix: Fix::ContactRegistrant })
            }

            possible.into_iter().for_each(|p| {
                issues.push(Suggestion {
                    problem: Problem::UnknownPartner { event: db_event, round: event.round },
                    fix: Fix::UseThisRecord(IGRANumber(p.igra_number.clone())),
                });
                possible_others.insert(&p.igra_number, p);
            });
        }
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
fn validate_cross_reg(entries: &Vec<Processed>, person_a: &PersonRecord, entry_a: &Processed) -> Vec<Suggestion> {
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
        let b_to_a = entry_b.map(|b| b.confirmed_partners.get(person_a)).flatten();
        for (a_event, a_round) in a_events_with_b {
            if entry_b.is_none() {
                issues.push(Suggestion {
                    problem: Problem::UnregisteredPartner { event: *a_event, round: *a_round },
                    fix: Fix::AddRegistration(IGRANumber(person_b.igra_number.clone())),
                });
                continue;
            }

            let b_listed_a = b_to_a.map_or(false, |listings| listings.iter()
                .any(|(b_event, b_round)| b_event == a_event && b_round == a_round));

            // A listed B, but B didn't list A.
            if !b_listed_a {
                issues.push(Suggestion {
                    problem: Problem::MismatchedPartners {
                        event: *a_event,
                        round: *a_round,
                        partner: IGRANumber(person_b.igra_number.clone()),
                    },
                    fix: Fix::ContactRegistrant,
                });
            }
        }
    }

    issues
}

#[allow(unused)]
#[derive(Eq, Hash, PartialEq)]
#[derive(Debug, Copy, Clone)]
#[derive(Serialize, Deserialize)]
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
    UnknownPartner { event: RodeoEvent, round: RoundID },
    /// We have a matching database record for the partner,
    /// but they haven't registered yet.
    UnregisteredPartner { event: RodeoEvent, round: RoundID },
    /// We have a matching database record for the partner,
    /// that person has registered, but they listed someone else or no one at all.
    MismatchedPartners { event: RodeoEvent, round: RoundID, partner: IGRANumber },

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
    pub igra_number: &'a str,
}

/// This is the result of processing registration data.
#[derive(Debug, Serialize)]
pub struct Processed<'a> {
    pub registration: &'a Registration,
    pub found: Option<&'a str>,
    pub issues: Vec<Suggestion>,
    pub partners: Vec<Partner<'a>>,

    #[serde(skip)]
    confirmed_partners: HashMap<&'a PersonRecord, Vec<(RodeoEvent, RoundID)>>,
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
pub fn read_personnel<R: io::Read>(table: TableReader<Header<R>>) -> DBaseResult<Vec<PersonRecord>> {
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
                ("HOME_PHONE", Field::Character(s)) => person.home_phone = s,
                ("CELL_PHONE", Field::Character(s)) => person.cell_phone = s,
                ("E_MAIL", Field::Character(s)) => person.email = s,
                ("STATUS", Field::Character(s)) => person.status = s,
                ("FIRSTRODEO", Field::Character(s)) => person.first_rodeo = s,
                ("LASTUPDATE", Field::Character(s)) => person.last_updated = s,
                ("SORT_DATE", Field::Character(s)) => person.sort_date = s,
                ("EXT_DOLLAR", Field::Numeric(Some(n))) => person.ext_dollars = n,
                _ => {}
            }
        }

        people.push(person);
    }

    people.sort_by(|a, b| a.igra_number.cmp(&b.igra_number));
    Ok(people)
}

/// Read event records from a DBF table.
fn read_rodeo_events<R: io::Read>(table: TableReader<Header<R>>) -> DBaseResult<Vec<RegistrationRecord>> {
    let mut registrations = Vec::<RegistrationRecord>::with_capacity(table.n_records());
    let mut records = table.records();

    while let Some(record) = records.next() {
        let record = record?;

        let mut entrant = RegistrationRecord::default();
        for field in record {
            let f = field?;

            if f.name.ends_with("_SAT") || f.name.ends_with("_SUN") {
                let is_x = if let Field::Character(ref x) = f.value { x == "X" } else { false };

                if &f.name[5..6] == "E" && is_x {
                    let mut evnt = EventRecord::default();
                    evnt.name = f.name.into();
                    entrant.events.push(evnt);
                } else if let Some(evnt) = entrant.events.iter_mut().find(|e| {
                    e.name[..4] == f.name[..4] && e.name[6..] == f.name[6..]
                }) {
                    match (&f.name[5..6], f.value) {
                        ("S", Field::Numeric(Some(n))) => { evnt.outcome = Some(EventMetric::Score(n)) }
                        ("T", Field::Numeric(Some(n))) => { evnt.outcome = Some(EventMetric::Time(n)) }
                        ("P", Field::Numeric(Some(n))) => { evnt.points = n }
                        ("D", Field::Numeric(Some(n))) => { evnt.dollars = n }
                        ("W", Field::Numeric(Some(n))) => { evnt.world = n }
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
                write!(f, "{:10}: score={s:5}  dollars=${:5}  points={:5}  world={:5}",
                       self.name, self.dollars, self.points, self.world,
                )
            }
            Some(EventMetric::Time(t)) => {
                write!(f, "{:10}:  time={t:5}  dollars=${:5}  points={:5}  world={:5}",
                       self.name, self.dollars, self.points, self.world,
                )
            }
        }
    }
}

impl Display for PersonRecord {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "    {:4}   {:1}   {:1}   {:1}   {:26} {:22} {:5}",
               self.igra_number,
               self.sex,
               self.division,
               self.id_checked,
               format!("{}, {}", self.last_name, self.first_name),
               format!("{}, {}", self.city, self.state),
               self.association,
        )
    }
}

impl Display for RegistrationRecord {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:4} {:6} {:10} {:17} {cat:7} {:18} {:2}  sat={:5}  sun={:5}  tot={:5}  ext={:5}",
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
    fn region(&self) -> Option<&&'static str> {
        REGIONS.get(&self.state)
    }
}

#[allow(unused)]
#[derive(Deserialize, Serialize)]
#[derive(Debug, Copy, Clone, Eq, Hash, PartialEq)]
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
    fn num_partners(self) -> u8 {
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
            _ => { return None; }
        };

        Some(event)
    }
}
