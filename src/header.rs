//! The database file header: the first page of the file, with a copy (the
//! shadow) in the second.

use crate::bytes::{array_at, u32_at, u64_at};
use crate::checksum;
use crate::Error;

/// The file signature, at offset 4.
pub(crate) const SIGNATURE: u32 = 0x89ab_cdef;
/// Bytes of the header that hold fields; the rest of its page is zeros.
pub(crate) const HEADER_SIZE: usize = 668;
/// The page sizes ESE writes.
const PAGE_SIZES: [u32; 5] = [2048, 4096, 8192, 16384, 32768];
/// The format version every known ESE database carries.
const KNOWN_FORMAT_VERSIONS: [u32; 2] = [0x620, 0x623];

/// Header field offsets.
const SIGNATURE_OFFSET: usize = 4;
const FORMAT_VERSION: usize = 8;
const FILE_TYPE: usize = 12;
const DATABASE_TIME: usize = 16;
const STATE: usize = 52;
const CONSISTENT_TIME: usize = 64;
const ATTACH_TIME: usize = 72;
const DETACH_TIME: usize = 88;
const LAST_OBJECT_ID: usize = 212;
const OS_MAJOR: usize = 216;
const OS_MINOR: usize = 220;
const OS_BUILD: usize = 224;
const OS_SERVICE_PACK: usize = 228;
const FORMAT_REVISION: usize = 232;
const PAGE_SIZE: usize = 236;
const REPAIR_COUNT: usize = 240;
const CREATION_FORMAT_VERSION: usize = 340;
const CREATION_FORMAT_REVISION: usize = 344;

/// What the file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// A database: pages of B+-trees.
    Database,
    /// A streaming file (`.stm`), raw data for an Exchange database.
    StreamingFile,
    /// A value this crate doesn't know.
    Other(u32),
}

impl FileType {
    fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::Database,
            1 => Self::StreamingFile,
            other => Self::Other(other),
        }
    }
}

/// How the database was left: only a clean shutdown means every change is
/// in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseState {
    /// Just created.
    JustCreated,
    /// In use, or not shut down cleanly: changes may still be in the
    /// transaction log files only.
    DirtyShutdown,
    /// Shut down cleanly: the file holds every committed change.
    CleanShutdown,
    /// Being upgraded to a newer format.
    BeingConverted,
    /// Detached by force (internal state, from Windows XP).
    ForceDetach,
    /// A value this crate doesn't know.
    Other(u32),
}

impl DatabaseState {
    fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Self::JustCreated,
            2 => Self::DirtyShutdown,
            3 => Self::CleanShutdown,
            4 => Self::BeingConverted,
            5 => Self::ForceDetach,
            other => Self::Other(other),
        }
    }
}

/// A date and time as ESE logs it (`JET_LOGTIME`): local fields, to the
/// second, years from 1900.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogTime {
    /// The year (1900 and up).
    pub year: u16,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day of the month, 1 to 31.
    pub day: u8,
    /// The hour, 0 to 23.
    pub hour: u8,
    /// The minute, 0 to 59.
    pub minute: u8,
    /// The second, 0 to 59.
    pub second: u8,
}

/// The base of [`LogTime`] years.
const LOG_TIME_EPOCH_YEAR: u16 = 1900;

impl LogTime {
    /// The log time at `at`, or `None` when unset (all zeros).
    fn at(header: &[u8], at: usize) -> Option<Self> {
        let [second, minute, hour, day, month, year, ..] = array_at::<8>(header, at)?;
        (month != 0).then(|| Self {
            year: LOG_TIME_EPOCH_YEAR + u16::from(year),
            month,
            day,
            hour,
            minute,
            second,
        })
    }
}

