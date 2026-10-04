//! The values plaso's own tests expect from these databases
//! (`tests/parsers/esedb_plugins/` at the commit in the fixtures' NOTICE):
//! its SRUM, MSIE WebCache and File History plugins, read here through
//! this crate's rows.

mod support;

use ese::{Database, Row, Table, Value};

/// OLE automation dates count days from 1899-12-30, 25,569 days before
/// the Unix epoch.
const OLE_DAYS_BEFORE_UNIX: f64 = 25_569.0;
const SECONDS_PER_DAY: f64 = 86_400.0;

fn open(fixture: &str) -> Database<'static> {
    Database::open(support::fixture(fixture)).unwrap()
}

/// Every row of `table`, and the table.
fn rows<'db>(db: &'db Database, table: &str) -> (&'db Table, Vec<Row>) {
    let mut rows = db.rows(table).unwrap();
    let all: Vec<Row> = rows.by_ref().collect();
    assert_eq!(rows.problems(), &[] as &[String]);
    (db.table(table).unwrap(), all)
}

/// The row of `table` whose `column` holds the integer `key`.
fn row_where<'r>(table: &Table, rows: &'r [Row], column: &str, key: i64) -> &'r Row {
    rows.iter()
        .find(|row| row.get(table, column).and_then(Value::as_i64) == Some(key))
        .unwrap_or_else(|| panic!("no row of {} with {column} = {key}", table.name))
}

fn integer(table: &Table, row: &Row, column: &str) -> i64 {
    row.get(table, column).and_then(Value::as_i64).unwrap()
}

fn text<'r>(table: &Table, row: &'r Row, column: &str) -> &'r str {
    row.get(table, column).and_then(Value::as_text).unwrap()
}

fn bytes<'r>(table: &Table, row: &'r Row, column: &str) -> &'r [u8] {
    row.get(table, column).and_then(Value::as_bytes).unwrap()
}

/// An OLE automation date in Unix milliseconds.
fn ole_unix_millis(table: &Table, row: &Row, column: &str) -> i64 {
    let Some(Value::DateTime(days)) = row.get(table, column) else {
        panic!("{column} is not a date");
    };
    ((days - OLE_DAYS_BEFORE_UNIX) * SECONDS_PER_DAY * 1000.0).round() as i64
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// The `SruDbIdMapTable` entry `index` names, as stored.
fn srum_id<'r>(db: &Database, rows: &'r [Row], index: i64) -> &'r [u8] {
    let table = db.table("SruDbIdMapTable").unwrap();
    bytes(table, row_where(table, rows, "IdIndex", index), "IdBlob")
}

#[test]
fn srum_network_usage_application_usage_and_connectivity() {
    let db = open("SRUDB.dat");
    let (_, id_map) = rows(&db, "SruDbIdMapTable");

    let (usage, usage_rows) = rows(&db, "{973F5D5C-1D90-4944-BE8E-24B94231A174}");
    let row = row_where(usage, &usage_rows, "AutoIncId", 3495);
    assert_eq!(integer(usage, row, "BytesSent"), 2076);
    assert_eq!(integer(usage, row, "InterfaceLuid"), 1_689_399_632_855_040);
    assert_eq!(ole_unix_millis(usage, row, "TimeStamp"), 1_509_881_520_000); // 2017-11-05T11:32:00
    let app = srum_id(&db, &id_map, integer(usage, row, "AppId"));
    assert!(app.starts_with(&utf16("DiagTrack")));
    // S-1-5-18, as a binary SID.
    let user = srum_id(&db, &id_map, integer(usage, row, "UserId"));
    assert_eq!(user, [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0]);

    let (resources, resource_rows) = rows(&db, "{D10CA2FE-6FCF-4F6D-848E-B2E99266FA89}");
    let row = row_where(resources, &resource_rows, "AutoIncId", 22167);
    assert_eq!(
        ole_unix_millis(resources, row, "TimeStamp"),
        1_509_881_520_000
    );
    let app = srum_id(&db, &id_map, integer(resources, row, "AppId"));
    assert!(app.starts_with(&utf16("Memory Compression")));

    let (connectivity, connectivity_rows) = rows(&db, "{DD6636C4-8929-4683-974E-22C046A43763}");
    let row = row_where(connectivity, &connectivity_rows, "AutoIncId", 501);
    assert_eq!(integer(connectivity, row, "AppId"), 1);
    assert_eq!(
        ole_unix_millis(connectivity, row, "TimeStamp"),
        1_509_888_780_000
    ); // 13:33:00
       // 2017-11-05T10:30:48.1679714, a FILETIME.
    assert_eq!(
        integer(connectivity, row, "ConnectStartTime"),
        131_543_514_481_679_714
    );
}

