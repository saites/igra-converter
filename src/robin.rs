use chrono::{NaiveDate};
use serde::{Deserialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
pub struct Registration {
    id: u64,
    pub stalls: String,  // Should probably be an integer.
    pub contestant: Contestant,
    pub events: Vec<Event>,
    pub payment: Payment,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize="camelCase"))]
pub struct Payment {
    pub total: u64, // Should this be integral?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize="camelCase"))]
pub struct Contestant {
    pub first_name: String,
    pub last_name: String,
    pub performance_name: String,
    pub dob: Date,
    pub age: u8,
    pub gender: String,
    pub is_member: String, // Should probably be a boolean.
    pub ssn: String,
    pub note_to_director: String,
    pub address: Address,
    pub association: Association,
}

impl Contestant {
    /// Get this contestant's last 4 SSN/SSI string
    /// formatted to match the old DOS system.
    pub fn dos_ssn(&self) -> String {
        format!("XXX-XX-{}", self.ssn)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize="camelCase"))]
pub enum CompetitionCategory {
    Cowboys,
    Cowgirls,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize="camelCase"))]
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

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize="camelCase"))]
pub struct Association {
    pub igra: String,
    pub member_assn: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize="camelCase"))]
pub struct Event {
    #[serde(rename(deserialize="rodeoEventRelId"))]
    pub id: u64,
    pub partners: Vec<String>,
    pub round: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all(deserialize="camelCase"))]
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

