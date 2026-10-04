# ese

Microsoft ESE ("JET Blue") database files, read without ESE: the System Resource Usage Monitor's `SRUDB.dat`, Internet Explorer and Edge `WebCacheV01.dat`, Windows Search `Windows.edb`, Active Directory `ntds.dit`, File History catalogs and the rest. Read from Joachim Metz's [format documentation](https://github.com/libyal/libesedb/blob/main/documentation/Extensible%20Storage%20Engine%20(ESE)%20Database%20File%20(EDB)%20format.asciidoc) and, for compressed values, Microsoft's [MS-XCA](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/). Read-only, no dependencies, and the file is borrowed, not copied.

```toml
[dependencies]
sootmark-ese = "0.1"
```

```rust
let file = std::fs::read("SRUDB.dat")?;
let db = ese::Database::open(&file)?;
for table in &db.tables {
    println!("{} (root page {}): {:?}", table.name, table.root_page, table.column_names());
}
let table = db.table("SruDbIdMapTable").ok_or("no such table")?;
let mut rows = db.rows("SruDbIdMapTable")?;
for row in &mut rows {
    println!("{:?} {:?}", row.get(table, "IdIndex"), row.get(table, "IdBlob"));
}
for problem in db.problems.iter().chain(rows.problems()) {
    eprintln!("{problem}");
}
```

## What you get

- `Database::open(bytes)`: the header (format version and revision, page size from 2 to 32 KiB, database state, database time, when it was last shut down cleanly, attached and detached, the Windows version, repair count), from its shadow copy when the first is damaged; and the catalog (`MSysObjects`): every table with its columns (identifier, name, type, codepage, flags, size, record offset, default value), indexes (name, root page, flags, locale, key columns), long value tree and template table.
- `db.rows(table)`: an iterator over the rows in key order (the primary index's), each with the leaf page holding it and one `Value` per column: `Null`, `Bool`, `U8`, `I16`, `U16`, `I32`, `U32`, `I64`, `Currency`, `F32`, `F64`, `DateTime` (an OLE automation date, in days), `Guid`, `Text`, `Binary`, `MultiValue`. Rows are read as the iterator advances, never all at once; names compare ignoring ASCII case, as in ESE.
- Pages of both formats: the 40-byte header, and from Windows 7 the 80-byte header of 16 and 32 KiB pages with 15-bit tags and entry flags in the entries; page flags; the tag array; keys sharing the page's prefix; B+-trees walked from the root through branch pages; deleted (defunct) entries skipped.
- Checksums verified on every page read, in the three formats: the original XOR, XOR and ECC (Exchange 2003 SP1, Windows Vista), and XOR and ECC per quarter of 16 and 32 KiB pages. A mismatch is reported; the page is still read.
- Records: fixed columns and their NULL bitmap, variable columns, tagged columns in both layouts (from format revision 9 an offset array, the value's flags byte always present on large pages; before it one column after another), multi-values (offset arrays, long values among them, the two-value form), long values (4 or 8-byte identifiers, segments joined in order), and compressed values: 7-bit ASCII, 7-bit Unicode and Xpress (plain LZ77). Text in UTF-16, Windows-1252 or ASCII, the terminating NULs applications store dropped, 8-bit text stored in a UTF-16 column (as Windows Search does) recognised by its odd size.

## Damage is contained

Evidence is hostile. Only a file that isn't an ESE database, or whose header can't be used (no signature in the header or its shadow, a page size ESE doesn't write, a streaming file), is an error. The rest is listed in `problems` and skipped: a dirty shutdown (the transaction logs are not replayed, so the latest changes may be missing), checksum mismatches, pages out of range or past the end of a truncated file, blank pages, cycles in a tree and pages claimed twice (every page is visited at most once per walk), pages of another tree or of a space tree, tags and entries outside their page, records whose header, offsets or values don't fit, values of the wrong size, multi-values and compressed data that don't decode, long values missing or cut short (the bytes before the damage are kept), catalog entries of tables it doesn't define. Nothing is allocated beyond what the input holds, and every walk ends.

## Not yet

- Replaying the transaction logs (`.log`, `.jrs`): a database left dirty, as one copied from a running system is, may lack changes still only in its logs. It is reported.
- Recovery of deleted records (defunct entries, free space, the space trees), and reading index trees (index definitions are read; rows come in primary key order).
- Xpress9, Xpress10 and scrubbed compressed values, reported as unsupported per value; streaming files (`.stm`) and the obsolete SLV columns (their references read as binary).
- Codepages other than 1200, 1252 and 20127 (read as Windows-1252, reported per column).
- Default values aren't put in absent columns (they read NULL, as with libesedb; `Column::default` holds the default).
- Template tables and the tagged layout before format revision 9 follow the documentation, but no openly licensed database exercises them.

## How it's checked

| Check | Result |
|---|---|
| The six ESE databases in [plaso](https://github.com/log2timeline/plaso)'s test data (Apache-2.0, `tests/fixtures/plaso/`): 4 KiB pages (SRUM, a File History catalog) and 32 KiB extended pages at format revisions 0x11, 0x14 and 0x50 (Windows Search, three WebCache databases), clean and dirty, with long values, 7-bit compressed values and compressed multi-values; against libesedb 20240420's reading of every value of every table (its Python binding and `esedbexport`, `tests/oracle/gen.sh`) | 159 tables, 51,041 rows, 1,109,850 values: all match, every page checksum verifies. Two differences are by design and checked: 85,126 NULL fixed values for which libesedb, ignoring the record's NULL bitmap, returns the column's bytes (84,805 the 0x2a filler ESE writes, 321 a priority Windows Search cleared), and 35 binary values in 7-bit Unicode that `esedbexport` decodes one byte per unit (ESE restores two) |
| The values plaso's own tests expect from its SRUM, MSIE WebCache and File History plugins on the same files | match |
| Xpress: the examples of [MS-XCA], and streams checked with libfwnt's decoder (`tests/oracle/xpress.py`) | decode |
| Headers unusable or damaged (the shadow copy is read), checksum mismatches, tree cycles, pages claimed twice, out of range or of another tree, blank long value pages, truncated files | each reported, every walk ends |
| Property tests: arbitrary bytes, arbitrary pages behind a real header (small and extended pages), real databases damaged and cut anywhere | no panic, row counts bounded by the input |

## Licence

MIT or Apache-2.0, at your option. The test databases are plaso's (Apache-2.0, see `tests/fixtures/plaso/NOTICE`); libesedb and libfwnt (LGPL) are only run to write the expected values, never included.
