//! SIDR's test `Windows.edb` (Apache-2.0, a Windows 10 search index,
//! downloaded by `tests/fetch-sidr.sh`; skipped without it): Windows 10
//! compresses long values without flagging them in the record, and every
//! one reads whole.

#[test]
fn windows_10_compressed_long_values() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/sidr/Windows.edb"
    );
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    let db = ese::Database::open(&data).unwrap();
    let mut rows = db.rows("SystemIndex_PropertyStore").unwrap();
    assert_eq!(rows.by_ref().count(), 1182);
    assert_eq!(rows.problems(), &[] as &[String]);
}
