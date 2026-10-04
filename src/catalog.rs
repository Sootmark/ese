//! The catalog (`MSysObjects`), the tree at page 4: one record per table,
//! then one per column, index and long value tree of that table, keyed by
//! the table's object identifier.
//!
//! Its records are read with the catalog's own column layout, fixed here;
//! the catalog also describes itself, so `MSysObjects` reads as any table.

use crate::btree::Cursor;
use crate::bytes::u16_at;
use crate::pager::Pages;
use crate::record::Record;
use crate::schema::{Column, ColumnKind, ColumnType, Index, Table};
use crate::text;

/// The catalog's root page.
const CATALOG_ROOT_PAGE: u32 = 4;
/// Where the first fixed column of a record starts.
const FIRST_FIXED_OFFSET: u16 = 4;
/// Bytes of a key column entry in an index's `KeyFldIDs`, and where in it
/// the column identifier is.
const KEY_COLUMN_SIZE: usize = 4;
const KEY_COLUMN_ID_OFFSET: usize = 2;

/// What a catalog record defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Table,
    Column,
    Index,
    LongValue,
    Callback,
    Other(i16),
}

impl Kind {
    fn from_raw(raw: i16) -> Self {
        match raw {
            1 => Self::Table,
            2 => Self::Column,
            3 => Self::Index,
            4 => Self::LongValue,
            5 => Self::Callback,
            other => Self::Other(other),
        }
    }
}

/// The catalog's columns: (identifier, name, type).
const CATALOG_COLUMNS: [(u32, &str, ColumnType); 17] = [
    (1, "ObjidTable", ColumnType::Long),
    (2, "Type", ColumnType::Short),
    (3, "Id", ColumnType::Long),
    (4, "ColtypOrPgnoFDP", ColumnType::Long),
    (5, "SpaceUsage", ColumnType::Long),
    (6, "Flags", ColumnType::Long),
    (7, "PagesOrLocale", ColumnType::Long),
    (8, "RootFlag", ColumnType::Bit),
    (9, "RecordOffset", ColumnType::Short),
    (10, "LCMapFlags", ColumnType::Long),
    (11, "KeyMost", ColumnType::UnsignedShort),
    (12, "LVChunkMax", ColumnType::Long),
    (128, "Name", ColumnType::Text),
    (129, "Stats", ColumnType::Binary),
    (130, "TemplateTable", ColumnType::Text),
    (131, "DefaultValue", ColumnType::Binary),
    (132, "KeyFldIDs", ColumnType::Binary),
];

/// Positions in [`CATALOG_COLUMNS`] of the columns read.
const OBJID_TABLE: usize = 0;
const TYPE: usize = 1;
const ID: usize = 2;
const COLTYP_OR_PGNO_FDP: usize = 3;
const SPACE_USAGE: usize = 4;
const FLAGS: usize = 5;
const PAGES_OR_LOCALE: usize = 6;
const RECORD_OFFSET: usize = 8;
const NAME: usize = 12;
const TEMPLATE_TABLE: usize = 14;
const DEFAULT_VALUE: usize = 15;
const KEY_FLD_IDS: usize = 16;

/// The catalog's own column layout, to read its records with.
struct Layout {
    columns: Vec<Column>,
}

impl Layout {
    fn new() -> Self {
        let mut offset = FIRST_FIXED_OFFSET;
        let columns = CATALOG_COLUMNS
            .iter()
            .map(|&(id, name, column_type)| {
                let mut column = Column {
                    id,
                    name: name.to_owned(),
                    column_type,
                    codepage: text::WINDOWS_1252,
                    flags: 0,
                    max_size: 0,
                    record_offset: 0,
                    default: None,
                    from_template: false,
                };
                if column.kind() == ColumnKind::Fixed {
                    column.record_offset = offset;
                    offset += column.fixed_size() as u16;
                }
                column
            })
            .collect();
        Self { columns }
    }
}

/// One catalog record.
#[derive(Debug)]
struct Entry {
    object_id: u32,
    kind: Kind,
    id: u32,
    /// The column type, or the root page of a tree.
    type_or_root: u32,
    space_usage: u32,
    flags: u32,
    /// The codepage of a column, the locale of an index.
    codepage_or_locale: u32,
    record_offset: u16,
    name: String,
    template: Option<String>,
    default: Option<Vec<u8>>,
    key_columns: Vec<u32>,
}

impl Entry {
    fn parse(data: &[u8], layout: &Layout, pages: &Pages) -> Result<Self, String> {
        // The catalog's tagged columns (callbacks, space hints) aren't read.
        let (record, _) = Record::parse(data, pages.tagged_format())?;
        let fixed = |position: usize| -> Result<u32, String> {
            let bytes = record.fixed(&layout.columns[position])?.unwrap_or_default();
            Ok(bytes
                .iter()
                .rev()
                .fold(0u32, |value, &byte| (value << 8) | u32::from(byte)))
        };
        let variable = |position: usize| {
            let id = layout.columns[position].id;
            record.variable(id).map(|bytes| bytes.map(<[u8]>::to_vec))
        };
        let text = |position| -> Result<Option<String>, String> {
            Ok(variable(position)?.map(|bytes| text::decode(&bytes, text::WINDOWS_1252)))
        };
        Ok(Self {
            object_id: fixed(OBJID_TABLE)?,
            kind: Kind::from_raw(fixed(TYPE)? as i16),
            id: fixed(ID)?,
            type_or_root: fixed(COLTYP_OR_PGNO_FDP)?,
            space_usage: fixed(SPACE_USAGE)?,
            flags: fixed(FLAGS)?,
            codepage_or_locale: fixed(PAGES_OR_LOCALE)?,
            record_offset: fixed(RECORD_OFFSET)? as u16,
            name: text(NAME)?.unwrap_or_default(),
            template: text(TEMPLATE_TABLE)?,
            default: variable(DEFAULT_VALUE)?,
            key_columns: key_columns(&variable(KEY_FLD_IDS)?.unwrap_or_default()),
        })
    }

