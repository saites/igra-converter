use std::fmt::{Display, Formatter};
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::num::{ParseFloatError, ParseIntError};
use std::path::Path;
use std::str::FromStr;

use log;

use binary_layout::prelude::*;
use chrono::{NaiveDate};
use thiserror::Error;
use crate::xbase::DBaseErrorKind::{InvalidLastUpdated, UnknownFieldType, UnknownLogicalValue};


// 3 bytes representing YYMMDD, where YY is years since 1900.
define_layout!(yymmdd, LittleEndian, {
    year: u8,
    month: u8,
    day: u8,
});

define_layout!(dbase_header, LittleEndian, {
    flags: u8, // bits 0-2= version, 3= has DOS memo file, 4-6= has SQL table, 7= any memo file
    last_updated: yymmdd::NestedView,
    n_records: u32,
    n_header_bytes: u16,
    _reserved1: [u8; 2],
    incomplete_transaction: u8,
    encrypted: u8,
    _reserved2: [u8; 12],
    is_production: u8,
    language_driver_id: u8,
    // _reserved3: [u8; 2],
});

define_layout!(field_descriptor, LittleEndian, {
    name: [u8; 11],
    f_type: u8,
    _reserved1: [u8; 4],
    length: u8,
    decimal_count: u8,
    work_area_id: u16,
    example: u8,
    _reserved2: [u8; 10],
    is_production: u8,
});

// A Clipper Index file (.NTX) is somewhere between a modified B+ tree and a skip list.
// It's made up of a series of pages. Each page is 1024 bytes.
//
// The first page is a header with the address of the root page, description of the key size,
// and the string expression describing the key the index is built on.
//
// Each following page starts with a header indicating the number of used entries on the page
// followed by an array of offsets (relative the page start) pointing to each child.
// Each child consists of a pointer to its left page, a DBF record number, and a key.
//
// After the final entry, there may be an "extra" entry
// with the left page index of values smaller than some element larger than any in this list.
// Functionally, it can be thought of as the right-ward branch relative the final element.
define_layout!(clipper_index_header, LittleEndian, {
    signature: u8,
    binary_version: u8,
    indexing_version: u8,
    compiler_version: u8,
    root_page_addr: u32,
    next_page_addr: u32,
    key_size_plus_8: u16,
    key_size: u16,
    num_dec_in_key: u16,
    max_keys_per_page: u16, // maximum number of keys with pointers that can fit on an index page
    half_page: u16, // the above value, divided by 2
    key_expression: [u8; 256], // expression on which index was built; null-terminated
    is_unique: u8,  // 1 = unique, 0 = NOT unique
});

define_layout!(clipper_index_page, LittleEndian, {
    used_entries: u16, // number of used entries on the current page
});

define_layout!(clipper_index_offset, LittleEndian, {
    offset: u16, // 0x00=No record; others are offsets from start of page
});

define_layout!(clipper_index_entry, LittleEndian, {
    next_page_address: u32,
    record_number: u32,  // in DBF
});

#[derive(Debug)]
pub enum FieldType {
    Character,
    Date,
    Float,
    Boolean,
    Memo,
    Numeric,
}

#[derive(Debug, Default)]
pub struct Decimal {
    mantissa: i64,
    exponent: u32,
}

#[allow(unused)]
impl Decimal {
    fn integral(&self) -> i64 {
        self.mantissa / (10_i64.pow(self.exponent))
    }

    fn fractional(&self) -> u64 {
        if self.mantissa > 0 {
            (self.mantissa % (10_i64.pow(self.exponent))) as u64
        } else {
            (-self.mantissa % (10_i64.pow(self.exponent))) as u64
        }
    }

    fn to_f64_lossy(&self) -> f64 {
        return self.integral() as f64 + self.fractional() as f64;
    }
}

