//! Microsoft ESE ("JET Blue") database files, read without ESE: the
//! System Resource Usage Monitor (`SRUDB.dat`), Internet Explorer and Edge
//! `WebCacheV01.dat`, Windows Search `Windows.edb`, Active Directory
//! `ntds.dit`, File History catalogs and the rest. Read from Joachim Metz's
//! [format documentation] and Microsoft's [MS-XCA] for compressed values.
//!
//! Read-only and dependency-free, for evidence: nothing is written, the
//! transaction logs are not replayed, and the file is borrowed, not copied.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let file = std::fs::read("SRUDB.dat")?;
//! let db = ese::Database::open(&file)?;
//! for table in &db.tables {
//!     println!("{} (root page {}): {:?}", table.name, table.root_page, table.column_names());
//! }
//! let table = db.table("SruDbIdMapTable").ok_or("no such table")?;
//! let mut rows = db.rows("SruDbIdMapTable")?;
//! for row in &mut rows {
//!     println!("{:?} {:?}", row.get(table, "IdIndex"), row.get(table, "IdBlob"));
//! }
//! for problem in db.problems.iter().chain(rows.problems()) {
//!     eprintln!("{problem}");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Damage is reported, never a panic: only a file that isn't an ESE
//! database, or whose header can't be used, is an error. Checksum
//! mismatches, a dirty shutdown, pages out of range or past the end of a
//! truncated file, cycles and cross-links between trees, records, values
//! and long values that don't fit are listed in `problems` and skipped.
//! Every page is visited at most once per walk, so every walk ends and
//! memory stays proportional to the input.
//!
//! [format documentation]: https://github.com/libyal/libesedb/blob/main/documentation/Extensible%20Storage%20Engine%20(ESE)%20Database%20File%20(EDB)%20format.asciidoc
//! [MS-XCA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/

mod btree;
mod bytes;
mod catalog;
mod checksum;
mod compression;
mod header;
mod long_value;
mod page;
mod pager;
mod record;
mod rows;
mod schema;
mod text;
mod value;
mod xpress;

pub use header::{DatabaseState, FileType, Header, LogTime};
pub use rows::{Row, Rows};
pub use schema::{Column, ColumnKind, ColumnType, Index, Table};
pub use value::Value;

use pager::Pages;

/// This crate's version, for records of what parsed them.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Why a database, or a table in it, can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Shorter than the 668-byte header (this many bytes).
    TooShort(usize),
    /// No ESE signature, in the header or its shadow copy.
    NotEse,
    /// A page size other than 2, 4, 8, 16 or 32 KiB.
    PageSize(u32),
    /// An ESE file that holds no database (a streaming file).
    NotADatabase(FileType),
    /// No table of this name.
    NoSuchTable(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort(size) => write!(f, "{size} bytes, too short for an ESE header"),
            Self::NotEse => f.write_str("not an ESE database (no signature)"),
            Self::PageSize(size) => write!(f, "invalid page size {size}"),
            Self::NotADatabase(kind) => write!(f, "an ESE file of type {kind:?}, not a database"),
            Self::NoSuchTable(name) => write!(f, "no table named {name}"),
        }
    }
}

impl std::error::Error for Error {}

/// An open database: its header and catalog, read on opening; rows are
/// read on demand.
pub struct Database<'a> {
    /// The file header.
    pub header: Header,
    /// The tables the catalog defines, in catalog order (by object
    /// identifier), the catalog itself (`MSysObjects`) first.
    pub tables: Vec<Table>,
    /// Damage met opening the database and reading its catalog.
    pub problems: Vec<String>,
    pages: Pages<'a>,
}

impl<'a> Database<'a> {
    /// Open a database file.
    ///
    /// # Errors
    /// When it isn't an ESE database or its header can't be used.
    pub fn open(file: &'a [u8]) -> Result<Self, Error> {
        let mut problems = Vec::new();
        let header = Header::parse(file, &mut problems)?;
        let pages = Pages::new(file, &header);
        let trailing = pages.trailing_bytes();
        if trailing > 0 {
            problems.push(format!("{trailing} bytes after the last whole page"));
        }
        let (tables, catalog_problems) = catalog::read(&pages);
        problems.extend(catalog_problems);
        Ok(Self {
            header,
            tables,
            problems,
            pages,
        })
    }

    /// Pages in the file after the header and its shadow.
    #[must_use]
    pub fn page_count(&self) -> u32 {
        self.pages.count()
    }

    /// The table named `name` (ignoring ASCII case, as ESE does).
    #[must_use]
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables
            .iter()
            .find(|table| table.name.eq_ignore_ascii_case(name))
    }

    /// The rows of table `name`, in key order, read as the iterator
    /// advances.
    ///
    /// # Errors
    /// When there is no such table.
    pub fn rows(&self, name: &str) -> Result<Rows<'_>, Error> {
        let table = self
            .table(name)
            .ok_or_else(|| Error::NoSuchTable(name.to_owned()))?;
        Ok(Rows::new(&self.pages, table))
    }
}
