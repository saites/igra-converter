mod xbase;

use std::fmt::{Display, Formatter};
use std::io;
use std::io::{BufReader};
use std::fs::File;
use std::iter::zip;
use eddie::DamerauLevenshtein;
use itertools::Itertools;

use log;

use crate::xbase::{DBaseTable, Decimal, FieldDescriptor, Field, DBaseResult, TableReader, Header};

fn main() -> DBaseResult<()> {
    env_logger::init();

    /*
    let az_events_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/AZEVENTS.DBF";
    let (az_events_dbase, mmapped) = DBaseTable::try_open(az_events_path).expect("opened dbase");
    read_rodeo_events(&mmapped[az_events_dbase.n_header_bytes()..], &az_events_dbase.fields);
     */

    let personnel_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/PERSONEL.DBF";
    let dbt = xbase::try_from_path(personnel_path)?;

    let mut people = read_personnel(dbt)?;
    let damlev = DamerauLevenshtein::new();
    println!("{}", people.len());

    let mut compared: Vec<(usize, &PersonRecord, &PersonRecord)> = people.iter()
        .take(500)
        .tuple_combinations()
        .map(|(p0, p1)| {
            let sim = (
                damlev.distance(&p0.first_name, &p1.first_name)
                    + damlev.distance(&p0.last_name, &p1.last_name)
            );
            (sim, p0, p1)
        }
    ).sorted_unstable_by(
        |a, b| (a.0).cmp(&b.0)
    ).collect();

    for (sim, p0, p1) in compared.iter().take(100) {
        println!("{:15} {:<20} | {:15} {:<20} | {}",
                 p0.first_name, p0.last_name, p1.first_name, p1.last_name,
                 sim)
    }

    Ok(())
}

/// An event is scored using either Time or Score.
#[allow(dead_code)]
#[derive(Debug)]
enum EventMetric {
    Time(Decimal),
    Score(Decimal),
}

/// Actual results for an event.
#[derive(Debug, Default)]
struct EventRecord {
    name: String,
    outcome: Option<EventMetric>,
    dollars: Decimal,
    points: Decimal,
    world: Decimal,
}

/// Headers used for event registration.
#[allow(unused)]
#[derive(Debug, Default)]
struct RegistrationHeader {
    event_name: &'static str,
    entered: &'static str,
    outcome: &'static str,
    dollars: &'static str,
    points: &'static str,
    world: &'static str,

}

#[allow(unused)]
#[derive(Debug, Default)]
struct PersonRecord {
    igra_number: String,
    association: String,
    birthdate: String,
    ssn: String,
    division: String,
    last_name: String,
    first_name: String,
    legal_last: String,
    legal_first: String,
    id_checked: String,
    sex: String,

    address: String,
    city: String,
    state: String,
    zip: String,
    home_phone: String,
    cell_phone: String,
    email: String,
    status: String,

    first_rodeo: String,
    last_updated: String,
    sort_date: String,
    ext_dollars: Decimal,
}

#[allow(unused)]
#[derive(Debug, Default)]
struct RegistrationRecord {
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

#[derive(Clone)]
struct IGRANumber(String);

#[derive(Clone)]
struct LegalLast(String);

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

fn read_personnel<R: io::Read>(table: TableReader<Header<R>>) -> DBaseResult<Vec::<PersonRecord>> {
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

fn read_rodeo_events(mmapped: &[u8], fields: &Vec<FieldDescriptor>) {
    let mut i = 0;
    while i + 1 < mmapped.len() {
        i += 1;
        let mut entrant = RegistrationRecord::default();

        for f in fields {
            let r = f.read_field(&mmapped[i..i + f.length]);
            i += f.length as usize;

            if r.is_err() {
                log::error!("{:?}", r);
                continue;
            }

            if f.name.ends_with("_SAT") || f.name.ends_with("_SUN") {
                let is_x = if let Ok(Field::Character(ref x)) = r { x == "X" } else { false };

                if &f.name[5..6] == "E" && is_x {
                    let mut evnt = EventRecord::default();
                    evnt.name = f.name.clone();
                    entrant.events.push(evnt);
                } else if let Some(evnt) = entrant.events.iter_mut().find(|e| {
                    e.name[..4] == f.name[..4] && e.name[6..] == f.name[6..]
                }) {
                    match (&f.name.as_str()[5..6], r.unwrap()) {
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

            match (&f.name.as_str(), r.unwrap()) {
                (&"IGRA_NUM", Field::Character(s)) => entrant.igra_number = s,
                (&"FIRST_NAME", Field::Character(s)) => entrant.first_name = s,
                (&"LAST_NAME", Field::Character(s)) => entrant.last_name = s,
                (&"SEX", Field::Character(s)) => entrant.sex = s,
                (&"CITY", Field::Character(s)) => entrant.city = s,
                (&"STATE", Field::Character(s)) => entrant.state = s,
                (&"STATE_ASSN", Field::Character(s)) => entrant.association = s,
                (&"SSN", Field::Character(s)) => entrant.ssn = s,
                (&"SAT_POINTS", Field::Numeric(Some(n))) => entrant.sat_points = n,
                (&"SUN_POINTS", Field::Numeric(Some(n))) => entrant.sun_points = n,
                (&"EXT_POINTS", Field::Numeric(Some(n))) => entrant.ext_points = n,
                (&"TOT_POINTS", Field::Numeric(Some(n))) => entrant.tot_points = n,
                _ => {}
            }
        }

        println!("{}", entrant);
        for evnt in entrant.events {
            println!("\t{}", evnt);
        }
    }
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
