use std::fmt::{Display, Formatter};
use std::fs::File;
use std::num::{ParseFloatError, ParseIntError};
use std::ops::{BitAnd, Rem};
use std::str::FromStr;
use std::io::{Cursor};

use log;

use binary_layout::prelude::*;
use chrono::{NaiveDate};
use memmap2::Mmap;
use thiserror::Error;

use crate::DBaseErrorKind::{InvalidLastUpdated, UnknownFieldType, UnknownLogicalValue};

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
enum FieldType {
    Character,
    Date,
    Float,
    Boolean,
    Memo,
    Numeric,
}

#[derive(Debug, Default)]
struct Decimal {
    mantissa: i64,
    exponent: u32,
}

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

#[derive(Debug)]
enum Record {
    Character(String),
    Date(NaiveDate),
    Float(f64),
    Boolean(Option<bool>),
    Memo(Option<u64>),
    Numeric(Option<Decimal>),
}

#[derive(Debug)]
struct FieldDescriptor {
    name: String,
    field_type: FieldType,
    length: usize,
    decimal_count: u8,
    work_area_id: u16,
    example: u8,
}

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
            name: name,
            field_type: field_type,
            length: view.length().read() as usize,
            decimal_count: view.decimal_count().read(),
            work_area_id: view.work_area_id().read(),
            example: view.example().read(),
        })
    }

    fn read_record(&self, data: &[u8]) -> DBaseResult<Record> {
        let val = data_to_string(&data[0..self.length]);
        match self.field_type {
            FieldType::Character => {
                Ok(Record::Character(val))
            }
            FieldType::Date => {
                Ok(Record::Memo(None))
            }
            FieldType::Float => {
                Ok(Record::Float(f64::from_str(&val)?))
            }
            FieldType::Numeric => {
                if val.is_empty() {
                    return Ok(Record::Numeric(None));
                }

                let dec_point = val.find('.');
                if dec_point.is_none() {
                    let mantissa = i64::from_str(&val)?;
                    return Ok(Record::Numeric(Some(Decimal { mantissa, exponent: 0 })));
                }

                let (integral_s, fractional_s) = val.split_at(dec_point.unwrap());
                let fractional_s = &fractional_s[1..];
                let exponent = fractional_s.len() as u32;

                log::debug!("val: {}, point: {:?} int: {}, frac: {}, exp: {}",
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
                Ok(Record::Numeric(Some(Decimal { mantissa, exponent })))
            }
            FieldType::Boolean => {
                match val.as_str() {
                    "y" | "Y" | "t" | "T" => { Ok(Record::Boolean(Some(true))) }
                    "n" | "N" | "f" | "F" => { Ok(Record::Boolean(Some(false))) }
                    "?" => Ok(Record::Boolean(None)),
                    _ => Err(UnknownLogicalValue(val)),
                }
            }
            FieldType::Memo => {
                if val.is_empty() {
                    Ok(Record::Memo(None))
                } else {
                    Ok(Record::Memo(Some(u64::from_str(&val)?)))
                }
            }
        }
    }
}

