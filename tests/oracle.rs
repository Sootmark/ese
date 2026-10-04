//! Every test database against libesedb's reading of it (`tests/oracle/`,
//! written by `tests/oracle/gen.sh`): the tables in catalog order, their
//! columns and types, and every value of every record, in order.
//!
//! Two differences are by design, and checked rather than ignored:
//! - A NULL fixed column still takes its bytes in the record; the record's
//!   NULL bitmap says it is NULL. libesedb doesn't read the bitmap and
//!   returns the bytes: the filler ESE writes (0x2a, so 10794 for a Short),
//!   or what the column held before it was set to NULL. The second is only
//!   accepted where listed in [`NULLED_COLUMNS`].
//! - esedbexport decodes 7-bit compressed binary data one byte per unit;
//!   7-bit Unicode data (scheme 2) restores each unit to two bytes, as
//!   compressed Unicode text does.

mod support;

use ese::{Column, ColumnKind, Database, DatabaseState, Table};
use support::OracleTable;

/// What ESE fills a NULL fixed column's bytes with.
const NULL_FILLER: u8 = 0x2a;
/// Fixed columns set to NULL after holding a value, their old bytes left
/// in the record: (database, table, column). Windows Search clears a
/// crawl's priority once done.
const NULLED_COLUMNS: [(&str, &str, &str); 1] = [("Windows.edb", "SystemIndex_Gthr", "Priority")];

fn check(fixture: &str) {
    let db = Database::open(support::fixture(fixture)).unwrap();
    // Nothing wrong but how three of them were left: in use.
    let dirty = db.header.state == DatabaseState::DirtyShutdown;
    assert_eq!(
        db.problems.len(),
        usize::from(dirty),
        "{fixture}: {:?}",
        db.problems
    );
    let oracle = support::oracle(fixture);
    let names: Vec<&str> = db.tables.iter().map(|table| table.name.as_str()).collect();
    let expected: Vec<&str> = oracle.iter().map(|table| table.name.as_str()).collect();
    assert_eq!(names, expected, "{fixture}: tables");
    let mut values = 0;
    for (table, expected) in db.tables.iter().zip(&oracle) {
        values += check_table(fixture, &db, table, expected);
    }
    assert!(values > 0, "{fixture}: no values compared");
}

/// Compare one table; the number of values compared.
fn check_table(fixture: &str, db: &Database, table: &Table, expected: &OracleTable) -> usize {
    let context = format!("{fixture}: {}", table.name);
    assert_eq!(table.column_names(), expected.columns, "{context}: columns");
    let types: Vec<u32> = table
        .columns
        .iter()
        .map(|column| column_type_number(column.column_type))
        .collect();
    assert_eq!(types, expected.types, "{context}: column types");
    let mut rows = db.rows(&table.name).unwrap();
    let ours: Vec<_> = rows.by_ref().collect();
    assert_eq!(rows.problems(), &[] as &[String], "{context}");
    assert_eq!(ours.len(), expected.rows.len(), "{context}: rows");
    for (index, (row, expected_row)) in ours.iter().zip(&expected.rows).enumerate() {
        for ((column, value), expected_value) in
            table.columns.iter().zip(&row.values).zip(expected_row)
        {
            let ours = support::encode(value);
            let nulled =
                NULLED_COLUMNS.contains(&(fixture, table.name.as_str(), column.name.as_str()));
            assert!(
                same(&ours, expected_value, column) || (nulled && ours == "-"),
                "{context}: row {index}, column {}: ours {ours}, libesedb {expected_value}",
                column.name
            );
        }
    }
    ours.len() * table.columns.len()
}

/// Whether our encoded value of `column` is libesedb's (see the module
/// documentation for the two differences).
fn same(ours: &str, expected: &str, column: &Column) -> bool {
    if ours == "-" && is_null_filler(expected, column) {
        return true;
    }
    if let Some(byte) = expected.strip_prefix("b:") {
        return ours == if byte == "00" { "b:0" } else { "b:1" };
    }
    match (ours.strip_prefix("x:"), expected.strip_prefix("w:")) {
        (Some(ours), Some(units)) => ours == units || ours == widened(units),
        _ => ours == expected,
    }
}

/// Whether libesedb's value is the filler of a NULL fixed column.
fn is_null_filler(expected: &str, column: &Column) -> bool {
    if column.kind() != ColumnKind::Fixed {
        return false;
    }
    let size = column.column_type.size().unwrap_or_default();
    let (kind, data) = expected.split_once(':').unwrap_or_default();
    let filler = format!("{NULL_FILLER:02x}");
    let all_filler = |hex: &str| {
        !hex.is_empty()
            && hex
                .as_bytes()
                .chunks(2)
                .all(|byte| byte == filler.as_bytes())
    };
    match kind {
        "b" | "f" | "x" => all_filler(data),
        "i" => data.parse::<i64>().is_ok_and(|value| {
            value.to_le_bytes()[..size]
                .iter()
                .all(|&byte| byte == NULL_FILLER)
        }),
        _ => false,
    }
}

/// Hex bytes, each followed by a zero byte.
fn widened(hex: &str) -> String {
    let mut widened = String::with_capacity(2 * hex.len());
    for byte in hex.as_bytes().chunks(2) {
        widened.push_str(std::str::from_utf8(byte).unwrap());
        widened.push_str("00");
    }
    widened
}

fn column_type_number(column_type: ese::ColumnType) -> u32 {
    use ese::ColumnType::*;
    match column_type {
        Nil => 0,
        Bit => 1,
        UnsignedByte => 2,
        Short => 3,
        Long => 4,
        Currency => 5,
        IeeeSingle => 6,
        IeeeDouble => 7,
        DateTime => 8,
        Binary => 9,
        Text => 10,
        LongBinary => 11,
        LongText => 12,
        Slv => 13,
        UnsignedLong => 14,
        LongLong => 15,
        Guid => 16,
        UnsignedShort => 17,
        Unknown(raw) => raw,
    }
}

#[test]
fn catalog1_edb() {
    check("Catalog1.edb");
}

#[test]
fn srudb_dat() {
    check("SRUDB.dat");
}

#[test]
fn webcache_v01_dat() {
    check("WebCacheV01.dat");
}

#[test]
fn partitions_ex_webcache_v01_dat() {
    check("PartitionsEx-WebCacheV01.dat");
}

#[test]
fn webcache_v01_cookies_dat() {
    check("WebCacheV01_cookies.dat");
}

#[test]
fn windows_edb() {
    check("Windows.edb");
}
