use chrono::{NaiveDate};
use serde::{Serialize, Deserialize};
use crate::validation::RodeoEvent;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Registration {
    #[serde(alias = "rodeoContestantId")]
    #[serde(rename(serialize = "rodeoContestantId"))]
    pub id: u64,
    pub stalls: u64,
    pub contestant: Contestant,
    pub events: Vec<Event>,
    pub payment: Payment,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Payment {
    /// Total payment is in USD cents, e.g. $60 is represented as 6000.
    pub total: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Contestant {
    pub first_name: String,
    pub last_name: String,
    pub performance_name: String,
    pub dob: Date,
    pub age: u8,
    pub gender: String,
    // Should probably be a boolean.
    pub is_member: String,
    pub ssn: String,
    pub note_to_director: String,
    pub address: Address,
    pub association: Association,
}

impl Contestant {
    /// Get this contestant's last 4 SSN/SSI string
    /// formatted to match the old DOS system.
    pub fn dos_ssn(&self) -> String {
        format!("XXX-XX-{:04}", self.ssn)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Address {
    pub email: String,
    pub address_line_1: String,
    pub address_line_2: String,
    pub city: String,
    pub region: String,
    pub country: String,
    pub zip_code: String,
    pub cell_phone_no: String,
    pub home_phone_no: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Association {
    pub igra: String,
    pub member_assn: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(untagged, from = "SomeEventID")]
#[serde(rename_all(serialize = "camelCase"))]
pub enum EventID {
    Known(RodeoEvent),
    Unknown(u64),
}

// This (and the From implementation below) is a kludge
// to deserialize both ints and strings into a RodeoEvent (when possible)
// or to keep the Unknown id (when needed),
// and to reproduce that output afterward.
//
// It's a consequence of trying several other things that didn't work quite right,
// and ending up with some vestigial code that should be refactored,
// but since this isn't expected to live for long anyway, we'll keep what works.
#[derive(Deserialize)]
#[serde(untagged)]
enum SomeEventID {
    Known(RodeoEvent),
    Unknown(u64),
}

impl From<SomeEventID> for EventID {
    fn from(value: SomeEventID) -> Self {
        match value {
            SomeEventID::Known(re) => EventID::Known(re),
            SomeEventID::Unknown(id) => {
                match RodeoEvent::from_id(id) {
                    Some(re) => EventID::Known(re),
                    None => EventID::Unknown(id),
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Event {
    #[serde(alias = "eventId")]
    #[serde(rename(serialize = "eventId"))]
    pub id: EventID,
    pub partners: Vec<String>,
    pub round: u64,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl Date {
    /// Attempt to convert this to a NaiveDate.
    /// Returns None if that's not possible.
    pub fn naive_date(&self) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(self.year as i32, self.month as u32, self.day as u32)
    }

    /// Format this date to match the old DOS system format.
    pub fn dos(&self) -> String {
        format!("{year:04}{month:02}{day:02}",
                year = self.year, month = self.month, day = self.day)
    }
}

