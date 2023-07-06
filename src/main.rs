mod xbase;
mod bktree;
mod robin;
mod validation;

use std::{env, io};
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::io::Write;

use log;
use serde::Serialize;
use validation::{EntryValidator, PersonRecord, Processed};
use crate::robin::Registration;
use crate::validation::{Fix, Problem, Report, Suggestion};


fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let mut args = env::args().skip(1);
    let personnel_path = args.next().expect("first arg should be the dbf file");
    let reg_path = args.next().expect("second arg should be a JSON file");

    // let pp = env::current_dir().unwrap().as_path().join(&personnel_path).canonicalize().unwrap();
    log::debug!("Personnel File: {personnel_path}, Registration File: {reg_path}");

    let reg = validation::read_reg(reg_path)?;
    log::debug!("Registration File:\n{:#?}", reg);

    let dbt = xbase::try_from_path(personnel_path)?;
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
                    e=format!("{:?}", &partner_details.event),
                    r=&partner_details.round,
                    p=person_rec,
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