fn main() {
    env_logger::init();

    // let path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/IGRANDX.NTX";
    // let path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/MYINDEX.NTX";
    let az_index_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/AZINDEX1.NTX";
    let name_index_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/NAMENDX.NTX";
    let igra_index_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/IGRANDX.NTX";
    let aa_igra_index_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/after-adding/IGRANDX.NTX";
    let aa_name_index_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/after-adding/NAMENDX.NTX";
    let my_igra_index_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/mine/IGRANDX.NTX";
    let my_name_index_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/mine/NAMENDX.NTX";
    let az_events_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/AZEVENTS.DBF";
    let personnel_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/PERSONEL.DBF";

    let (az_events_dbase, mmapped) = DBaseTable::try_open(az_events_path).expect("opened dbase");
    read_rodeo_events(&mmapped[az_events_dbase.n_header_bytes()..], &az_events_dbase.fields);

    let (dbase, mmapped) = DBaseTable::try_open(personnel_path).expect("opened dbase");
    print_master_list(&mmapped[dbase.n_header_bytes()..], &dbase.fields, dbase.n_records);

    // let (mmapped, n_records, n_header_bytes, fields) = open_dbase_file(personnel_path);
    // print_master_list(&mmapped[n_header_bytes..], &fields, n_records);


    // round_trip_index::<IGRANumber>(az_index_path).expect("round-trip failed");
    // round_trip_index::<IGRANumber>(igra_index_path).expect("round-trip failed");
    // round_trip_index::<LegalLast>(name_index_path).expect("round-trip failed");

    // round_trip_index::<IGRANumber>(my_igra_index_path).expect("round-trip failed");
    // round_trip_index::<LegalLast>(my_name_index_path).expect("round-trip failed");

    // round_trip_index::<IGRANumber>(aa_igra_index_path).expect("round-trip failed");
    // round_trip_index::<LegalLast>(aa_name_index_path).expect("round-trip failed");

    // reindex::<IGRANumber>(igra_index_path, my_igra_index_path).expect("reindex failed");
    // reindex::<LegalLast>(name_index_path,my_name_index_path).expect("reindex failed");
}

fn experiment(n: usize) -> IndexResult<()> {
    let az_index_path = "/media/mnt/raid/projects/IGRA/old-data-management/shared/AZINDEX1.NTX";
    let mut c_idx = ClipperIndex::<IGRANumber>::try_open(az_index_path)?;
    let orig_len = c_idx.items.len();

    let mut new_items: Vec<IndexedItem<IGRANumber>> = c_idx.items.iter()
        .enumerate()
        .flat_map(|(idx, item)|
            (0..n).map(move |i| {
                IndexedItem {
                    record: item.record.clone(),
                    record_number: (n * idx + i) as u32,
                }
            })
        ).collect();
    new_items.sort_by(|a, b| a.record.0.cmp(&b.record.0));

    let mut buff = Cursor::new(Vec::<u8>::new());
    c_idx.write_items(&mut buff, &new_items)?;
    let c_idx = ClipperIndex::<IGRANumber>::try_from(buff.get_ref().as_slice())?;
    for (rn, item) in c_idx.items.iter().enumerate() {
        println!("{:02} {}", item.record_number, item.record);
        if rn != item.record_number as usize {
            break;
        }
    }

    println!("N Items: {}; expected: {}", c_idx.items.len(), n * orig_len);
    assert_eq!(c_idx.items.len(), n * orig_len);

    Ok(())
}

fn reindex<T: ReadableRecord + WritableRecord>(in_path: &str, out_path: &str) -> IndexResult<()> {
    let c_idx = ClipperIndex::<T>::try_open(in_path)?;
    let mut file = File::create(out_path)?;
    c_idx.write_items(&mut file, &c_idx.items)
}

fn round_trip_index<T: ReadableRecord + WritableRecord + Display>(path: &str) -> IndexResult<()> {
    let c_idx = ClipperIndex::<T>::try_open(path)?;

    let orig = c_idx.items.len();
    {
        for item in &c_idx.items {
            println!("{:02} {}", item.record_number, item.record);
        }
    }

    let mut buff = Cursor::new(Vec::<u8>::new());
    c_idx.write_items(&mut buff, &c_idx.items)?;

    let c_idx = ClipperIndex::<T>::try_from(buff.get_ref().as_slice())?;
    let mut prev: Option<&IndexedItem<T>> = None;
    for item in &c_idx.items {
        println!("{:02} {}", item.record_number, item.record);
        prev = Some(item);
    }

    println!("Original N Items: {}; after: {}", orig, c_idx.items.len());

    Ok(())
}

