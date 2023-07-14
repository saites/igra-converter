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
use std::path::PathBuf;
use std::sync::Arc;

use axum::response::IntoResponse;
use axum_server::tls_rustls::RustlsConfig;
use axum::{
    extract::{Host, State},
    handler::HandlerWithoutStateExt,
    http::{StatusCode, Uri, header},
    response::Redirect,
    routing::{post},
    Json, Router, BoxError,
};
use axum::http::HeaderValue;
use log;
use rand::prelude::*;
use tower_http::services::ServeDir;
use serde_json::json;

use crate::robin::{
    Address, Association, Contestant, Date, EventID, Payment, Registration,
};
use crate::validation::{
    EntryValidator, PersonRecord, Report, RodeoEvent, Suggestion,
};
use crate::xbase::{
    Header, TableReader,
};

type MyResult<T> = Result<T, Box<dyn Error>>;

#[tokio::main]
async fn main() -> MyResult<()> {
    env_logger::init();

    let mut args = env::args().skip(1);
    let command = args.next().expect("first arg should be the command");
    let personnel_path = args.next().expect("second arg should be the dbf file");

    log::debug!("Personnel File: {personnel_path}");
    let dbt = xbase::try_from_path(personnel_path)?;

    match command.as_str() {
        "validate" => {
            let target_path = args.next().ok_or("third arg should be a path")?;
            let people = validation::read_personnel(dbt)?;
            log::info!("Number of people in personnel database: {}", people.len());

            let reg = validation::read_reg(target_path)?;
            let report = do_validate(&people, &reg)?;
            let j = serde_json::to_string_pretty(&report)?;
            println!("{j}");
        }
        "gen_db" => {
            let target_path = args.next().ok_or("third arg should be a path")?;
            do_db_gen(dbt, target_path)?;
        }
        "gen_reg" => {
            let target_path = args.next().ok_or("third arg should be a path")?;
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
        match serde_json::to_string(&self) {
            Ok(j) => {
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))],
                    j
                ).into_response()
            }
            Err(e) => {
                log::error!("Report Serialization Error: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

async fn do_serve(people: Vec<PersonRecord>, port: u16) -> MyResult<()> {
    let state = AppState::new(people);
    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port);

    let config = if port == 443 {
        tokio::spawn(redirect_http_to_https(80, port));

        Some(
            RustlsConfig::from_pem_file(
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

/// Validates the collection of Registrations using the current database.
/// On success, returns a JSON response of results.
async fn handle_validate<'a>(
    State(state): State<AppState>,
    Json(payload): Json<Vec<Registration>>,
) -> impl IntoResponse
{
    let people = state.people.clone();
    let j = do_validate(&people, &payload)
        .and_then(|r| { serde_json::to_string(&r).map_err(|e| e.into()) })
        .map_err(|e| json!({"err": e.to_string()}).to_string());

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        j
    )
}

/// Generates random registration data and returns the result as a JSON object.
async fn handle_generate(
    State(state): State<AppState>,
    // Json(payload): Json<GenerationOptions>,
) -> Result<(StatusCode, Json<Vec<Registration>>), String>
{
    generate_fake_reg(&state.people.clone(), 10)
        .map(|r| (StatusCode::OK, Json(r)))
        .map_err(|err| {
            log::error!("{:?}", err);
            "An unexpected error occurred".to_string()
        })
}

/// Validates a collection of registrations against a collection of PersonRecords.
fn do_validate<'a>(
    people: &'a Vec<PersonRecord>,
    reg: &'a Vec<Registration>,
) -> MyResult<Report<'a>>
{
    log::info!("Number of entries JSON file: {}", reg.len());
    let validator = EntryValidator::new(&people);
    Ok(validator.validate_entries(&reg))
}

/// Generate a database of random people, using the given table as a template for fields.
fn do_db_gen<P, R>(dbt: TableReader<Header<R>>, target_path: P) -> MyResult<()>
    where P: AsRef<std::path::Path>,
          R: io::Read
{
    let mut tw = xbase::TableWriter::new(BufWriter::new(File::create(target_path)?))?;
    tw.add_fields(&dbt);
    let people = generate_fake_db()?;
    Ok(tw.write_records(&people)?)
}

/// Generates `n` random `Registration`s from the given collection of people.
fn generate_fake_reg(people: &Vec<PersonRecord>, n: usize) -> MyResult<Vec<Registration>> {
    let mut rng = thread_rng();
    let mut registrations = Vec::with_capacity(n);
    let today = chrono::Utc::now().naive_utc().date();

    // Non-team events.
    let event_names = vec![
        RodeoEvent::CalfRopingOnFoot,
        RodeoEvent::MountedBreakaway,
        RodeoEvent::PoleBending,
        RodeoEvent::BarrelRacing,
        RodeoEvent::FlagRacing,
        RodeoEvent::ChuteDogging,
        RodeoEvent::RanchSaddleBroncRiding,
        RodeoEvent::SteerRiding,
        RodeoEvent::BullRiding,
    ];
    let event_mod = event_names.len() - 2;

    // Determine who will register.
    let participants: Vec<_> = people.choose_multiple(&mut rng, n).collect();
    let n = participants.len(); // in case n > people.len()

    fn add_event(rng: &mut ThreadRng, events: &mut Vec<robin::Event>, rid: RodeoEvent, partners: Vec<String>) {
        // With high probability, register for this event twice.
        if rng.gen::<u8>() > 15 {
            events.push(robin::Event {
                id: EventID::Known(rid),
                partners: partners.clone(),
                round: 1,
            });
            events.push(robin::Event {
                id: EventID::Known(rid),
                partners,
                round: 2,
            });
        } else {
            // Otherwise randomly choose which round to register for.
            events.push(robin::Event {
                id: EventID::Known(rid),
                partners,
                round: rng.gen_range(1..=2),
            });
        }
    }

    // Start with single-person events and create registration entries.
    for r in &participants {
        // Start by deciding how many/which events this person will register for.
        let n_events = rng.gen_range(0..=event_mod) + 2;
        let mut events = Vec::<>::with_capacity(n_events);
        let chosen_events = event_names.choose_multiple(&mut rng, n_events);

        for eid in chosen_events {
            add_event(&mut rng, &mut events, *eid, vec![]);
        }

        let payment = Payment { total: (events.len() * 30) as u64 };

        let perf_name = format!("{} {}", r.first_name, r.last_name);

        log::debug!("{:?}", r.birthdate);
        let dob_y = r.birthdate[0..4].parse::<u16>().unwrap_or(1970);
        let dob_m = r.birthdate[4..6].parse::<u8>().unwrap_or(1);
        let dob_d = r.birthdate[6..8].parse::<u8>().unwrap_or(1);
        let dob = Date { year: dob_y, month: dob_m, day: dob_d };

        fn format_phone(phone_num: &str) -> String {
            format!("{}{}{}", &phone_num[1..4], &phone_num[5..8], &phone_num[9..13])
        }

        let pr = Registration {
            id: rng.gen(),
            stalls: "".to_string(),
            contestant: Contestant {
                first_name: r.legal_first.clone(),
                last_name: r.legal_last.clone(),
                performance_name: perf_name,
                age: dob.naive_date().and_then(|d| today.years_since(d)).unwrap_or(0) as u8,
                dob,
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
                    cell_phone_no: format_phone(&r.cell_phone),
                    home_phone_no: format_phone(&r.home_phone),
                },
                association: Association {
                    igra: r.igra_number.clone(),
                    member_assn: r.association.clone(),
                },
            },
            events,
            payment,
        };

        registrations.push(pr);
    }

    // Now handle partner events.
    fn partner_name(rng: &mut ThreadRng, p: &PersonRecord) -> String {
        // Decide randomly among possible ways of writing the partner's name.
        // Each subsequent style is half as likely as the one before it.
        if rng.gen() {
            p.igra_number.clone()
        } else if rng.gen() {
            format!("{} {}", p.first_name, p.last_name)
        } else if rng.gen() {
            format!("{} {}", p.legal_first, p.legal_last)
        } else if rng.gen() {
            format!("{} {} | {}", p.first_name, p.last_name, p.igra_number)
        } else if rng.gen() {
            format!("{} {} | {}", p.legal_first, p.legal_last, p.igra_number)
        } else {
            format!("{} | {} {}", p.igra_number, p.first_name, p.last_name)
        }
    }

    // For most team events, we can pair up people randomly.
    [
        RodeoEvent::TeamRopingHeader,
        RodeoEvent::TeamRopingHeeler,
        RodeoEvent::GoatDressing,
        RodeoEvent::SteerDecorating,
    ].iter().for_each(|eid| {
        let n_event = 2 * rng.gen_range(0..=n / 2);
        let mut in_event = participants.iter().zip(registrations.iter_mut())
            .choose_multiple(&mut rng, n_event);
        let mut in_event = in_event.iter_mut();

        while let Some((p1, ref mut r1)) = in_event.next() {
            let (p2, ref mut r2) = in_event.next().unwrap();
            let p1_partners = vec![partner_name(&mut rng, p2)];
            let p2_partners = vec![partner_name(&mut rng, p1)];
            add_event(&mut rng, &mut r1.events, *eid, p1_partners);
            add_event(&mut rng, &mut r2.events, *eid, p2_partners);
        }
    });

    // Drag teams have restrictions on team composition.
    let (mut cowboys, mut cowgirls): (Vec<_>, Vec<_>) = participants
        .into_iter()
        .zip(registrations.iter_mut())
        .partition(|(p, _)| p.sex == "M");
    let mut cowboys = cowboys.iter_mut();
    let mut cowgirls = cowgirls.iter_mut();

    for _ in 0..rng.gen_range(0..=(n / 3)) {
        let drag = if rng.gen() { cowboys.next() } else { cowgirls.next() };
        match (cowboys.next(), cowgirls.next(), drag) {
            (Some(cb), Some(cg), Some(d)) => {
                let cbp = vec![partner_name(&mut rng, cg.0), partner_name(&mut rng, d.0)];
                let cgp = vec![partner_name(&mut rng, cb.0), partner_name(&mut rng, d.0)];
                let dp = vec![partner_name(&mut rng, cb.0), partner_name(&mut rng, cg.0)];
                add_event(&mut rng, &mut cb.1.events, RodeoEvent::WildDragRace, cbp);
                add_event(&mut rng, &mut cg.1.events, RodeoEvent::WildDragRace, cgp);
                add_event(&mut rng, &mut d.1.events, RodeoEvent::WildDragRace, dp);
            }
            _ => break,
        }
    }

    Ok(registrations)
}


fn generate_fake_db() -> Result<Vec<PersonRecord>, Box<dyn Error>> {
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

    fn to_division(association: &str) -> String {
        match association {
            "CRGRA" => 1,
            "DSRA" => 3,
            "AGRA" => 2,
            "GSGRA" => 1,
            "CGRA" => 2,
            "ASGRA" => 4,
            "MIGRA" => 4,
            "NSGRA" => 4,
            "MGRA" => 3,
            "NMGRA" => 2,
            "NGRA" => 1,
            "GPRA" => 3,
            "TGRA" => 3,
            "RRRA" => 3,
            "UGRA" => 2,
            _ => 0,
        }.to_string()
    }

    fn phone(rng: &mut ThreadRng) -> String {
        format!("({area:03}){prefix:03}-{number:04}",
                area = rng.gen_range(100..=999),
                prefix = rng.gen_range(100..=999),
                number = rng.gen_range(0..=9999),
        )
    }

    // Generate 8,000 records with IGRA numbers 1000 to 8999,
    // leaving 0000..=0999 and 9000..=9999 available.
    let mut res = Vec::with_capacity(8000);
    for igra_number in 1000..10000 {
        let last_name = last_names.choose(&mut rng).unwrap().clone();
        let first_name = first_names.choose(&mut rng).unwrap().clone();
        let association = associations.choose(&mut rng).unwrap().clone();
        let division = to_division(&association);

        let pr = PersonRecord {
            igra_number: format!("{:4}", igra_number),
            birthdate: format!("{y:04}{m:02}{d:02}",
                               y = rng.gen_range(1900..=2004),
                               m = rng.gen_range(1..=12),
                               d = rng.gen_range(1..=28),
            ),
            ssn: format!("XXX-XX-{:04}", rng.gen_range(0..=9999)),
            // With high probability, use the same legal and performance names.
            // The magic numbers come from the distribution in the existing database.
            legal_last: if rng.gen::<u8>() < 223 { last_name.clone() } else { last_names.choose(&mut rng).unwrap().clone() },
            legal_first: if rng.gen::<u8>() < 210 { first_name.clone() } else { first_names.choose(&mut rng).unwrap().clone() },
            id_checked: if rng.gen::<u8>() < 179 { "Y".to_string() } else { "".to_string() },
            // The actual DB distribution is about 2:1, but we'll be more evenly split.
            sex: if rng.gen() { "M".to_string() } else { "F".to_string() },
            home_phone: phone(&mut rng),
            cell_phone: phone(&mut rng),
            email: format!("{}@example.com", first_name.to_lowercase()),
            status: "0".to_string(),
            first_rodeo: "20230706".to_string(),
            last_updated: "20230706".to_string(),
            sort_date: "20230706".to_string(),
            ext_dollars: Default::default(),
            address: format!(
                "{num} {street} {end}",
                num = rng.gen_range(10..=99999),
                street = streets.choose(&mut rng).unwrap(),
                end = street_ends.choose(&mut rng).unwrap(),
            ),
            city: cities.choose(&mut rng).unwrap().clone(),
            state: regions.choose(&mut rng).unwrap().clone(),
            zip: format!("{:05}", rng.gen_range(10000..=99999)),
            association,
            division,
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


