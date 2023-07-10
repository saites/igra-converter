mod xbase;
mod bktree;
mod robin;
mod validation;

use std::{env, io};
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::path::PathBuf;

use axum::response::IntoResponse;
use axum_server::tls_rustls::RustlsConfig;
use axum::{
    extract::{Host, State},
    handler::HandlerWithoutStateExt,
    http::{StatusCode, Uri, header},
    response::Redirect,
    routing::{get, post},
    Json, Router, BoxError,
};
use log;
use rand::prelude::*;
use tower_http::services::ServeDir;
use xbase::Header;
use serde_json::json;

use crate::validation::{
    Report, Suggestion, RodeoEvent, EntryValidator, PersonRecord,
};
use crate::xbase::{TableReader};
use crate::robin::{
    Contestant, Registration, EventID, Payment,
    Date, Address, Association
};

type MyResult<T> = Result<T, Box<dyn Error>>;

#[tokio::main]
async fn main() {
    process_command().await.unwrap();
}

async fn process_command() -> MyResult<()> {
    env_logger::init();

    let mut args = env::args().skip(1);
    let command = args.next().expect("first arg should be the command");
    let personnel_path = args.next().expect("second arg should be the dbf file");

    log::debug!("Personnel File: {personnel_path}");
    let dbt = xbase::try_from_path(personnel_path)?;
    

    match command.as_str() {
        "validate" => { 
            let target_path = args.next().expect("thrid arg should be a path");
            let people = validation::read_personnel(dbt)?;
            log::info!("Number of people in personnel database: {}", people.len());

            let reg = validation::read_reg(target_path)?;
            let report = do_validate(&people, &reg)?;
            let j = serde_json::to_string_pretty(&report)?;
            write!(BufWriter::new(
                    File::create("./web/validation_output.json")?), "{j}")?;
        }
        "gen_db" => { 
            let target_path = args.next().expect("thrid arg should be a path");
            do_db_gen(dbt, target_path)?; 
        }
        "gen_reg" => { 
            let target_path = args.next().expect("thrid arg should be a path");
            let people = validation::read_personnel(dbt)?;
            let fake_regs = generate_fake_reg(&people, 10)?;
            let j = serde_json::to_string_pretty(&fake_regs)?;
            write!(BufWriter::new(File::create(target_path)?), "{j}")?;
        }
        "serve" => {
            let people = validation::read_personnel(dbt)?;
            let port = args.next()
                .and_then(|var| var.parse::<u16>().ok())
                .unwrap_or(8080 as u16);
            do_serve(people, port).await?;
        }
        _ => { return Err("Unknown command".into()); }
    }

    Ok(())
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub people: Arc<Vec<PersonRecord>>,
}

impl AppState {
    fn new(people: Vec<PersonRecord>) -> Self {
        AppState {
            people: Arc::new(people),
        }
    }
}

impl<'a> IntoResponse for Report<'a> {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::OK, Json(&self)).into_response()
    }
}

async fn do_serve(
    people: Vec<PersonRecord>, 
    port: u16,
) -> MyResult<()>
{
    let state = AppState::new(people);
    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port);

    let config = if port == 443 {
      tokio::spawn(redirect_http_to_https(80, port));

        Some(
            RustlsConfig::from_pem_file(
//                 PathBuf::from("./certs/domain.cert.pem"),
                PathBuf::from("./certs/intermediate.cert.pem"),
                PathBuf::from("./certs/private.key.pem"),
            )
            .await
            .unwrap()
        )
    } else { None };

    let routes = Router::new()
        .nest_service("/", ServeDir::new("./web"))
        .route("/validate", post(handle_validate))
        .route("/generate", post(handle_generate))
        .with_state(state)
        .fallback(handle_404);

    log::info!("Running on {socket}.");
    if let Some(config) = config {
        axum_server::bind_rustls(socket, config)
            .serve(routes.into_make_service())
            .await
            .expect("server failed to start");
    } else {
        axum::Server::bind(&socket)
            .serve(routes.into_make_service())
            .await
            .expect("server failed to start");
    }

    Ok(())
}

async fn redirect_http_to_https(http_port: u16, https_port: u16) {
    fn make_https(host: String, uri: Uri, http_port: u16, https_port: u16) -> Result<Uri, BoxError> {
        let mut parts = uri.into_parts();

        parts.scheme = Some(axum::http::uri::Scheme::HTTPS);
        if parts.path_and_query.is_none() {
            parts.path_and_query = Some("/".parse().unwrap());
        }

        let https_host = host.replace(&http_port.to_string(), &https_port.to_string());
        parts.authority = Some(https_host.parse()?);

        Ok(Uri::from_parts(parts)?)
    }

    let redirect = move |Host(host): Host, uri: Uri| async move {
        match make_https(host, uri, http_port, https_port) {
            Ok(uri) => Ok(Redirect::permanent(&uri.to_string())),
            Err(error) => {
                log::warn!("failed to convert URI to HTTPS: {:?}", error);
                Err(StatusCode::BAD_REQUEST)
            }
        }
    };

    // let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    // let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), http_port);

    log::debug!("listening on {}", &socket);
    axum::Server::bind(&socket)
        .serve(redirect.into_make_service())
        .await
        .expect("server failed to start");
}