#[derive(Error, Debug)]
enum IndexErrorKind {
    #[error("key expression is longer than 255")]
    KeyExpressionTooLong,

    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

trait Indexable {
    fn write(&self, data: &mut [u8]) -> Result<usize, std::io::Error>;
    fn read(&mut self, data: &[u8]) -> Result<(), std::io::Error> where Self: Sized;
}

trait IGRANumbered {
    fn igra_number(&self) -> &str;
    fn set_igra_number(&self, igra_number: &str);
}

impl Indexable for dyn IGRANumbered {
    fn write(&self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if buf.len() <= 4 {
            std::io::Error::new(std::io::ErrorKind::WriteZero, "buffer is too small");
        }

        buf.copy_from_slice(self.igra_number().as_bytes());
        Ok(4)
    }

    fn read(&mut self, data: &[u8]) -> Result<(), std::io::Error> {
        let key = data_to_string(&data[..4]);
        log::debug!(" Read IGRA Number: {key}");
        self.set_igra_number(&key);
        Ok(())
    }
}

impl Indexable for IGRANumber {
    fn write(&self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if buf.len() <= 4 {
            std::io::Error::new(std::io::ErrorKind::WriteZero, "buffer is too small");
        }

        buf.copy_from_slice(self.0.as_bytes());
        Ok(4)
    }

    fn read(&mut self, data: &[u8]) -> Result<(), std::io::Error> {
        let key = data_to_string(&data[..4]);
        log::debug!(" Read IGRA Number: {key}");
        self.0.clone_from(&key);
        Ok(())
    }
}

impl Indexable for LegalLast {
    fn write(&self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if buf.len() <= 17 {
            std::io::Error::new(std::io::ErrorKind::WriteZero, "buffer is too small");
        }

        let self_len = self.0.len();
        buf[..self_len].copy_from_slice(self.0.as_bytes());
        buf[self_len..].fill(0x20);
        Ok(17)
    }

    fn read(&mut self, data: &[u8]) -> Result<(), std::io::Error> {
        let key = data_to_string(&data[..17]);
        log::debug!(" Read Legal Last: {key}");
        self.0.clone_from(&key);
        Ok(())
    }
}

#[derive(Debug)]
struct IndexedItem<T> {
    record_number: u32,
    record: T,
}

type IndexResult<T> = Result<T, IndexErrorKind>;

struct ClipperIndex<T: Indexable> {
    key_expression: String,

    signature: u8,
    binary_version: u8,
    indexing_version: u8,
    compiler_version: u8,

    key_size: u16,
    num_dec_in_key: u16,
    is_unique: bool,

    items: Vec<IndexedItem<T>>,
}

trait DivCeil<Rhs = Self> {
    type Output;
    fn my_div_ceil(self, rhs: Rhs) -> Self::Output;
}

macro_rules! div_ceil_impl_integer {
    ($($t:ty)*) => ($(
    impl DivCeil for $t {
        type Output = $t;

        #[inline]
        fn my_div_ceil(self, rhs: $t) -> $t {
            self / rhs + if self % rhs == 0 { 0 } else { 1 }
        }
    }
    )*)
}
div_ceil_impl_integer! { usize u8 u16 u32 u64 u128 isize i8 i16 i32 i64 i128 }

impl<T: ReadableRecord + WritableRecord> ClipperIndex<T> {
    fn try_open(path: &str) -> IndexResult<Self> {
        let file = File::open(path)?;
        let mmapped = unsafe { Mmap::map(&file)? };
        ClipperIndex::<T>::try_from(&mmapped)
    }

