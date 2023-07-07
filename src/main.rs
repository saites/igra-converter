mod xbase;
mod bktree;
mod robin;
mod validation;

use std::{env, io};
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter};
use std::io::Write;
use itertools::Itertools;

use log;
use serde::Serialize;
use validation::{EntryValidator, PersonRecord, Processed};
use crate::robin::{Contestant, Registration};
use crate::validation::{Fix, Problem, Report, Suggestion};

use rand::prelude::*;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let mut args = env::args().skip(1);
    let personnel_path = args.next().expect("first arg should be the dbf file");
    let reg_path = args.next().expect("second arg should be a JSON file");

    log::debug!("Personnel File: {personnel_path}, Registration File: {reg_path}");

    let reg = validation::read_reg(reg_path)?;
    log::debug!("Registration File:\n{:#?}", reg);

    let dbt = xbase::try_from_path(personnel_path)?;
    // let mut tw = xbase::TableWriter::new(BufWriter::new(File::create("./random-data.dbf")?))?;
    // tw.add_fields(&dbt);
    // let people = generate_fake(10000)?;
    // tw.write_records(&people)?;
    let people = validation::read_personnel(dbt)?;

    log::info!("Number of people in personnel database: {}", people.len());
    log::info!("Number of entries JSON file: {}", reg.len());

    let validator = EntryValidator::new(&people);
    let report = validator.validate_entries(&reg);

    write_output(
        io::stdout(),
        &report,
        &people,
    )?;

    // let j = serde_json::to_string_pretty(&report::Report::from_processed(&validation))?;
    let j = serde_json::to_string_pretty(&report)?;
    write!(BufWriter::new(File::create("./web/validation_output.json")?), "{j}")?;

    /*
    let proc = report::Report::from_processed(&validation);
    let j = serde_json::to_string_pretty(&proc)?;

    let j = serde_json::to_string_pretty(&validation)?;
    println!("{j}");
     */


    Ok(())
}

fn generate_fake_reg(db: Vec<PersonRecord>, n: usize) -> Result<Vec<Registration>, Box<dyn Error>> {
    let mut rng = thread_rng();
    let mut res = Vec::with_capacity(n);

    let event_names = vec![]; // TODO

    for _ in 0..n {
        let r = db.choose(&mut rng).unwrap();

        // TODO: decide if this should be a bad record
        // TODO 
        let events = vec![
            robin::Event{
                id: event_names.choose(&mut rng).unwrap(),
                partners: vec![],
                round: 1,
            },
        ];
        let payment = Payment { total: events.len() * 30 };

        let perf_name = format!(
            "{} {}", r.first_name, r.last_name
            );
        let dob_y = r.birthdate[:4].parse<u16>();
        let dob_m = r.birthdate[4:6].parse<u8>();
        let dob_d = r.birthdate[6:].parse<u8>();

        let pr = Registration{
            id: rng.gen(),
            stalls: "".to_string(),
            contestant: Contestant {
                first_name: r.legal_first.clone(),
                last_name: r.legal_last.clone(),
                performance_name: perf_name, 
                dob: Date {
                    year: dob_y,
                    month: dob_m,
                    day: dob_d,
                },
                age: 0,
                gender: if r.sex == "M" { "Cowboys".to_string() } else { "Cowgirls".to_string() },
                is_member: "Yes".to_string(),
                ssn: r.ssn.clone(), 
                note_to_director: "".to_string(),
                address: Address {
                    email: r.email.clone(),
                    address_line_1: r.address.clone(), 
                    address_line_2: "".to_string(),
                    city: r.city.clone(),
                    region: r.state.clone(), 
                    country: "United States".to_string(),
                    zip_code: r.zip.clone(), 
                    cell_phone_no: r.cell_phone.clone(), 
                    home_phone_no: r.home_phone.clone(), 
                },
                association: Association { 
                    igra: r.igra_number.clone(), 
                    member_assn: r.association.clone(), 
                },
            },
            events,
            payment,
        }
        res.push(pr);
    }


    Ok(res)
}