async fn handle_404() -> &'static str {
    "Page not found"
}

async fn handle_validate<'a>(
    State(state): State<AppState>,
    Json(payload): Json<Vec<Registration>>,
) -> impl IntoResponse
{
    let people = state.people.clone();

    let j = match do_validate(&people, &payload) {
        Ok(r) => {
            match serde_json::to_string(&r) {
                Ok(j) => j,
                Err(err) => json!({"err": err.to_string()}).to_string(),
            }
        },
        Err(err) => json!({"err": err.to_string()}).to_string(),
    };

    (
        StatusCode::OK,
        [ (header::CONTENT_TYPE, "application/json"), ],
        j
    )
}

async fn handle_generate(
    State(state): State<AppState>,
    // Json(payload): Json<Vec<Registration>,
) -> Result<(StatusCode, Json<Vec<Registration>>), String> 
{
    generate_fake_reg(&state.people.clone(), 10)
        .map(|r| (StatusCode::OK, Json(r)))
        .map_err(|err| {
            log::error!("{:?}", err);
            "An unexpected error occurred".to_string()
        })
}

fn do_validate<'a>(people: &'a Vec<PersonRecord>, reg: &'a Vec<Registration>) 
    -> MyResult<Report<'a>>
{
    log::info!("Number of entries JSON file: {}", reg.len());
    let validator = EntryValidator::new(&people);
    Ok(validator.validate_entries(&reg))
}

fn do_db_gen<P, R>(dbt: TableReader<Header<R>>, target_path: P) -> MyResult<()> 
    where P: AsRef<std::path::Path>,
        R: io::Read
{
    let mut tw = xbase::TableWriter::new(BufWriter::new(File::create(target_path)?))?;
    tw.add_fields(&dbt);
    let people = generate_fake_db(10000)?;
    Ok(tw.write_records(&people)?)
}

fn generate_fake_reg(people: &Vec<PersonRecord>, n: usize) 
    -> Result<Vec<Registration>, Box<dyn Error>> {
    let mut rng = thread_rng();
    let mut res = Vec::with_capacity(n);

    let event_names = vec![
        RodeoEvent::CalfRopingOnFoot ,
        RodeoEvent::MountedBreakaway ,
        RodeoEvent::TeamRopingHeader ,
        RodeoEvent::TeamRopingHeeler ,
        RodeoEvent::PoleBending ,
        RodeoEvent::BarrelRacing ,
        RodeoEvent::FlagRacing ,
        RodeoEvent::ChuteDogging ,
        RodeoEvent::RanchSaddleBroncRiding ,
        RodeoEvent::SteerRiding ,
        RodeoEvent::BullRiding ,
        RodeoEvent::GoatDressing ,
        RodeoEvent::SteerDecorating ,
        RodeoEvent::WildDragRace ,
    ];
    
    // TODO: mutate some records so they are bad  

    let participants: Vec<_> = people
        .choose_multiple(&mut rng, n).collect();

    // todo: handle partners better

    for r in &participants {
        let mut events = Vec::<>::with_capacity(5); 
        
        let n_events = (rng.gen::<usize>() % 10) + 2;
        let chosen_events: Vec<_> = event_names
            .choose_multiple(&mut rng, n_events)
            .collect();

        for eid in chosen_events {
            let chosen_partners = participants
                .choose_multiple(&mut rng, eid.num_partners() as usize);

            // todo: handle choosing yourself
            let mut partners = vec![];
            for p in chosen_partners {
                if rng.gen() {
                    partners.push(p.igra_number.clone())
                } else if rng.gen() {
                    partners.push( format!("{} {}", p.first_name, p.last_name))
                } else if rng.gen() {
                    partners.push( format!("{} {}", p.legal_first, p.legal_last))
                } else if rng.gen() {
                    partners.push( format!("{} {} | {}", p.first_name, p.last_name,
                                           p.igra_number))
                } else if rng.gen() {
                    partners.push( format!("{} {} | {}", p.legal_first, p.legal_last, 
                                           p.igra_number))
                } else {
                    partners.push( format!("{} | {} {}", 
                                           p.igra_number,
                                           p.first_name, p.last_name
                                           ))
                }
            }
            
            let event = robin::Event{
                id: EventID::Known(*eid),  
                partners,
                round: rng.gen::<u64>() % 2 + 1,
            };

            events.push(event);
        }

        let payment = Payment { total: (events.len() * 30) as u64 };

        let perf_name = format!(
            "{} {}", r.first_name, r.last_name
            );

        log::debug!("{:?}", r.birthdate);
        let dob_y = r.birthdate[0..4].parse::<u16>().unwrap_or(1900);
        let dob_m = r.birthdate[4..6].parse::<u8>().expect("bday month should be valid");
        let dob_d = r.birthdate[6..8].parse::<u8>().expect("bday day should be valid");

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
                is_member: "yes".to_string(),
                ssn: r.ssn[7..].to_string(), 
                note_to_director: "".to_string(),
                address: Address {
                    email: r.email.clone(),
                    address_line_1: r.address.clone(), 
                    address_line_2: "".to_string(),
                    city: r.city.clone(),
                    region: r.region().map_or(
                        r.state.clone(),
                        |re| re.to_string()),
                    country: "United States".to_string(),
                    zip_code: r.zip.clone(), 
                    cell_phone_no: format!("{}{}{}", 
                                           &r.cell_phone[1..4],
                                           &r.cell_phone[5..8],
                                           &r.cell_phone[9..13],
                                           ), 
                    home_phone_no: format!("{}{}{}", 
                                           &r.home_phone[1..4],
                                           &r.home_phone[5..8],
                                           &r.home_phone[9..13],
                                           ), 
                },
                association: Association { 
                    igra: r.igra_number.clone(), 
                    member_assn: r.association.clone(), 
                },
            },
            events,
            payment,
        };

        res.push(pr);
    }

    Ok(res)
}