    pub fn try_from(data: &[u8]) -> IndexResult<Self> {
        let view = clipper_index_header::View::new(data);

        let signature = view.signature().read();
        let binary_version = view.binary_version().read();
        let indexing_version = view.indexing_version().read();
        let compiler_version = view.compiler_version().read();
        let root_page = view.root_page_addr().read();
        let key_size = view.key_size().read();
        let num_dec_in_key = view.num_dec_in_key().read();
        let max_keys_per_page = view.max_keys_per_page().read();
        let key_expression = data_to_string(view.key_expression());
        let is_unique = view.is_unique().read();

        let n_pages = data.len() / 1024;
        let est_n_records = ((n_pages - 1) / (max_keys_per_page as usize)).min(1);
        let mut items = Vec::<IndexedItem<T>>::with_capacity(est_n_records);

        log::debug!("Index on {key_expression} with size {key_size}, <={max_keys_per_page}/page @ 0x{root_page:08x}");

        Self::read_page(data, root_page as usize, key_size as usize, &mut items)?;

        let c_idx = ClipperIndex {
            key_expression,
            signature,
            binary_version,
            indexing_version,
            compiler_version,
            key_size,
            num_dec_in_key,
            is_unique: is_unique == 1,
            items,
        };

        Ok(c_idx)
    }

    fn log_bytes(data: &[u8]) {
        for line in data.chunks(16) {
            for chunk in line.chunks(4) {
                for byte in chunk {
                    print!("{byte:02x} ")
                }
                print!(" ")
            }
            print!("\n")
        }
    }

    fn read_page(data: &[u8], page_start: usize, key_size: usize, records: &mut Vec::<IndexedItem<T>>) -> IndexResult<()> {
        let page_data = &data[page_start..page_start + 1024];
        let view = clipper_index_page::View::new(page_data);
        let no_entries = view.used_entries().read();
        log::debug!("Page 0x{page_start:08x} -- Number of Used Entries: {no_entries}");
        // Self::log_bytes(&page_data);

        for entry_idx in 1..=no_entries {
            let offset = clipper_index_offset::View::new(&page_data[(2 * entry_idx as usize)..]).offset().read() as usize;
            log::debug!("  Page 0x{page_start:08x}  Entry {entry_idx:02} Offset: 0x{offset:04x}");
            if offset == 0x0 {
                continue;
            }

            let view = clipper_index_entry::View::new(&page_data[offset..]);
            let left = view.next_page_address().read() as usize;
            let record_number = view.record_number().read();

            if left != 0x0 {
                Self::read_page(data, left, key_size, records)?;
            }

            log::debug!("    Left Page Address: 0x{left:08x}  Record Number: {record_number}");
            let key_start = offset + 8;
            let record = ReadableRecord::read(&page_data[key_start..key_start + key_size])?;
            records.push(IndexedItem::<T> { record_number, record });
        }

        // Read the last entry to see if there's a right page.
        let offset = clipper_index_offset::View::new(&page_data[(2 * (1 + no_entries) as usize)..]).offset().read() as usize;
        if offset == 0x0 {
            return Ok(());
        }

        let view = clipper_index_entry::View::new(&page_data[offset..]);
        let right = view.next_page_address().read() as usize;
        let record_number = view.record_number().read();
        if right != 0x0 {
            log::debug!("  Page 0x{page_start:08x}  Final Entry Offset: 0x{offset:04x}");
            log::debug!("    Right Page Address: 0x{right:08x}  Record Number: {record_number}");
            Self::read_page(data, right, key_size, records)?;
        }

        Ok(())
    }

