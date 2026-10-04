//! Damage is reported, never a panic, and every walk ends: unusable
//! headers, checksum mismatches, cycles and cross-links between trees,
//! pages out of range, blank long value pages, truncated files, and
//! (property tests) arbitrary bytes and real files damaged anywhere.

mod support;

use ese::{Database, Error, FileType};
use support::Walk;

/// Catalog1.edb: 4 KiB pages, small enough to damage and walk often.
const SMALL: &str = "Catalog1.edb";
const SMALL_PAGE_SIZE: usize = 4096;
/// Page header and tag sizes of a small page, and its tag count field.
const PAGE_HEADER_SIZE: usize = 40;
const TAG_SIZE: usize = 4;
const TAG_COUNT_OFFSET: usize = 34;
const FLAGS_OFFSET: usize = 36;
/// The catalog's root page, a branch page in Catalog1.edb.
const CATALOG_ROOT: usize = 4;
/// Header field offsets.
const SIGNATURE: usize = 4;
const FILE_TYPE: usize = 12;
const PAGE_SIZE_FIELD: usize = 236;
const LAST_OBJECT_ID: usize = 212;
/// Page flags.
const LEAF: u32 = 0x02;
const SPACE_TREE: u32 = 0x20;
const LONG_VALUE: u32 = 0x80;

fn small() -> Vec<u8> {
    support::fixture(SMALL).to_vec()
}

fn page_offset(page: usize, page_size: usize) -> usize {
    (page + 1) * page_size
}

