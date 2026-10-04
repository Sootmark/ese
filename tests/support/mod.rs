//! Shared by the integration tests: the test databases (stored
//! gzip-compressed, decompressed once per test binary), libesedb's reading
//! of them, and a walk through everything a database holds.

#![allow(dead_code)] // Each test crate uses its own part of this module.

use std::collections::HashMap;
use std::fmt::Write;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ese::{Database, Value};

/// The test databases, all from plaso's test data.
pub const FIXTURES: [&str; 6] = [
    "Catalog1.edb",
    "SRUDB.dat",
    "WebCacheV01.dat",
    "PartitionsEx-WebCacheV01.dat",
    "WebCacheV01_cookies.dat",
    "Windows.edb",
];

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn gunzip(path: &Path) -> Vec<u8> {
    let compressed = std::fs::read(path).unwrap();
    let mut plain = Vec::new();
    common::gzip::Decoder::new(compressed.as_slice())
        .read_to_end(&mut plain)
        .unwrap();
    plain
}

/// A test database's bytes (`tests/fixtures/plaso/<name>.gz`).
pub fn fixture(name: &str) -> &'static [u8] {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static [u8]>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Mutex::default).lock().unwrap();
    cache.entry(name.to_owned()).or_insert_with(|| {
        let path = tests_dir()
            .join("fixtures/plaso")
            .join(format!("{name}.gz"));
        gunzip(&path).leak()
    })
}

/// A table as libesedb reads it (`tests/oracle/<fixture>.tsv.gz`, written
/// by `tests/oracle/gen.sh`).
#[derive(Debug, Default)]
pub struct OracleTable {
    pub name: String,
    pub columns: Vec<String>,
    pub types: Vec<u32>,
    pub rows: Vec<Vec<String>>,
}

/// libesedb's reading of a test database: every table, in catalog order.
pub fn oracle(fixture: &str) -> Vec<OracleTable> {
    let path = tests_dir().join("oracle").join(format!("{fixture}.tsv.gz"));
    let text = String::from_utf8(gunzip(&path)).unwrap();
    let mut tables: Vec<OracleTable> = Vec::new();
    for line in text.lines() {
        let mut fields = line.split('\t');
        let kind = fields.next().unwrap();
        let fields: Vec<String> = fields.map(str::to_owned).collect();
        if kind == "table" {
            tables.push(OracleTable {
                name: fields[0].clone(),
                ..OracleTable::default()
            });
            continue;
        }
        let table = tables.last_mut().unwrap();
        match kind {
            "columns" => table.columns = fields,
            "types" => table.types = fields.iter().map(|t| t.parse().unwrap()).collect(),
            "row" => table.rows.push(fields),
            _ => panic!("oracle line of unknown kind: {line}"),
        }
    }
    tables
}

/// Text escaped as the oracle escapes it.
pub fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Bytes as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut hex, byte| {
        write!(hex, "{byte:02x}").unwrap();
        hex
    })
}

/// A value as the oracle writes it (multi-values as text joined by "; ",
/// as esedbexport prints them).
pub fn encode(value: &Value) -> String {
    match value {
        Value::Null => "-".to_owned(),
        Value::Bool(bit) => format!("b:{}", u8::from(*bit)),
        Value::F32(_) | Value::F64(_) | Value::DateTime(_) => {
            format!("f:{}", hex(&value.as_f64().unwrap().to_be_bytes()))
        }
        Value::Guid(bytes) => format!("x:{}", hex(bytes)),
        Value::Binary(bytes) => format!("x:{}", hex(bytes)),
        Value::Text(text) => format!("t:{}", escape(text)),
        Value::MultiValue(values) => {
            let texts: Vec<&str> = values
                .iter()
                .map(|value| value.as_text().unwrap())
                .collect();
            format!("j:{}", escape(&texts.join("; ")))
        }
        integer => format!("i:{}", integer.as_i64().unwrap()),
    }
}

/// What a walk through a whole database found.
#[derive(Debug, Default)]
pub struct Walk {
    pub rows: usize,
    pub problems: Vec<String>,
}

/// Read every row of every table, as a reader of untrusted evidence would.
pub fn walk(db: &Database) -> Walk {
    let mut walk = Walk {
        problems: db.problems.clone(),
        ..Walk::default()
    };
    for table in &db.tables {
        let mut rows = db.rows(&table.name).unwrap();
        walk.rows += rows.by_ref().count();
        walk.problems.extend_from_slice(rows.problems());
    }
    walk
}