#[test]
fn webcache_containers() {
    let db = open("WebCacheV01.dat");
    let (containers, container_rows) = rows(&db, "Containers");
    let row = row_where(containers, &container_rows, "ContainerId", 1);
    assert_eq!(text(containers, row, "Name"), "Content");
    assert_eq!(
        text(containers, row, "Directory"),
        r"C:\Users\test\AppData\Local\Microsoft\Windows\INetCache\IE\"
    );
    assert_eq!(integer(containers, row, "SetId"), 0);
    // 2014-05-12T07:30:25.4861987, a FILETIME.
    assert_eq!(
        integer(containers, row, "LastAccessTime"),
        130_443_534_254_861_987
    );
}

#[test]
fn webcache_partitions_ex_container_entry() {
    let db = open("PartitionsEx-WebCacheV01.dat");
    let (entries, entry_rows) = rows(&db, "Container_14");
    let row = row_where(entries, &entry_rows, "EntryId", 63);
    assert_eq!(
        text(entries, row, "Url"),
        "https://www.bing.com/rs/3R/kD/ic/878ca0cd/b83d57c0.svg"
    );
    assert_eq!(text(entries, row, "Filename"), "b83d57c0[1].svg");
    assert_eq!(integer(entries, row, "AccessCount"), 5);
    assert_eq!(integer(entries, row, "FileSize"), 726);
    assert_eq!(integer(entries, row, "CacheId"), 0);
    assert_eq!(integer(entries, row, "SyncCount"), 0);
    // 2019-03-20T17:22:14, a FILETIME.
    assert_eq!(
        integer(entries, row, "ModifiedTime"),
        131_975_761_340_000_000
    );
    let headers = String::from_utf8_lossy(bytes(entries, row, "ResponseHeaders"));
    for header in [
        "HTTP/1.1 200",
        "content-length: 726",
        "content-type: image/svg+xml",
        "x-cache: TCP_HIT",
        "x-msedge-ref: Ref A: 3CD5FCBC8EAD4E0A80FA41A62FBC8CCC Ref B: PRAEDGE0910 Ref C: 2019-12-16T20:55:28Z",
        "date: Mon, 16 Dec 2019 20:55:28 GMT",
    ] {
        assert!(headers.contains(header), "{header} not in {headers:?}");
    }
}

#[test]
fn webcache_cookie() {
    let db = open("WebCacheV01_cookies.dat");
    let (cookies, cookie_rows) = rows(&db, "CookieEntryEx_2");
    let row = row_where(cookies, &cookie_rows, "EntryId", 13);
    assert_eq!(bytes(cookies, row, "Name"), b"abid\0");
    assert_eq!(
        bytes(cookies, row, "Value"),
        b"fcc450d1-8674-1bd3-4074-a240cff5c5b1\0"
    );
    assert_eq!(text(cookies, row, "RDomain"), "com.associates-amazon");
    assert_eq!(integer(cookies, row, "Flags"), 0x8008_2401);
    assert_eq!(
        support::hex(bytes(cookies, row, "CookieHash")),
        "5b4342ed6e2b0ae16f7e2c4c"
    );
}

#[test]
fn file_history_namespace() {
    let db = open("Catalog1.edb");
    let (namespace, namespace_rows) = rows(&db, "namespace");
    // plaso makes one event per namespace row.
    assert_eq!(namespace_rows.len(), 1373);
    let row = row_where(namespace, &namespace_rows, "id", 356);
    assert_eq!(integer(namespace, row, "parentId"), 230);
    assert_eq!(integer(namespace, row, "fileAttrib"), 16);
    assert_eq!(integer(namespace, row, "usn"), 9_251_162_904);
    // 2013-09-02T20:02:25.6957745 and 2013-10-12T17:34:36.6885806.
    assert_eq!(
        integer(namespace, row, "fileCreated"),
        130_226_257_456_957_745
    );
    assert_eq!(
        integer(namespace, row, "fileModified"),
        130_260_728_766_885_806
    );
    // plaso names an entry by the string of the same identifier.
    let (strings, string_rows) = rows(&db, "string");
    let name = row_where(strings, &string_rows, "id", 356);
    assert_eq!(text(strings, name, "string"), r"?UP\Favorites\Links\Lenovo");
}