    /// This assumes items are already sorted.
    pub fn write_items(&self, writer: &mut impl std::io::Write, items: &Vec<IndexedItem<T>>) -> Result<(), IndexErrorKind> {
        if self.key_expression.len() > 255 {
            return Err(IndexErrorKind::KeyExpressionTooLong);
        }

        let key_size = self.key_size as usize;

        // Page size is 1024, -2 for the the number of entries used.
        // Key size is +4 for the next page address, +4 for the record number, +2 for the index to the entry.
        // The max_per_page is -1 because we have to reserve room for an entry to the "right" page.
        let mut max_per_page = ((1024 - 2) as usize / (key_size + 10)) - 1;
        // max_per_page must be even.
        if max_per_page.bitand(&1) == 1 {
            max_per_page -= 1;
        }

        let mpp1 = max_per_page + 1; // Used a bunch below.

        // This address reserves enough space for the maximum number of entries.
        // First offset is at +2 for the "number of entries" +2 bytes per entry.
        // There's actually an extra entry after the official ones, hence the +1.
        let first_offset = 2 + (2 * mpp1);

        // Repeatedly split n items to a page, then reserve an item for the next level of the tree.
        // Once all items for the current level are determined, swap to the reserved items (the next level).
        //
        // When reserving an item, note its left page (which is the current one) and its right page,
        // which is either the one just after the current page (if we'll write more on this level)
        // or the previously recorded right page address (if this is the last item on the level).
        // Initialize the list for the lowest level with 0s for both the left and right addresses.
        //
        // After the last item on a page, we'll write its right page address as a final element.
        // Functionally, this is an extra element greater than any key, so its left page it right of the previous element.
        let mut page_idx = 0;
        let mut page_addr = 0_u32;
        let mut level_idx = 0;
        let mut level: Vec<_> = items.iter().map(|item| (item, 0_u32, 0_u32)).collect();
        let mut pages = Vec::<[u8; 1024]>::new();
        while level.len() > 0 {
            let mut next_level = Vec::<(&IndexedItem<T>, u32, u32)>::new();  // This holds items meant for the next level.

            let pages_this_level = level.len().my_div_ceil(mpp1);

            log::debug!("---- NEW LEVEL ---- ... {} items, {} pages", level.len(), pages_this_level);

            let mut paged = level.chunks(mpp1);
            for pgl_idx in 0..(pages_this_level.saturating_sub(2)) {
                let chunk = paged.next().unwrap();
                page_addr += 1024;
                page_idx += 1;
                log::debug!("--New Page: {:2} @ {:04x}--", page_idx, page_addr);

                // The left page for the reserved item is the page we're about to write.
                // There are pages after this, so the right page will be the next page.
                let (final_item, chunk) = chunk.split_last().unwrap();
                next_level.push((final_item.0, page_addr, page_addr + 1024));
                pages.push(Self::write_page(&key_size, first_offset, chunk)?);
            }

            // Balance the last 2-3 pages to maintain the B-Tree constraint
            // that each non-root node have at least ceil(n/2) entries.
            // Typically this will be 2 pages, with a root page of 1 item,
            // but if there are exactly (2*(max_per_page + 1)) remaining items,
            // split into three pages with two items on the root page.
            page_addr += 1024;
            page_idx += 1;

            let last_pages = [paged.next().unwrap(), paged.next().unwrap_or_default()].concat();
            let (before, middle, after) = if last_pages.len() == 2 * mpp1 {
                log::debug!("Split {} items into 3 pages.", last_pages.len());
                let middle_0 = last_pages.len() / 3;
                let middle_1 = 2 * last_pages.len() / 3;
                next_level.push((last_pages[middle_0].0, page_addr, page_addr + 1024));
                next_level.push((last_pages[middle_1].0, page_addr + 1024, page_addr + 2048));
                (
                    &last_pages[..middle_0],
                    &last_pages[(middle_0 + 1)..middle_1],
                    &last_pages[middle_1 + 1..]
                )
            } else if last_pages.len() >= 3 {
                log::debug!("Split {} items into 2 pages with next level.", last_pages.len());
                let middle_idx = last_pages.len() / 2;
                next_level.push((last_pages[middle_idx].0, page_addr, page_addr + 1024));
                (
                    &last_pages[..middle_idx],
                    &[] as &[(&IndexedItem<T>, u32, u32)],
                    &last_pages[middle_idx + 1..],
                )
            } else {
                log::debug!("Root page with {} items.", last_pages.len());
                (
                    &last_pages[..],
                    &[] as &[(&IndexedItem<T>, u32, u32)],
                    &[] as &[(&IndexedItem<T>, u32, u32)],
                )
            };

            if !middle.is_empty() && last_pages.is_empty() {
                assert!(before.len() >= max_per_page.my_div_ceil(2), "too few before: {} < {} (after={})",
                        before.len(), max_per_page.my_div_ceil(2), after.len());
                assert!(after.len() >= max_per_page.my_div_ceil(2), "too few after: {} < {} (before={})",
                        after.len(), max_per_page.my_div_ceil(2), before.len());
                assert!(after.len() <= max_per_page, "too many after: {} > {} (before={})",
                        after.len(), max_per_page, before.len());
                assert!((after.len() as i64 - before.len() as i64).abs() <= 1, "{}, {}", after.len(), before.len());
            }
            assert!(before.len() <= max_per_page, "too many before: {} > {} (after={})",
                    before.len(), max_per_page, after.len());

            log::debug!("--New Page: {:2} @ {:04x}--", page_idx, page_addr);
            pages.push(Self::write_page(&key_size, first_offset, before)?);

            if !middle.is_empty() {
                page_addr += 1024;
                page_idx += 1;
                log::debug!("--New Page: {:2} @ {:04x}--", page_idx, page_addr);
                pages.push(Self::write_page(&key_size, first_offset, middle)?);
            }

            if !after.is_empty() {
                page_addr += 1024;
                page_idx += 1;
                log::debug!("--New Page: {:2} @ {:04x}--", page_idx, page_addr);
                pages.push(Self::write_page(&key_size, first_offset, after)?);
            }

            level = next_level;
            level_idx += 1;
        }

        let page_count = pages.len();
        let root_address = (1024 * page_count).try_into().expect("root address does not fit in a u32");
        log::debug!("Max Per Page: {max_per_page}");
        log::debug!("Page Count: {page_count}");

        {
            let mut page: [u8; 1024] = [0; 1024];
            let mut v = clipper_index_header::View::new(&mut page);
            // v.signature_mut().write(0b0000_0110);
            // v.indexing_version_mut().write(0b0000_0001);
            v.signature_mut().write(self.signature);
            v.indexing_version_mut().write(self.indexing_version);
            v.binary_version_mut().write(self.binary_version);
            v.compiler_version_mut().write(self.compiler_version);
            v.root_page_addr_mut().write(root_address);
            v.key_size_plus_8_mut().write(key_size as u16 + 8);
            v.key_size_mut().write(key_size as u16);
            v.num_dec_in_key_mut().write(self.num_dec_in_key);
            v.max_keys_per_page_mut().write(max_per_page as u16);
            v.half_page_mut().write(max_per_page as u16 / 2);
            v.key_expression_mut()[..self.key_expression.len()].copy_from_slice(self.key_expression.as_bytes());
            v.is_unique_mut().write(if self.is_unique { 1 } else { 0 });
            writer.write(&page)?;
        }

        for page in pages {
            writer.write(&page)?;
        }

        Ok(())
    }

