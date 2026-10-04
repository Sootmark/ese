//! Tables, columns and indexes, as the catalog defines them.

/// A column's type (`JET_COLTYP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// Invalid (`JET_coltypNil`); its bytes read as binary.
    Nil,
    /// A boolean, one byte.
    Bit,
    /// An 8-bit unsigned integer.
    UnsignedByte,
    /// A 16-bit signed integer.
    Short,
    /// A 32-bit signed integer.
    Long,
    /// A 64-bit signed integer, in units of 1/10,000 of a currency unit.
    Currency,
    /// A 32-bit IEEE 754 float.
    IeeeSingle,
    /// A 64-bit IEEE 754 float.
    IeeeDouble,
    /// An OLE automation date: a 64-bit float of days since 1899-12-30.
    DateTime,
    /// Bytes, up to 255.
    Binary,
    /// Text, up to 255 bytes, in the column's codepage.
    Text,
    /// Bytes, up to 2 GiB, stored in the long value tree when large.
    LongBinary,
    /// Text, up to 2 GiB, stored in the long value tree when large.
    LongText,
    /// A super long value (obsolete): a reference into a streaming file,
    /// read as binary.
    Slv,
    /// A 32-bit unsigned integer.
    UnsignedLong,
    /// A 64-bit signed integer.
    LongLong,
    /// A GUID, 16 bytes.
    Guid,
    /// A 16-bit unsigned integer.
    UnsignedShort,
    /// A value this crate doesn't know; its bytes read as binary.
    Unknown(u32),
}

impl ColumnType {
    pub(crate) fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::Nil,
            1 => Self::Bit,
            2 => Self::UnsignedByte,
            3 => Self::Short,
            4 => Self::Long,
            5 => Self::Currency,
            6 => Self::IeeeSingle,
            7 => Self::IeeeDouble,
            8 => Self::DateTime,
            9 => Self::Binary,
            10 => Self::Text,
            11 => Self::LongBinary,
            12 => Self::LongText,
            13 => Self::Slv,
            14 => Self::UnsignedLong,
            15 => Self::LongLong,
            16 => Self::Guid,
            17 => Self::UnsignedShort,
            other => Self::Unknown(other),
        }
    }

    /// The size of every value of this type, for types of one size.
    #[must_use]
    pub fn size(self) -> Option<usize> {
        match self {
            Self::Bit | Self::UnsignedByte => Some(1),
            Self::Short | Self::UnsignedShort => Some(2),
            Self::Long | Self::IeeeSingle | Self::UnsignedLong => Some(4),
            Self::Currency | Self::IeeeDouble | Self::DateTime | Self::LongLong => Some(8),
            Self::Guid => Some(16),
            _ => None,
        }
    }

    /// Whether values of this type are text.
    #[must_use]
    pub fn is_text(self) -> bool {
        matches!(self, Self::Text | Self::LongText)
    }
}

/// Where a column's values are stored in a record, from its identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// Identifiers 1 to 127: at a fixed offset in every record.
    Fixed,
    /// Identifiers 128 to 255: up to 255 bytes, located by an offset array.
    Variable,
    /// Identifiers 256 and up: stored only when set, with their identifier,
    /// possibly several values, possibly in the long value tree.
    Tagged,
}

/// The first variable and tagged column identifiers.
pub(crate) const FIRST_VARIABLE: u32 = 128;
pub(crate) const FIRST_TAGGED: u32 = 256;

/// The column flag (`JET_bitColumnMultiValued`) of columns that may hold
/// several values.
const MULTI_VALUED: u32 = 0x400;

/// A column of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The identifier, unique in the table (in its template, for columns
    /// from a template table).
    pub id: u32,
    /// The name.
    pub name: String,
    /// The type.
    pub column_type: ColumnType,
    /// The codepage of text: 1200 (UTF-16 little-endian), 1252 (Windows
    /// Latin-1) or 20127 (ASCII).
    pub codepage: u32,
    /// The column flags (`JET_bitColumn…`): 0x1 fixed, 0x2 tagged, 0x4 not
    /// NULL, 0x10 autoincrement, 0x400 multi-valued, 0x800 escrow update…
    pub flags: u32,
    /// The most bytes a value takes (the size of a fixed text or binary
    /// column).
    pub max_size: u32,
    /// Where a fixed column's value is in a record.
    pub record_offset: u16,
    /// The default value, as stored.
    pub default: Option<Vec<u8>>,
    /// Whether the column comes from the table's template table.
    pub from_template: bool,
}

impl Column {
    /// Where the column's values are stored in a record.
    #[must_use]
    pub fn kind(&self) -> ColumnKind {
        match self.id {
            id if id < FIRST_VARIABLE => ColumnKind::Fixed,
            id if id < FIRST_TAGGED => ColumnKind::Variable,
            _ => ColumnKind::Tagged,
        }
    }

    /// Whether the column may hold several values.
    #[must_use]
    pub fn is_multi_valued(&self) -> bool {
        self.flags & MULTI_VALUED != 0
    }

    /// The bytes a fixed column's value takes in a record.
    pub(crate) fn fixed_size(&self) -> usize {
        self.column_type.size().unwrap_or(self.max_size as usize)
    }
}

/// An index of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// The identifier.
    pub id: u32,
    /// The name.
    pub name: String,
    /// The root page of the index's tree (the table's own for its primary
    /// index).
    pub root_page: u32,
    /// The index flags (`JET_bitIndex…`): 0x1 unique, 0x2 primary, …
    pub flags: u32,
    /// The locale identifier keys are normalised with.
    pub locale: u32,
    /// The identifiers of the key's columns, most significant first.
    pub key_columns: Vec<u32>,
}

/// A table: its columns, indexes and trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The object identifier.
    pub object_id: u32,
    /// The name.
    pub name: String,
    /// The root page of the table's tree of records.
    pub root_page: u32,
    /// The table flags (`JET_bitTableCreate…`): 0x1 fixed DDL, 0x2 template.
    pub flags: u32,
    /// The template table the table derives from, whose columns come first
    /// in [`Table::columns`].
    pub template: Option<String>,
    /// The columns, in identifier order (a template's first).
    pub columns: Vec<Column>,
    /// The indexes.
    pub indexes: Vec<Index>,
    /// The root page of the table's long value tree, if it has one.
    pub long_value_root: Option<u32>,
}

impl Table {
    /// The position in [`Table::columns`] (and in a row's values) of the
    /// column named `name` (ignoring ASCII case, as ESE does).
    #[must_use]
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
    }

    /// The column named `name` (ignoring ASCII case, as ESE does).
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.get(self.column_index(name)?)
    }

    /// The columns' names, in order.
    #[must_use]
    pub fn column_names(&self) -> Vec<&str> {
        self.columns
            .iter()
            .map(|column| column.name.as_str())
            .collect()
    }
}