fn generate_fake_db(n: usize) -> Result<Vec<PersonRecord>, Box<dyn Error>> {
    let first_names: Vec<_> = BufReader::new(
        File::open("./data/common_first_names.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let last_names: Vec<_> = BufReader::new(
        File::open("./data/common_last_names.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let cities: Vec<_> = BufReader::new(
        File::open("./data/common_cities.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let regions: Vec<_> = BufReader::new(
        File::open("./data/common_regions.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let streets: Vec<_> = BufReader::new(
        File::open("./data/common_streets.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let street_ends: Vec<_> = BufReader::new(
        File::open("./data/common_street_endings.txt")?
    ).lines().filter_map(|r| r.ok()).collect();
    let associations: Vec<_> = BufReader::new(
        File::open("./data/associations.txt")?
    ).lines().filter_map(|r| r.ok()).collect();

    let mut rng = thread_rng();


    let mut res = Vec::with_capacity(n);
    for igra_number in 1000..10000 {
        let last_name = last_names.choose(&mut rng).unwrap().clone();
        let first_name = first_names.choose(&mut rng).unwrap().clone();

        let pr = PersonRecord {
            igra_number: format!("{:4}", igra_number),
            association: associations.choose(&mut rng).unwrap().clone(),
            birthdate: format!("19{:02}{:02}{:02}",
                               rng.gen::<u8>() % 100,
                               (rng.gen::<u8>() % 12) + 1,
                               (rng.gen::<u8>() % 28) + 1,
            ),
            ssn: format!("XXX-XX-{:04}", rng.gen::<u16>() % 10000),
            division: "".to_string(),
            legal_last: if rng.gen::<u8>() > 200 { last_names.choose(&mut rng).unwrap().clone() } else { last_name.clone() },
            legal_first: if rng.gen::<u8>() > 200 { first_names.choose(&mut rng).unwrap().clone() } else { first_name.clone() },
            id_checked: "Y".to_string(),
            sex: if rng.gen() { "M".to_string() } else { "F".to_string() },
            city: cities.choose(&mut rng).unwrap().clone(),
            state: regions.choose(&mut rng).unwrap().clone(),
            zip: format!("{:5}", rng.gen::<u16>() % 10000),
            home_phone: format!("({:3}){:3}-{:4}",
                                (rng.gen::<u32>() % 900) + 100,
                                (rng.gen::<u32>() % 900) + 100,
                                (rng.gen::<u32>() % 9000) + 1000,
            ),
            cell_phone: format!("({:3}){:3}-{:4}",
                                (rng.gen::<u32>() % 900) + 100,
                                (rng.gen::<u32>() % 900) + 100,
                                (rng.gen::<u32>() % 9000) + 1000,
            ),
            email: format!("{}@example.com", first_name.to_lowercase()),
            status: "0".to_string(),
            first_rodeo: "20230706".to_string(),
            last_updated: "20230706".to_string(),
            sort_date: "20230706".to_string(),
            ext_dollars: Default::default(),
            address: format!(
                "{num} {street} {end}",
                num = (rng.gen::<u16>() % 50000) + 1,
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

fn write_output(mut w: impl io::Write, report: &Report, people: &Vec<PersonRecord>) 
    -> Result<(), Box<dyn Error>> 
{
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
            for (person_rec, partner_details) in v.partners.iter()
                .filter_map(|p| report.relevant.get(p.igra_number).zip(Some(p))) {
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