    fn write_page(key_size: &usize, first_offset: usize, chunk: &[(&IndexedItem<T>, u32, u32)]) -> Result<[u8; 1024], IndexErrorKind> {
        let mut page: [u8; 1024] = [0; 1024];

        // Write the header, which is the "number of entries" followed by "offsets to entries".
        let mut v = clipper_index_page::View::new(&mut page);
        v.used_entries_mut().write((chunk.len()).try_into().expect("too many entries"));

        // We're just going to write the items (and thus these offsets) sequentially.
        let mut right = 0_u32;
        for (idx, (item, lp, rp)) in chunk.iter().enumerate() {
            right = *rp;

            // Again, here the +2 reserves space for the "number of entries".
            let mut v = clipper_index_offset::View::new(&mut page[2 + idx * 2..]);
            let offset = first_offset + (idx * (key_size + 8));
            v.offset_mut().write(offset as u16);

            log::debug!("  Entry {:02}  Offset: 0x{:04x}  Left: 0x{:08x}  Record: {}",
                        idx + 1, offset, lp, item.record_number);

            let mut v = clipper_index_entry::View::new(&mut page[offset..]);
            v.next_page_address_mut().write(*lp);
            v.record_number_mut().write(item.record_number);
            item.record.write(&mut page[offset + 8..(offset + 8 + key_size)])?;
        }

        let idx = chunk.len();
        let offset = first_offset + (idx * (key_size + 8));
        let mut v = clipper_index_offset::View::new(&mut page[2 + idx * 2..]);
        v.offset_mut().write(offset as u16);

        log::debug!("  Final     Offset: 0x{:04x}  Right: 0x{:08x}  Record: {}", offset, right, 0);
        let mut v = clipper_index_entry::View::new(&mut page[offset..]);
        v.next_page_address_mut().write(right);

        Ok(page)
    }
}

struct DBaseTable {
    last_updated: NaiveDate,
    flags: u8,
    fields: Vec<FieldDescriptor>,
    n_records: usize,
}

type DBaseResult<T> = Result<T, DBaseErrorKind>;

impl DBaseTable {
    fn n_header_bytes(&self) -> usize {
        self.fields.len() * 32 + 33
    }