    fn table(self) -> Table {
        Table {
            object_id: self.object_id,
            name: self.name,
            root_page: self.type_or_root,
            flags: self.flags,
            template: self.template,
            columns: Vec::new(),
            indexes: Vec::new(),
            long_value_root: None,
        }
    }

    fn column(self) -> Column {
        Column {
            id: self.id,
            name: self.name,
            column_type: ColumnType::from_raw(self.type_or_root),
            codepage: self.codepage_or_locale,
            flags: self.flags,
            max_size: self.space_usage,
            record_offset: self.record_offset,
            default: self.default,
            from_template: false,
        }
    }

    fn index(self) -> Index {
        Index {
            id: self.id,
            name: self.name,
            root_page: self.type_or_root,
            flags: self.flags,
            locale: self.codepage_or_locale,
            key_columns: self.key_columns,
        }
    }
}

/// The column identifiers of an index key (`KeyFldIDs`).
fn key_columns(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(KEY_COLUMN_SIZE)
        .filter_map(|entry| u16_at(entry, KEY_COLUMN_ID_OFFSET).map(u32::from))
        .collect()
}

/// Every table the catalog defines, in catalog order, and the damage met.
pub(crate) fn read(pages: &Pages) -> (Vec<Table>, Vec<String>) {
    let layout = Layout::new();
    let mut cursor = Cursor::new(pages, CATALOG_ROOT_PAGE);
    let mut tables: Vec<Table> = Vec::new();
    let mut problems = Vec::new();
    while let Some(found) = cursor.next_entry() {
        match Entry::parse(found.node.data, &layout, pages) {
            Ok(entry) => add(&mut tables, entry, &mut problems),
            Err(problem) => {
                problems.push(format!("catalog record on page {}: {problem}", found.page));
            }
        }
    }
    problems.extend(cursor.problems);
    for table in &mut tables {
        complete(table, &mut problems);
    }
    inherit_templates(&mut tables, &mut problems);
    (tables, problems)
}

/// Add a catalog entry: a new table, or a part of the table it names.
fn add(tables: &mut Vec<Table>, entry: Entry, problems: &mut Vec<String>) {
    if entry.kind == Kind::Table {
        tables.push(entry.table());
        return;
    }
    let Some(table) = tables
        .iter_mut()
        .rev()
        .find(|table| table.object_id == entry.object_id)
    else {
        problems.push(format!(
            "catalog: {:?} {} of table {}, which it doesn't define",
            entry.kind, entry.name, entry.object_id
        ));
        return;
    };
    match entry.kind {
        Kind::Column => table.columns.push(entry.column()),
        Kind::Index => table.indexes.push(entry.index()),
        Kind::LongValue => table.long_value_root = Some(entry.type_or_root),
        Kind::Table | Kind::Callback => {}
        Kind::Other(raw) => problems.push(format!(
            "catalog: {} of table {} has unknown type {raw}",
            entry.name, table.name
        )),
    }
}

/// Order a table's columns, place fixed columns the catalog gives no
/// usable offset, and report text the crate can't decode as such.
///
/// Fixed columns follow each other in identifier order. The catalog's
/// description of itself gives every one offset 4 (its layout is built into
/// ESE): an offset inside the previous column means the next free one.
fn complete(table: &mut Table, problems: &mut Vec<String>) {
    table.columns.sort_by_key(|column| column.id);
    let mut next_offset = FIRST_FIXED_OFFSET;
    for column in table
        .columns
        .iter_mut()
        .filter(|column| column.kind() == ColumnKind::Fixed)
    {
        if column.record_offset < next_offset {
            column.record_offset = next_offset;
        }
        next_offset = column
            .record_offset
            .saturating_add(column.fixed_size() as u16);
    }
    for column in &table.columns {
        if column.column_type.is_text() && !text::is_supported(column.codepage) {
            problems.push(format!(
                "table {}, column {}: codepage {} is read as Windows-1252",
                table.name, column.name, column.codepage
            ));
        }
    }
}

/// Put each template table's columns before those of the tables derived
/// from it.
fn inherit_templates(tables: &mut [Table], problems: &mut Vec<String>) {
    for index in 0..tables.len() {
        let Some(template_name) = tables[index].template.clone() else {
            continue;
        };
        let Some(template) = tables.iter().find(|table| table.name == template_name) else {
            problems.push(format!(
                "table {}: template table {template_name} not in the catalog",
                tables[index].name
            ));
            continue;
        };
        let inherited: Vec<Column> = template
            .columns
            .iter()
            .map(|column| Column {
                from_template: true,
                ..column.clone()
            })
            .collect();
        tables[index].columns.splice(0..0, inherited);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_layout_matches_its_own_description() {
        // MSysObjects describes ObjidTable at offset 4 and RootFlag at 30.
        let layout = Layout::new();
        assert_eq!(layout.columns[OBJID_TABLE].record_offset, 4);
        assert_eq!(layout.columns[7].record_offset, 30);
        assert_eq!(layout.columns[11].record_offset, 39);
    }

    #[test]
    fn key_columns_are_the_high_halves() {
        assert_eq!(key_columns(&[0, 0, 1, 0, 0, 0, 0x80, 0]), [1, 128]);
        assert_eq!(key_columns(&[0, 0, 0, 1]), [256]);
    }
}