/// The database header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The format version (0x620 for every ESE in use).
    pub format_version: u32,
    /// The format revision: which features the database may use (0x11 and
    /// up: 16 and 32 KiB pages, compression).
    pub format_revision: u32,
    /// The format version the database was created with.
    pub creation_format_version: u32,
    /// The format revision the database was created with.
    pub creation_format_revision: u32,
    /// What the file holds.
    pub file_type: FileType,
    /// Bytes per page: 2048, 4096, 8192, 16384 or 32768.
    pub page_size: u32,
    /// How the database was left.
    pub state: DatabaseState,
    /// The database time (a counter of changes) when the header was written.
    pub database_time: u64,
    /// When the database was last shut down cleanly (unset when dirty).
    pub consistent_time: Option<LogTime>,
    /// When the database was last attached.
    pub attach_time: Option<LogTime>,
    /// When the database was last detached.
    pub detach_time: Option<LogTime>,
    /// The last object identifier the database handed out.
    pub last_object_id: u32,
    /// The Windows version (major, minor, build, service pack) that last
    /// updated the database's indexes.
    pub windows_version: (u32, u32, u32, u32),
    /// How many times the database was repaired.
    pub repair_count: u32,
    /// Whether the header was read from its shadow copy in the second page,
    /// the first being damaged.
    pub from_shadow: bool,
}

impl Header {
    /// The header of `file`: from its first page, else from the shadow copy
    /// in its second page. Damage short of unusable is added to `problems`.
    pub(crate) fn parse(file: &[u8], problems: &mut Vec<String>) -> Result<Self, Error> {
        if file.len() < HEADER_SIZE {
            return Err(Error::TooShort(file.len()));
        }
        let (at, from_shadow) = if has_signature(file, 0) {
            (0, false)
        } else {
            let shadow = shadow_offset(file).ok_or(Error::NotEse)?;
            problems.push(format!(
                "the file header is damaged: read its shadow copy at offset {shadow}"
            ));
            (shadow, true)
        };
        let header = &file[at..];
        let field = |offset| u32_at(header, offset).unwrap_or_default();
        let page_size = field(PAGE_SIZE);
        if !PAGE_SIZES.contains(&page_size) {
            return Err(Error::PageSize(page_size));
        }
        let file_type = FileType::from_raw(field(FILE_TYPE));
        if file_type != FileType::Database {
            return Err(Error::NotADatabase(file_type));
        }
        let parsed = Self {
            format_version: field(FORMAT_VERSION),
            format_revision: field(FORMAT_REVISION),
            creation_format_version: field(CREATION_FORMAT_VERSION),
            creation_format_revision: field(CREATION_FORMAT_REVISION),
            file_type,
            page_size,
            state: DatabaseState::from_raw(field(STATE)),
            database_time: u64_at(header, DATABASE_TIME).unwrap_or_default(),
            consistent_time: LogTime::at(header, CONSISTENT_TIME),
            attach_time: LogTime::at(header, ATTACH_TIME),
            detach_time: LogTime::at(header, DETACH_TIME),
            last_object_id: field(LAST_OBJECT_ID),
            windows_version: (
                field(OS_MAJOR),
                field(OS_MINOR),
                field(OS_BUILD),
                field(OS_SERVICE_PACK),
            ),
            repair_count: field(REPAIR_COUNT),
            from_shadow,
        };
        parsed.check(header, problems);
        Ok(parsed)
    }

    /// Report what makes the header, or the database it describes, less
    /// than trustworthy.
    fn check(&self, header: &[u8], problems: &mut Vec<String>) {
        let page = header.get(..self.page_size as usize).unwrap_or(header);
        let stored = u32_at(page, 0).unwrap_or_default();
        let computed = checksum::header(page);
        if stored != computed {
            problems.push(format!(
                "file header checksum mismatch: stored 0x{stored:08x}, computed 0x{computed:08x}"
            ));
        }
        if !KNOWN_FORMAT_VERSIONS.contains(&self.format_version) {
            problems.push(format!(
                "unknown format version 0x{:x}: read as 0x620",
                self.format_version
            ));
        }
        match self.state {
            DatabaseState::CleanShutdown => {}
            DatabaseState::DirtyShutdown => problems.push(
                "dirty shutdown: changes still only in the transaction log files are \
                 missing (logs are not replayed)"
                    .to_owned(),
            ),
            state => problems.push(format!(
                "database state {state:?}: it may lack changes still only in the \
                 transaction log files (logs are not replayed)"
            )),
        }
    }

