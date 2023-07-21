mod xbase;
mod bktree;
mod robin;
mod validation;
mod api;

use std::env;
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
    http::{HeaderValue, StatusCode, Uri, header},
    response::Redirect,
    routing::{post},
    Json, Router, BoxError,
};
use axum_extra::extract::WithRejection;

use log;
use rand::prelude::*;
use serde::Deserialize;
use tower_http::services::ServeDir;
use serde_json::json;

use crate::api::ApiError;
use crate::robin::{
    Address, Association, Contestant, Date, EventID, Payment, Registration,
};
use crate::validation::{
    EntryValidator, PersonRecord, Report, RodeoEvent,
};

type MyResult<T> = Result<T, Box<dyn Error>>;

#[tokio::main]
async fn main() -> MyResult<()> {
    env_logger::init();

    let mut args = env::args().skip(1);
    let command = args.next().expect("first arg should be the command");
    let personnel_path = args.next().expect("second arg should be the dbf file");
    log::debug!("Personnel File: {personnel_path}");

    match command.as_str() {
        "gen_db" => {
            do_db_gen(personnel_path)?;
        }
        "validate" => {
            let dbt = xbase::try_from_path(personnel_path)?;
            let target_path = args.next().ok_or("third arg should be a path")?;
            let people = validation::read_personnel(dbt)?;
            log::info!("Number of people in personnel database: {}", people.len());

            let reg = validation::read_reg(target_path)?;
            let report = do_validate(&people, &reg)?;
            let j = serde_json::to_string_pretty(&report)?;
            println!("{j}");
        }
        "search" => {
            let dbt = xbase::try_from_path(personnel_path)?;
            let person = args.next().ok_or("third arg should be a name")?;
            let legal_first = args.next().unwrap_or("".to_string());
            let legal_last = args.next().unwrap_or("".to_string());

            let people = validation::read_personnel(dbt)?;
            log::info!("Number of people in personnel database: {}", people.len());
            let validator = EntryValidator::new(&people);

            let (igra, name) = validation::split_partner(&person);
            let (perfect, matches) = validator.find_person(
                igra, &legal_first, &legal_last, &name);

            println!("IGRA #: {igra:?} | Name: {name} | Perfect Match: {perfect} | Num matches: {count}",
                     count = matches.len(),
                     name = if legal_first.is_empty() && legal_last.is_empty() {
                         name.into()
                     } else {
                         format!("'{legal_first} {legal_last}' aka {name}")
                     }
            );
            for p in matches {
                println!("\t{p}")
            }
        }
        "gen_reg" => {
            let dbt = xbase::try_from_path(personnel_path)?;
            let target_path = args.next().ok_or("third arg should be a path")?;
            let people = validation::read_personnel(dbt)?;
            let fake_regs = generate_fake_reg(&people, 10)?;
            let j = serde_json::to_string_pretty(&fake_regs)?;
            write!(BufWriter::new(File::create(target_path)?), "{j}")?;
        }
        "serve" => {
            let dbt = xbase::try_from_path(personnel_path)?;
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
    WithRejection(Json(payload), _): WithRejection<Json<Vec<Registration>>, ApiError>,
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

fn default_num_people() -> u8 { 10 }

#[derive(Copy, Clone, Debug, Deserialize)]
struct GenerationOptions {
    #[serde(default = "default_num_people")]
    num_people: u8,
}

/// Generates random registration data and returns the result as a JSON object.
async fn handle_generate(
    State(state): State<AppState>,
    Json(payload): Json<GenerationOptions>,
) -> Result<(StatusCode, Json<Vec<Registration>>), ApiError>
{
    if !matches!(payload.num_people, 2..=200) {
        return Err(ApiError::InvalidNumberOfPeople { amount: payload.num_people, min: 2, max: 100 });
    }

    generate_fake_reg(&state.people.clone(), payload.num_people as usize)
        .map(|r| (StatusCode::OK, Json(r)))
        .map_err(|err| {
            log::error!("{:?}", err);
            ApiError::Unexpected
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
fn do_db_gen<P>(target_path: P) -> MyResult<()>
    where P: AsRef<std::path::Path>,
{
    let tw = xbase::TableWriter::new(
            BufWriter::new(File::create(target_path)?))?;
    let people = generate_fake_db()?;
    Ok(tw.write_records(&people)?)
}

/// Generates `n` random `Registration`s from the given collection of people.
/// Barring bugs in the implementation, this returns a valid collection of registrations.
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

    // This decides randomly among possible ways of writing the partner's name.
    // Each subsequent style is half as likely as the one before it.
    fn partner_name(rng: &mut ThreadRng, p: &PersonRecord) -> String {
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

    // This helper registers a (possible 1-person) team of people,
    // either for a single go-round or for both go-rounds,
    // ensuring that all teammates mutually register with one another.
    fn register(rng: &mut ThreadRng, rid: RodeoEvent, who: &mut [(&PersonRecord, &mut Registration)]) {
        // With high probability, register for this event twice.
        // We'll also pick a round to register for if we decide to only register for one.
        let twice = rng.gen::<u8>() > 15;
        let round = rng.gen_range(1..=2);

        // We need to generate the partner names before taking the mutable borrow on who.
        let partner_names: Vec<_> = who.iter().map(|(p, _)| {
            who.iter().filter_map(|(p2, _)| {
                if p == p2 { None } else { Some(partner_name(rng, p2)) }
            }).collect::<Vec<_>>()
        }).collect();

        let id = EventID::Known(rid);

        // Register each person with their partners.
        for ((_, ref mut r), partners) in who.into_iter().zip(partner_names) {
            if twice {
                r.events.push(robin::Event { id, partners: partners.clone(), round: 1 });
                r.events.push(robin::Event { id, partners, round: 2 });
                r.payment.total += 60;
            } else {
                r.events.push(robin::Event { id, partners, round });
                r.payment.total += 30;
            }
        }
    }

    fn format_phone(phone_num: &str) -> String {
        format!("{}{}{}", &phone_num[1..4], &phone_num[5..8], &phone_num[9..13])
    }

    // Start with single-person events and create registration entries.
    for p in &participants {
        let perf_name = format!("{} {}", p.first_name, p.last_name);
        let dob_y = p.birthdate[0..4].parse::<u16>().unwrap_or(1970);
        let dob_m = p.birthdate[4..6].parse::<u8>().unwrap_or(1);
        let dob_d = p.birthdate[6..8].parse::<u8>().unwrap_or(1);
        let dob = Date { year: dob_y, month: dob_m, day: dob_d };

        let n_events = rng.gen_range(0..=event_mod) + 2;
        let events = Vec::<>::with_capacity(n_events);

        // The database wasn't designed with non-US address in mind.
        let (region, country) = if p.state != "FC" {
            if validation::CANADIAN_REGIONS.contains(&p.state) {
                (p.region().map_or(p.state.clone(), |re| re.to_string()),
                "Canada".to_string())
            } else {
                (p.region().map_or(p.state.clone(), |re| re.to_string()),
                    "United States".to_string())
            }
        } else {
            ("Sonora".to_string(), "Mexico".to_string())
        };

        let mut r = Registration {
            id: rng.gen(),
            stalls: "".to_string(),
            contestant: Contestant {
                first_name: p.legal_first.clone(),
                last_name: p.legal_last.clone(),
                performance_name: perf_name,
                age: dob.naive_date().and_then(|d| today.years_since(d)).unwrap_or(0) as u8,
                dob,
                gender: if p.sex == "M" { "Cowboys".to_string() } else { "Cowgirls".to_string() },
                is_member: "yes".to_string(),
                ssn: p.ssn[7..].to_string(),
                note_to_director: "".to_string(),
                address: Address {
                    email: p.email.clone(),
                    address_line_1: p.address.clone(),
                    address_line_2: "".to_string(),
                    city: p.city.clone(),
                    region,
                    country,
                    zip_code: p.zip.clone(),
                    cell_phone_no: format_phone(&p.cell_phone),
                    home_phone_no: format_phone(&p.home_phone),
                },
                association: Association {
                    igra: p.igra_number.clone(),
                    member_assn: p.association.clone(),
                },
            },
            events,
            payment: Payment { total: 0 },
        };

        for eid in event_names.choose_multiple(&mut rng, n_events) {
            register(&mut rng, *eid, &mut [(p, &mut r)]);
        }

        registrations.push(r);
    }

    // Now handle partner events. For most team events, we can pair up people randomly.
    [
        RodeoEvent::TeamRopingHeader,
        RodeoEvent::TeamRopingHeeler,
        RodeoEvent::GoatDressing,
        RodeoEvent::SteerDecorating,
    ].iter().for_each(|eid| {
        // First choose how many/which people will participate.
        let n_event = 2 * rng.gen_range(0..=n / 2);
        let mut in_event = participants.iter().zip(registrations.iter_mut())
            .choose_multiple(&mut rng, n_event);
        let mut in_event = in_event.iter_mut();

        // While we can grab a pair of participants, pair them up in the event.
        while let Some((p1, ref mut r1)) = in_event.next() {
            let (p2, ref mut r2) = in_event.next().expect("n_events is even, so we should have pairs");
            register(&mut rng, *eid, &mut [(p1, r1), (p2, r2)]);
        }
    });

    // Drag teams have restrictions on team composition.
    let (mut cowboys, mut cowgirls): (Vec<_>, Vec<_>) = participants
        .into_iter()
        .zip(registrations.iter_mut())
        .partition(|(p, _)| p.sex == "M");
    let mut cowboys = cowboys.iter_mut();
    let mut cowgirls = cowgirls.iter_mut();

    // Build up to n/3 drag teams by randomly choosing a cowboy, cowgirl, and drag.
    // Note that we can end up with fewer teams then chosen by the RNG,
    // if we happen to have a high imbalance of cowboys:cowgirls.
    for _ in 0..rng.gen_range(0..=(n / 3)) {
        let drag = if rng.gen() { cowboys.next() } else { cowgirls.next() };
        match (cowboys.next(), cowgirls.next(), drag) {
            (Some(cb), Some(cg), Some(d)) => {
                register(&mut rng, RodeoEvent::WildDragRace, &mut [(cb.0, cb.1), (cg.0, cg.1), (d.0, d.1)]);
            }
            _ => break,
        }
    }

    Ok(registrations)
}


fn generate_fake_db() -> Result<Vec<PersonRecord>, Box<dyn Error>> {
    // TODO: ensure these don't exceed field widths
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