    fn try_open(path: &str) -> DBaseResult<(DBaseTable, Mmap)> {
        let file = File::open(path)?;
        let mmapped = unsafe { Mmap::map(&file)? };
        let dbase = DBaseTable::try_from(&mmapped)?;
        Ok((dbase, mmapped))
    }

    fn try_from(data: &[u8]) -> DBaseResult<DBaseTable> {
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
        for field in data[32..(n_header_bytes - 1)].chunks(32) {
            fields.push(FieldDescriptor::from_bytes(field)?);
        }
        let table = DBaseTable {
            last_updated,
            fields,
            flags,
            n_records,
        };

        assert_eq!(table.n_header_bytes(), n_header_bytes);
        Ok(table)
    }
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

fn print_master_list(mmapped: &[u8], fields: &Vec<FieldDescriptor>, n_records: usize) {
    let record_size = 1 + fields.iter().fold(0, |s, f| s + f.length);
    let mut people = Vec::<PersonRecord>::with_capacity(n_records);
    let mut i = 0;

    while i + 1 < mmapped.len() {
        if mmapped[i] == 0x2a {
            // Record is deleted.
            i += record_size;
        } else {
            i += 1;
        }

        let mut person = PersonRecord::default();

        for f in fields {
            let r = f.read_record(&mmapped[i..i + f.length]);
            i += f.length;
            if r.is_err() {
                log::error!("{:?}", r);
                continue;
            }

            match (&f.name.as_str(), r.unwrap()) {
                (&"IGRA_NUM", Record::Character(s)) => person.igra_number = s,
                (&"STATE_ASSN", Record::Character(s)) => person.association = s,
                (&"BIRTH_DATE", Record::Character(s)) => person.birthdate = s,
                (&"SSN", Record::Character(s)) => person.ssn = s,
                (&"DIVISION", Record::Character(s)) => person.division = s,
                (&"LAST_NAME", Record::Character(s)) => person.last_name = s,
                (&"FIRST_NAME", Record::Character(s)) => person.first_name = s,
                (&"LEGAL_LAST", Record::Character(s)) => person.legal_last = s,
                (&"LEGALFIRST", Record::Character(s)) => person.legal_first = s,
                (&"ID_CHECKED", Record::Character(s)) => person.id_checked = s,
                (&"SEX", Record::Character(s)) => person.sex = s,
                (&"ADDRESS", Record::Character(s)) => person.address = s,
                (&"CITY", Record::Character(s)) => person.city = s,
                (&"STATE", Record::Character(s)) => person.state = s,
                (&"HOME_PHONE", Record::Character(s)) => person.home_phone = s,
                (&"CELL_PHONE", Record::Character(s)) => person.cell_phone = s,
                (&"E_MAIL", Record::Character(s)) => person.email = s,
                (&"STATUS", Record::Character(s)) => person.status = s,
                (&"FIRSTRODEO", Record::Character(s)) => person.first_rodeo = s,
                (&"LASTUPDATE", Record::Character(s)) => person.last_updated = s,
                (&"SORT_DATE", Record::Character(s)) => person.sort_date = s,
                (&"EXT_DOLLAR", Record::Numeric(Some(n))) => person.ext_dollars = n,
                _ => {}
            }
        }

        people.push(person);
    }

    people.sort_by(|a, b| a.igra_number.cmp(&b.igra_number));
    for person in people {
        println!("{person}")
    }
}

fn read_rodeo_events(mmapped: &[u8], fields: &Vec<FieldDescriptor>) {
    let mut i = 0;
    while i + 1 < mmapped.len() {
        i += 1;
        let mut entrant = RegistrationRecord::default();

        for f in fields {
            let r = f.read_record(&mmapped[i..i + f.length]);
            i += f.length as usize;

            if r.is_err() {
                log::error!("{:?}", r);
                continue;
            }

            if f.name.ends_with("_SAT") || f.name.ends_with("_SUN") {
                let is_x = if let Ok(Record::Character(ref x)) = r { x == "X" } else { false };

                if &f.name[5..6] == "E" && is_x {
                    let mut evnt = EventRecord::default();
                    evnt.name = f.name.clone();
                    entrant.events.push(evnt);
                } else if let Some(evnt) = entrant.events.iter_mut().find(|e| {
                    e.name[..4] == f.name[..4] && e.name[6..] == f.name[6..]
                }) {
                    match (&f.name.as_str()[5..6], r.unwrap()) {
                        ("S", Record::Numeric(Some(n))) => { evnt.outcome = Some(EventMetric::Score(n)) }
                        ("T", Record::Numeric(Some(n))) => { evnt.outcome = Some(EventMetric::Time(n)) }
                        ("P", Record::Numeric(Some(n))) => { evnt.points = n }
                        ("D", Record::Numeric(Some(n))) => { evnt.dollars = n }
                        ("W", Record::Numeric(Some(n))) => { evnt.world = n }
                        _ => {}
                    }
                }

                continue;
            }

            match (&f.name.as_str(), r.unwrap()) {
                (&"IGRA_NUM", Record::Character(s)) => entrant.igra_number = s,
                (&"FIRST_NAME", Record::Character(s)) => entrant.first_name = s,
                (&"LAST_NAME", Record::Character(s)) => entrant.last_name = s,
                (&"SEX", Record::Character(s)) => entrant.sex = s,
                (&"CITY", Record::Character(s)) => entrant.city = s,
                (&"STATE", Record::Character(s)) => entrant.state = s,
                (&"STATE_ASSN", Record::Character(s)) => entrant.association = s,
                (&"SSN", Record::Character(s)) => entrant.ssn = s,
                (&"SAT_POINTS", Record::Numeric(Some(n))) => entrant.sat_points = n,
                (&"SUN_POINTS", Record::Numeric(Some(n))) => entrant.sun_points = n,
                (&"EXT_POINTS", Record::Numeric(Some(n))) => entrant.ext_points = n,
                (&"TOT_POINTS", Record::Numeric(Some(n))) => entrant.tot_points = n,
                _ => {}
            }
        }

        println!("{}", entrant);
        for evnt in entrant.events {
            println!("\t{}", evnt);
        }
    }
}

/// An event is scored using either Time or Score.
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
#[derive(Debug, Default)]
struct RegistrationHeader {
    event_name: &'static str,
    entered: &'static str,
    outcome: &'static str,
    dollars: &'static str,
    points: &'static str,
    world: &'static str,

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


impl WritableRecord for PersonRecord {
    fn write(&self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if buf.len() <= 4 {
            std::io::Error::new(std::io::ErrorKind::WriteZero, "buffer is too small");
        }

        buf.copy_from_slice(self.igra_number.as_bytes());
        Ok(4)
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