    /// Whether pages use the extended format of 16 and 32 KiB pages: an
    /// 80-byte page header, 15-bit page tags, flags in the values.
    pub(crate) fn extended_pages(&self) -> bool {
        self.page_size >= 16384
    }
}

fn has_signature(file: &[u8], at: usize) -> bool {
    u32_at(file, at + SIGNATURE_OFFSET) == Some(SIGNATURE)
}

/// Where the shadow copy of the header is: one page in, its page size
/// matching that offset.
fn shadow_offset(file: &[u8]) -> Option<usize> {
    PAGE_SIZES
        .iter()
        .map(|&size| size as usize)
        .find(|&at| has_signature(file, at) && u32_at(file, at + PAGE_SIZE) == Some(at as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header page of `page_size` bytes in `state`, with a valid checksum.
    fn header_page(page_size: u32, state: u32) -> Vec<u8> {
        let mut page = vec![0; page_size as usize];
        let mut put =
            |at: usize, value: u32| page[at..at + 4].copy_from_slice(&value.to_le_bytes());
        put(SIGNATURE_OFFSET, SIGNATURE);
        put(FORMAT_VERSION, 0x620);
        put(FORMAT_REVISION, 0x14);
        put(PAGE_SIZE, page_size);
        put(STATE, state);
        let checksum = checksum::header(&page);
        page[..4].copy_from_slice(&checksum.to_le_bytes());
        page
    }

    #[test]
    fn a_clean_header_has_no_problems() {
        let mut problems = Vec::new();
        let header = Header::parse(&header_page(4096, 3), &mut problems).unwrap();
        assert_eq!(header.page_size, 4096);
        assert_eq!(header.format_revision, 0x14);
        assert_eq!(header.state, DatabaseState::CleanShutdown);
        assert!(!header.from_shadow);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn a_dirty_database_is_reported() {
        let mut problems = Vec::new();
        Header::parse(&header_page(4096, 2), &mut problems).unwrap();
        assert!(problems[0].starts_with("dirty shutdown"), "{problems:?}");
    }

    #[test]
    fn a_damaged_header_gives_way_to_its_shadow() {
        let mut file = header_page(8192, 3);
        file.extend(header_page(8192, 3));
        file[SIGNATURE_OFFSET] ^= 0xff;
        let mut problems = Vec::new();
        let header = Header::parse(&file, &mut problems).unwrap();
        assert!(header.from_shadow);
        assert!(problems[0].contains("shadow copy at offset 8192"));
    }

    #[test]
    fn a_checksum_mismatch_is_reported_not_fatal() {
        let mut page = header_page(4096, 3);
        page[LAST_OBJECT_ID] = 7;
        let mut problems = Vec::new();
        assert!(Header::parse(&page, &mut problems).is_ok());
        assert!(problems[0].starts_with("file header checksum mismatch"));
    }

    #[test]
    fn unusable_headers_are_errors() {
        let mut problems = Vec::new();
        assert_eq!(
            Header::parse(&[0; 100], &mut problems),
            Err(Error::TooShort(100))
        );
        assert_eq!(Header::parse(&[0; 4096], &mut problems), Err(Error::NotEse));
        let mut page = header_page(4096, 3);
        page[PAGE_SIZE..PAGE_SIZE + 4].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(
            Header::parse(&page, &mut problems),
            Err(Error::PageSize(1000))
        );
        let mut page = header_page(4096, 3);
        page[FILE_TYPE] = 1;
        assert_eq!(
            Header::parse(&page, &mut problems),
            Err(Error::NotADatabase(FileType::StreamingFile))
        );
    }

    #[test]
    fn log_times_count_years_from_1900() {
        let mut page = header_page(4096, 3);
        page[CONSISTENT_TIME..CONSISTENT_TIME + 6].copy_from_slice(&[5, 4, 3, 2, 11, 117]);
        let header = Header::parse(&page, &mut Vec::new()).unwrap();
        let time = header.consistent_time.unwrap();
        assert_eq!(
            (
                time.year,
                time.month,
                time.day,
                time.hour,
                time.minute,
                time.second
            ),
            (2017, 11, 2, 3, 4, 5)
        );
        assert_eq!(header.attach_time, None);
    }
}