fn set_u32(file: &mut [u8], at: usize, value: u32) {
    file[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn u32_at(file: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(file[at..at + 4].try_into().unwrap())
}

/// Where the child page number of branch entry `tag` of small page `page`
/// is: the last four bytes of the entry.
fn child_pointer(file: &[u8], page: usize, tag: usize) -> usize {
    let start = page_offset(page, SMALL_PAGE_SIZE);
    let at = start + SMALL_PAGE_SIZE - TAG_SIZE * (tag + 1);
    let size = usize::from(u16::from_le_bytes([file[at], file[at + 1]]) & 0x1fff);
    let offset = usize::from(u16::from_le_bytes([file[at + 2], file[at + 3]]) & 0x1fff);
    start + PAGE_HEADER_SIZE + offset + size - 4
}

fn walk(file: &[u8]) -> Walk {
    support::walk(&Database::open(file).unwrap())
}

fn has_problem(walk: &Walk, text: &str) -> bool {
    walk.problems.iter().any(|problem| problem.contains(text))
}

#[test]
fn unusable_headers_are_errors() {
    let file = small();
    assert_eq!(
        Database::open(&file[..667]).err(),
        Some(Error::TooShort(667))
    );
    let mut damaged = file.clone();
    damaged[SIGNATURE] ^= 0xff;
    damaged[SMALL_PAGE_SIZE + SIGNATURE] ^= 0xff;
    assert_eq!(Database::open(&damaged).err(), Some(Error::NotEse));
    let mut damaged = file.clone();
    set_u32(&mut damaged, PAGE_SIZE_FIELD, 1000);
    assert_eq!(Database::open(&damaged).err(), Some(Error::PageSize(1000)));
    let mut damaged = file;
    set_u32(&mut damaged, FILE_TYPE, 1);
    assert_eq!(
        Database::open(&damaged).err(),
        Some(Error::NotADatabase(FileType::StreamingFile))
    );
}

#[test]
fn a_damaged_header_gives_way_to_its_shadow() {
    let mut file = small();
    file[SIGNATURE] ^= 0xff;
    let db = Database::open(&file).unwrap();
    assert!(db.header.from_shadow);
    let walk = support::walk(&db);
    assert!(
        has_problem(&walk, "read its shadow copy at offset 4096"),
        "{:?}",
        walk.problems
    );
    assert_eq!(walk.rows, 3625);
}

#[test]
fn checksum_mismatches_are_reported_and_the_page_still_read() {
    let mut file = small();
    file[LAST_OBJECT_ID] ^= 1;
    let catalog_byte = page_offset(CATALOG_ROOT, SMALL_PAGE_SIZE) + 100;
    file[catalog_byte] ^= 0x10;
    let walk = walk(&file);
    assert!(
        has_problem(&walk, "file header checksum mismatch"),
        "{:?}",
        walk.problems
    );
    assert!(
        has_problem(&walk, "page 4: XOR checksum mismatch"),
        "{:?}",
        walk.problems
    );
    assert_eq!(walk.rows, 3625);
}

#[test]
fn a_dirty_database_is_reported() {
    let db = Database::open(support::fixture("Windows.edb")).unwrap();
    assert!(
        db.problems[0].starts_with("dirty shutdown"),
        "{:?}",
        db.problems
    );
}

#[test]
fn a_tree_cycle_is_reported_and_ends() {
    let mut file = small();
    let pointer = child_pointer(&file, CATALOG_ROOT, 1);
    set_u32(&mut file, pointer, CATALOG_ROOT as u32);
    let walk = walk(&file);
    assert!(
        has_problem(&walk, "page 4 reached twice"),
        "{:?}",
        walk.problems
    );
}

#[test]
fn a_page_out_of_range_is_reported() {
    let mut file = small();
    let pointer = child_pointer(&file, CATALOG_ROOT, 1);
    set_u32(&mut file, pointer, 100_000);
    let walk = walk(&file);
    assert!(
        has_problem(&walk, "page 100000 is outside the database's"),
        "{:?}",
        walk.problems
    );
}

#[test]
fn a_page_claimed_twice_is_read_once() {
    let mut file = small();
    let first = child_pointer(&file, CATALOG_ROOT, 1);
    let leaf = u32_at(&file, first);
    let second = child_pointer(&file, CATALOG_ROOT, 2);
    set_u32(&mut file, second, leaf);
    let walk = walk(&file);
    assert!(
        has_problem(&walk, &format!("page {leaf} reached twice")),
        "{:?}",
        walk.problems
    );
}

#[test]
fn a_page_of_another_tree_is_reported() {
    let mut file = small();
    let string_root = Database::open(&file)
        .unwrap()
        .table("string")
        .unwrap()
        .root_page;
    let pointer = child_pointer(&file, CATALOG_ROOT, 1);
    set_u32(&mut file, pointer, string_root);
    let walk = walk(&file);
    let problem = format!("page {string_root} belongs to object");
    assert!(has_problem(&walk, &problem), "{:?}", walk.problems);
}

#[test]
fn blank_long_value_pages_lose_only_their_values() {
    let mut file = support::fixture("WebCacheV01.dat").to_vec();
    let page_size = 32768;
    let full = walk(&file);
    let long_value_leaf = (1..file.len() / page_size - 1)
        .find(|&page| {
            let flags = u32_at(&file, page_offset(page, page_size) + FLAGS_OFFSET);
            flags & LONG_VALUE != 0 && flags & LEAF != 0 && flags & SPACE_TREE == 0
        })
        .unwrap();
    let start = page_offset(long_value_leaf, page_size);
    file[start..start + page_size].fill(0);
    let walk = walk(&file);
    assert!(
        has_problem(&walk, &format!("page {long_value_leaf} is blank")),
        "{:?}",
        walk.problems
    );
    assert!(
        has_problem(&walk, "not in the long value tree"),
        "{:?}",
        walk.problems
    );
    assert_eq!(walk.rows, full.rows);
}

#[test]
fn a_truncated_file_reads_what_is_left() {
    let file = small();
    let full = walk(&file);
    let whole_pages = file.len() / 8 / SMALL_PAGE_SIZE * SMALL_PAGE_SIZE;
    let walk = walk(&file[..whole_pages + 100]);
    assert!(
        has_problem(&walk, "100 bytes after the last whole page"),
        "{:?}",
        walk.problems
    );
    assert!(
        has_problem(&walk, "is outside the database's"),
        "{:?}",
        walk.problems
    );
    assert!(
        walk.rows > 0 && walk.rows < full.rows,
        "{} of {}",
        walk.rows,
        full.rows
    );
}

#[test]
fn a_page_claiming_too_many_tags_is_reported() {
    let mut file = small();
    let at = page_offset(CATALOG_ROOT, SMALL_PAGE_SIZE) + TAG_COUNT_OFFSET;
    file[at..at + 2].copy_from_slice(&5000u16.to_le_bytes());
    let walk = walk(&file);
    assert!(
        has_problem(&walk, "page 4: 5000 tags declared"),
        "{:?}",
        walk.problems
    );
}

mod properties {
    use proptest::prelude::*;

    use super::support::{self, Walk};
    use ese::Database;

    /// At most one row per four bytes of pages (the size of a tag).
    fn plausible(walk: &Walk, db: &Database) -> bool {
        let most = db.page_count() as usize * db.header.page_size as usize / super::TAG_SIZE;
        walk.rows <= most
    }

    fn damage(mut file: Vec<u8>, flips: &[(usize, u8)], cut: usize) -> Vec<u8> {
        if file.is_empty() {
            return file;
        }
        for &(at, byte) in flips {
            let len = file.len();
            file[at % len] = byte;
        }
        file.truncate(cut);
        file
    }

    proptest! {
        /// Any bytes: read or refused, never a panic.
        #[test]
        fn arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..8_192)) {
            if let Ok(db) = Database::open(&data) {
                support::walk(&db);
            }
        }

        /// Arbitrary small pages behind a real header and its shadow.
        #[test]
        fn arbitrary_small_pages(pages in proptest::collection::vec(any::<u8>(), 0..40_960)) {
            let mut file = support::fixture(super::SMALL)[..2 * super::SMALL_PAGE_SIZE].to_vec();
            file.extend(pages);
            let db = Database::open(&file).unwrap();
            let walk = support::walk(&db);
            prop_assert!(plausible(&walk, &db));
        }

        /// An arbitrary catalog root page (and after) on 32 KiB pages, the
        /// extended format, behind the real first pages.
        #[test]
        fn arbitrary_extended_pages(pages in proptest::collection::vec(any::<u8>(), 0..65_536)) {
            let mut file = support::fixture("WebCacheV01.dat")[..5 * 32_768].to_vec();
            file.extend(pages);
            let db = Database::open(&file).unwrap();
            let walk = support::walk(&db);
            prop_assert!(plausible(&walk, &db));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// A real small-page database damaged anywhere and cut anywhere.
        #[test]
        fn damaged_small_database(
            flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..40),
            cut in 0usize..3_200_000,
        ) {
            let file = damage(support::fixture(super::SMALL).to_vec(), &flips, cut);
            if let Ok(db) = Database::open(&file) {
                let walk = support::walk(&db);
                prop_assert!(plausible(&walk, &db));
            }
        }

        /// Real extended-format databases, with long values and compressed
        /// values, damaged anywhere in their first 2 MB and cut there.
        #[test]
        fn damaged_extended_database(
            fixture in prop::sample::select(vec!["WebCacheV01.dat", "PartitionsEx-WebCacheV01.dat", "Windows.edb"]),
            flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..40),
            cut in 0usize..2_000_000,
        ) {
            let file = damage(support::fixture(fixture)[..cut].to_vec(), &flips, cut);
            if let Ok(db) = Database::open(&file) {
                let walk = support::walk(&db);
                prop_assert!(plausible(&walk, &db));
            }
        }
    }
}