fn generate_fake_db(n: usize) -> Result<Vec<PersonRecord>, Box<dyn Error>> {
    let first_names: Vec<_> = BufReader::new(
        File::open("./common_first_names.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let last_names: Vec<_> = BufReader::new(
        File::open("./common_last_names.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let cities: Vec<_> = BufReader::new(
        File::open("./common_cities.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let regions: Vec<_> = BufReader::new(
        File::open("./common_regions.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let streets: Vec<_> = BufReader::new(
        File::open("./common_streets.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let street_ends: Vec<_> = BufReader::new(
        File::open("./common_street_endings.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let associations: Vec<_> = BufReader::new(
        File::open("./associations.txt")?
    ).lines().filter_map(|r| r.ok()).collect();

    let mut rng = thread_rng();

    let mut res = Vec::with_capacity(n);
    for _ in 0..n {
        let last_name = last_names.choose(&mut rng).unwrap().clone();
        let first_name = first_names.choose(&mut rng).unwrap().clone();

        let pr = PersonRecord {
            igra_number: format!("{:4}", rng.gen::<u32>() % 9000 + 1000),
            association: associations.choose(&mut rng).unwrap().clone(),
            birthdate: format!("19{:2}{:2}{:2}",
                               rng.gen::<u8>() % 100,
                               rng.gen::<u8>() % 12,
                               rng.gen::<u8>() % 28,
            ),
            ssn: format!("XXX-XX-{:4}", rng.gen::<u16>() % 10000),
            division: "".to_string(),
            legal_last: if rng.gen::<u8>() > 200 { last_names.choose(&mut rng).unwrap().clone() } else { last_name.clone() },
            legal_first: if rng.gen::<u8>() > 200 { first_names.choose(&mut rng).unwrap().clone() } else { first_name.clone() },
            id_checked: "Y".to_string(),
            sex: if rng.gen() { "M".to_string() } else { "F".to_string() },
            city: cities.choose(&mut rng).unwrap().clone(),
            state: regions.choose(&mut rng).unwrap().clone(),
            zip: format!("{:5}", rng.gen::<u16>() % 10000),
            home_phone: format!("({:3}){:3}-{:4}",
                                rng.gen::<u32>() % 900 + 100,
                                rng.gen::<u32>() % 900 + 100,
                                rng.gen::<u32>() % 9000 + 1000,
            ),
            cell_phone: format!("({:3}){:3}-{:4}",
                                rng.gen::<u32>() % 900 + 100,
                                rng.gen::<u32>() % 900 + 100,
                                rng.gen::<u32>() % 9000 + 1000,
            ),
            email: format!("{}@example.com", first_name.to_lowercase()),
            status: "0".to_string(),
            first_rodeo: "20230706".to_string(),
            last_updated: "20230706".to_string(),
            sort_date: "20230706".to_string(),
            ext_dollars: Default::default(),
            address: format!(
                "{num} {street} {end}",
                num = rng.gen::<u16>() % 50000 + 1,
                street = streets.choose(&mut rng).unwrap(),
                end = street_ends.choose(&mut rng).unwrap(),
            ),
            last_name,
            first_name,
        };

        res.push(pr);
    }


    Ok(res)
}

fn write_output(mut w: impl io::Write, report: &Report, people: &Vec<PersonRecord>) -> Result<(), Box<dyn Error>> {
    for v in &report.results {
        let c = &v.registration.contestant;

        println!("{} {}", c.first_name, c.last_name);
        if let Some(pr) = v.found {
            println!("\tFound: {:#?}", pr);
        } else {
            println!("\tMissing");
        }

        if c.note_to_director != "" {
            println!("\tNote to Director: {}", c.note_to_director);
        }

        if !v.partners.is_empty() {
            for (person_rec, partner_details) in v.partners.iter().filter_map(|p| report.relevant.get(p.igra_number).zip(Some(p))) {
                println!("\t\t{e:20} round {r} - {p}",
                         e = format!("{:?}", &partner_details.event),
                         r = &partner_details.round,
                         p = person_rec,
                );
            }
        }

        if v.issues.is_empty() {
            println!("\tNo issues!");
        } else {
            for Suggestion { problem: p, fix: f } in &v.issues {
                println!("\tProblem: {p:?} | Suggestion: {f:?}");
            }
        }
    }

    Ok(())
}