impl Display for Decimal {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if f.width().is_none() {
            write!(f, "{}.{:0width$}", self.integral(), self.fractional(), width = self.exponent as usize)
        } else {
            let s = format!("{}.{:0width$}", self.integral(), self.fractional(), width = self.exponent as usize);
            write!(f, "{s:>width$}", width = f.width().unwrap())
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum Field {
    Character(String),
    Date(NaiveDate),
    Float(f64),
    Boolean(Option<bool>),
    Memo(Option<u64>),
    Numeric(Option<Decimal>),
}

#[allow(unused)]
#[derive(Debug)]
pub struct FieldDescriptor {
    pub name: String,
    field_type: FieldType,
    pub length: usize,
    decimal_count: u8,
    work_area_id: u16,
    example: u8,
}

#[allow(unused)]
#[derive(Error, Debug)]
pub enum DBaseErrorKind {
    #[error("non-utf8 text data")]
    InvalidUTF8,
    #[error("unknown logical value: {}", .0)]
    UnknownLogicalValue(String),
    #[error("unknown field type: {:x}", .0)]
    UnknownFieldType(u8),
    #[error("invalid last updated date: {:04}-{:02}-{:02}", .0, .1, .2)]
    InvalidLastUpdated(u16, u8, u8),

    #[error(transparent)]
    FloatConversionError(#[from] ParseFloatError),
    #[error(transparent)]
    NumericConversionError(#[from] ParseIntError),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

pub type DBaseResult<T> = Result<T, DBaseErrorKind>;

fn data_to_string(data: &[u8]) -> String {
    String::from_utf8_lossy(&data)
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

impl FieldDescriptor {
    fn from_bytes(data: &[u8]) -> DBaseResult<FieldDescriptor> {
        let view = field_descriptor::View::new(data);

        let name = data_to_string(view.name());

        let field_type = match view.f_type().read() {
            b'C' => Ok(FieldType::Character),
            b'D' => Ok(FieldType::Date),
            b'F' => Ok(FieldType::Float),
            b'L' => Ok(FieldType::Boolean),
            b'M' => Ok(FieldType::Memo),
            b'N' => Ok(FieldType::Numeric),
            uft => Err(UnknownFieldType(uft)),
        }?;

        Ok(FieldDescriptor {
            name,
            field_type,
            length: view.length().read() as usize,
            decimal_count: view.decimal_count().read(),
            work_area_id: view.work_area_id().read(),
            example: view.example().read(),
        })
    }

    pub fn read_field(&self, data: &[u8]) -> DBaseResult<Field> {
        let val = data_to_string(&data[0..self.length]);
        match self.field_type {
            FieldType::Character => {
                Ok(Field::Character(val))
            }
            FieldType::Date => {
                Ok(Field::Memo(None))
            }
            FieldType::Float => {
                Ok(Field::Float(f64::from_str(&val)?))
            }
            FieldType::Numeric => {
                if val.is_empty() {
                    return Ok(Field::Numeric(None));
                }

                let dec_point = val.find('.');
                if dec_point.is_none() {
                    let mantissa = i64::from_str(&val)?;
                    return Ok(Field::Numeric(Some(Decimal { mantissa, exponent: 0 })));
                }

                let (integral_s, fractional_s) = val.split_at(dec_point.unwrap());
                let fractional_s = &fractional_s[1..];
                let exponent = fractional_s.len() as u32;

                log::trace!("val: {}, point: {:?} int: {}, frac: {}, exp: {}",
                    val, dec_point, integral_s, fractional_s, exponent);

                fn empty_to_zero(err: ParseIntError) -> Result<i64, ParseIntError> {
                    match err.kind() {
                        std::num::IntErrorKind::Empty => Ok(0),
                        _ => Err(err)
                    }
                }

                let integral = i64::from_str(&integral_s).or_else(empty_to_zero)?;
                let fractional = i64::from_str(&fractional_s).or_else(empty_to_zero)?;
                let mantissa = integral * (10_i64.pow(exponent)) + fractional;
                Ok(Field::Numeric(Some(Decimal { mantissa, exponent })))
            }
            FieldType::Boolean => {
                match val.as_str() {
                    "y" | "Y" | "t" | "T" => { Ok(Field::Boolean(Some(true))) }
                    "n" | "N" | "f" | "F" => { Ok(Field::Boolean(Some(false))) }
                    "?" => Ok(Field::Boolean(None)),
                    _ => Err(UnknownLogicalValue(val)),
                }
            }
            FieldType::Memo => {
                if val.is_empty() {
                    Ok(Field::Memo(None))
                } else {
                    Ok(Field::Memo(Some(u64::from_str(&val)?)))
                }
            }
        }
    }
}

#[allow(unused)]
#[derive(Debug)]
pub struct DBaseTable {
    last_updated: NaiveDate,
    flags: u8,
    pub fields: Vec<FieldDescriptor>,
    pub n_records: usize,
}

pub trait TableReaderState {}

pub struct Header<R> {
    inner: R,
}

impl<R> TableReaderState for Header<R> {}
impl<R: io::Read> TableReaderState for Records<R> {}


pub struct TableReader<S: TableReaderState> {
    table: Box<DBaseTable>,
    state: S,
}

pub fn try_from_path<P: AsRef<Path>>(path: P) -> DBaseResult<TableReader<Header<impl io::Read>>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    TableReader::<Header<BufReader<File>>>::new(reader)
}

impl<S: TableReaderState> TableReader<S> {
    pub fn n_records(&self) -> usize {
        self.table.n_records
    }

    pub fn n_header_bytes(&self) -> usize {
        self.table.fields.len() * 32 + 33
    }
}

impl<R> TableReader<Header<R>>
    where R: io::Read
{
    pub fn new(mut reader: R) -> DBaseResult<Self> {
        let mut data: [u8; 32] = [0; 32];
        reader.read_exact(&mut data)?;

        let view = dbase_header::View::new(&data[..32]);

        let flags = view.flags().read();
        let last_update_year = view.last_updated().year().read() as u16 + 2000;  // This "should" be `+ 1900` because it isn't updated for Y2K.
        let last_update_month = view.last_updated().month().read();
        let last_update_day = view.last_updated().day().read();
        let n_records = view.n_records().read() as usize;
        let n_header_bytes = view.n_header_bytes().read() as usize;
        let n_fields = (n_header_bytes - 31) / 32;

        let last_updated = NaiveDate::from_ymd_opt(
            last_update_year as i32,
            last_update_month as u32,
            last_update_day as u32,
        ).ok_or(InvalidLastUpdated(last_update_year, last_update_month, last_update_day))?;

        let mut fields = Vec::<FieldDescriptor>::with_capacity(n_fields);
        for _ in 0..n_fields {
            reader.read_exact(&mut data)?;
            fields.push(FieldDescriptor::from_bytes(&data)?);
        }

        let table = DBaseTable {
            last_updated,
            fields,
            flags,
            n_records,
        };

        let mut terminator: [u8; 1] = [0];
        reader.read_exact(&mut terminator)?;
        assert_eq!(terminator[0], 0x0d);
        // assert_eq!(table.n_header_bytes(), n_header_bytes);

        Ok(TableReader {
            table: Box::new(table),
            state: Header {
                inner: reader,
            },
        })
    }

    pub fn records<'a>(self) -> TableReader<Records<R>> {
        let record_size = 1 + self.table.fields.iter().fold(0, |s, f| s + f.length);
        log::info!("Record size: {record_size}");

        TableReader{
            table: self.table,
            state: Records {
                record_size,
                cur_record: 0,
                inner: self.state.inner,
            }
        }
    }
}


#[derive(Debug)]
pub struct Records<R: io::Read> {
    inner: R,
    record_size: usize,
    cur_record: usize,
}

#[derive(Debug)]
pub struct FieldIterator<'a> {
    table: &'a DBaseTable,
    buf: Vec<u8>,
    cur_field: usize,
    cur_byte: usize,
}

#[derive(Debug)]
pub struct FieldValue<'a> {
    pub name: &'a str,
    pub value: Field,
}

impl<R: io::Read> TableReader<Records<R>>
{
    pub fn next(&mut self) -> Option<DBaseResult<FieldIterator>> {
        const DELETED: u8 = 0x2a;

        if self.state.cur_record == self.table.n_records {
            return None;
        }

        let mut buf = vec![0; self.state.record_size];
        loop {
            if let Err(err) = self.state.inner.read_exact(&mut buf) {
                return Some(Err(DBaseErrorKind::IOError(err)));
            }
            if buf[0] != DELETED {
                break;
            }
            log::info!("Record {} is deleted", self.state.cur_record);
        }

        self.state.cur_record += 1;

        Some(Ok(FieldIterator {
            table: &self.table,
            buf,
            cur_field: 0,
            cur_byte: 1,
        }))
    }
}

impl<'a> Iterator for FieldIterator<'a> {
    type Item = DBaseResult<FieldValue<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_field == self.table.fields.len() {
            return None;
        }

        let f = &self.table.fields[self.cur_field];
        let r = f.read_field(&self.buf[self.cur_byte..]);

        match r {
            Err(err) => Some(Err(err)),
            Ok(value) => {
                self.cur_field += 1;
                self.cur_byte += f.length;
                Some(Ok(FieldValue {
                    name: &f.name,
                    value,
                }))
            }
        }
    }
}
