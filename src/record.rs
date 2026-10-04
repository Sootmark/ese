//! The record format ("data definitions"): a 4-byte header, the fixed
//! columns and a bitmap of the NULL ones, the variable columns' end offsets
//! then their data, and the tagged columns.
//!
//! The header holds the last fixed and last variable column identifiers the
//! record stores (later ones are absent) and where the variable offsets
//! start. A variable column's offset has its high bit set when NULL. The
//! tagged columns, from format revision 9, are an array of (identifier,
//! offset) pairs then their data; a value starts with a flags byte when its
//! offset says so, always on 16 and 32 KiB pages. Before revision 9 each
//! tagged column is stored whole: identifier, size, flags byte, data.

use crate::bytes::{slice_at, u16_at, u8_at};
use crate::schema::{Column, FIRST_VARIABLE};

/// Bytes of the record header.
const HEADER_SIZE: usize = 4;
/// A variable column offset with this bit set is NULL.
const VARIABLE_NULL: u16 = 0x8000;
const VARIABLE_OFFSET_MASK: u16 = 0x7fff;
/// Bytes of a variable column offset, and of a tagged column's entry.
const VARIABLE_OFFSET_SIZE: usize = 2;
const TAGGED_ENTRY_SIZE: usize = 4;
/// Tagged column offset bits: the column comes from the template table;
/// the value starts with a flags byte (small pages).
const TAGGED_DERIVED: u16 = 0x8000;
const TAGGED_HAS_FLAGS: u16 = 0x4000;
const TAGGED_OFFSET_MASK: u16 = 0x3fff;
const EXTENDED_TAGGED_OFFSET_MASK: u16 = 0x7fff;
/// Bytes of an entry of the linear (pre-revision 9) tagged format before
/// its data: identifier, size, flags.
const LINEAR_TAGGED_HEADER_SIZE: usize = 5;

/// How the tagged columns are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaggedFormat {
    /// Before format revision 9: each column whole, one after the other.
    Linear,
    /// An offset array then the data; `extended` on 16 and 32 KiB pages,
    /// where offsets take 15 bits and every value has its flags byte.
    Offsets { extended: bool },
}

/// How a tagged value is stored, from its flags byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TaggedFlags(pub(crate) u8);

impl TaggedFlags {
    const COMPRESSED: u8 = 0x02;
    const LONG_VALUE: u8 = 0x04;
    const MULTI_VALUE: u8 = 0x08;
    const TWO_VALUES: u8 = 0x10;

    /// The value (or its first value, when several) is compressed.
    pub(crate) fn is_compressed(self) -> bool {
        self.0 & Self::COMPRESSED != 0
    }

    /// The data is the identifier of a long value in the long value tree.
    pub(crate) fn is_long_value(self) -> bool {
        self.0 & Self::LONG_VALUE != 0
    }

    /// The data holds several values, located by an offset array.
    pub(crate) fn is_multi_value(self) -> bool {
        self.0 & Self::MULTI_VALUE != 0
    }

    /// The data holds two values: a byte with the size of the first, then
    /// the first, then the second.
    pub(crate) fn is_two_values(self) -> bool {
        self.0 & Self::TWO_VALUES != 0
    }
}

/// A tagged column's stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tagged<'a> {
    pub(crate) id: u32,
    /// Whether the column is the template table's.
    pub(crate) derived: bool,
    pub(crate) flags: TaggedFlags,
    pub(crate) data: &'a [u8],
}

/// A column's stored value: NULL (or absent), or its bytes.
pub(crate) type Stored<'a> = Result<Option<&'a [u8]>, String>;

/// A record, located but not yet decoded.
#[derive(Debug)]
pub(crate) struct Record<'a> {
    bytes: &'a [u8],
    last_fixed: u32,
    last_variable: u32,
    /// Where the variable column offsets start (the fixed data ends).
    variable_offsets: usize,
    /// The tagged columns, in identifier order.
    tagged: Vec<Tagged<'a>>,
}

impl<'a> Record<'a> {
    /// Locate the parts of `bytes`. Damage in the tagged columns keeps the
    /// ones before it and is returned as the second value.
    pub(crate) fn parse(
        bytes: &'a [u8],
        format: TaggedFormat,
    ) -> Result<(Self, Option<String>), String> {
        let header = |at| u8_at(bytes, at).ok_or("record header cut short");
        let last_fixed = u32::from(header(0)?);
        let last_variable = u32::from(header(1)?);
        let variable_offsets = usize::from(u16_at(bytes, 2).ok_or("record header cut short")?);
        let mut record = Self {
            bytes,
            last_fixed,
            last_variable,
            variable_offsets,
            tagged: Vec::new(),
        };
        let bitmap_start = record.null_bitmap_start().ok_or_else(|| {
            format!(
                "record header places the variable columns at {variable_offsets}, outside its {} bytes",
                bytes.len()
            )
        })?;
        if bitmap_start < HEADER_SIZE {
            return Err(format!(
                "record header leaves no room for {last_fixed} fixed columns"
            ));
        }
        let tagged_start = record.tagged_start()?;
        let area = &bytes[tagged_start..];
        let (tagged, problem) = match format {
            TaggedFormat::Linear => parse_linear(area),
            TaggedFormat::Offsets { extended } => parse_offsets(area, extended),
        };
        record.tagged = tagged;
        Ok((record, problem))
    }

    /// Variable columns the record stores.
    fn variable_count(&self) -> usize {
        self.last_variable.saturating_sub(FIRST_VARIABLE - 1) as usize
    }

    /// Where the NULL bitmap of the fixed columns starts (one bit per
    /// column, set when NULL), if the header's offsets are consistent.
    fn null_bitmap_start(&self) -> Option<usize> {
        let bitmap_size = self.last_fixed.div_ceil(8) as usize;
        self.variable_offsets
            .checked_sub(bitmap_size)
            .filter(|_| self.variable_offsets <= self.bytes.len())
    }

    /// Where the variable columns' data starts.
    fn variable_data(&self) -> usize {
        self.variable_offsets + VARIABLE_OFFSET_SIZE * self.variable_count()
    }

    /// The raw offset of variable column `index` (from 0).
    fn variable_offset(&self, index: usize) -> Option<u16> {
        u16_at(
            self.bytes,
            self.variable_offsets + VARIABLE_OFFSET_SIZE * index,
        )
    }

    fn tagged_start(&self) -> Result<usize, String> {
        let end = match self.variable_count().checked_sub(1) {
            None => 0,
            Some(last) => {
                let raw = self
                    .variable_offset(last)
                    .ok_or("variable column offsets cut short")?;
                usize::from(raw & VARIABLE_OFFSET_MASK)
            }
        };
        let start = self.variable_data() + end;
        if start > self.bytes.len() {
            return Err(format!(
                "variable columns end at {start}, past the record's {} bytes",
                self.bytes.len()
            ));
        }
        Ok(start)
    }

    /// Fixed column `column`'s value: absent when after the last the record
    /// stores.
    pub(crate) fn fixed(&self, column: &Column) -> Stored<'a> {
        if column.id > self.last_fixed || column.id == 0 {
            return Ok(None);
        }
        let bitmap_start = self.null_bitmap_start().unwrap_or_default();
        let bit = (column.id - 1) as usize;
        let null_byte = u8_at(self.bytes, bitmap_start + bit / 8).unwrap_or_default();
        if null_byte >> (bit % 8) & 1 == 1 {
            return Ok(None);
        }
        let start = usize::from(column.record_offset);
        let size = column.fixed_size();
        slice_at(self.bytes, start, size)
            .filter(|_| start >= HEADER_SIZE && start + size <= bitmap_start)
            .map(Some)
            .ok_or_else(|| format!("fixed value at {start} of {size} bytes outside the fixed data"))
    }

    /// Variable column `id`'s value: absent when after the last the record
    /// stores.
    pub(crate) fn variable(&self, id: u32) -> Stored<'a> {
        if id > self.last_variable || id < FIRST_VARIABLE {
            return Ok(None);
        }
        let index = (id - FIRST_VARIABLE) as usize;
        let raw = self
            .variable_offset(index)
            .ok_or("variable column offsets cut short")?;
        if raw & VARIABLE_NULL != 0 {
            return Ok(None);
        }
        let end = usize::from(raw & VARIABLE_OFFSET_MASK);
        let start = match index.checked_sub(1) {
            None => 0,
            Some(previous) => self
                .variable_offset(previous)
                .map_or(0, |raw| usize::from(raw & VARIABLE_OFFSET_MASK)),
        };
        let data = self.variable_data();
        self.bytes
            .get(data + start..data + end)
            .map(Some)
            .ok_or_else(|| format!("variable value from {start} to {end} outside the record"))
    }

    /// Tagged column `id`'s value, if stored. `derived` tells a template
    /// table's column (true) from the table's own of the same identifier;
    /// `None` takes either.
    pub(crate) fn tagged(&self, id: u32, derived: Option<bool>) -> Option<&Tagged<'a>> {
        self.tagged.iter().find(|tagged| {
            tagged.id == id && derived.map_or(true, |derived| tagged.derived == derived)
        })
    }
}

/// The tagged columns of format revision 9 and later: an array of
/// (identifier, offset) entries, its size given by the first offset, then
/// each value up to the next one's offset (the last up to the end).
fn parse_offsets(area: &[u8], extended: bool) -> (Vec<Tagged<'_>>, Option<String>) {
    if area.is_empty() {
        return (Vec::new(), None);
    }
    let mask = if extended {
        EXTENDED_TAGGED_OFFSET_MASK
    } else {
        TAGGED_OFFSET_MASK
    };
    let raw_offset = |index: usize| u16_at(area, index * TAGGED_ENTRY_SIZE + 2);
    let Some(first) = raw_offset(0) else {
        return (Vec::new(), Some("tagged columns cut short".to_owned()));
    };
    let count = usize::from(first & mask) / TAGGED_ENTRY_SIZE;
    if count == 0 || count * TAGGED_ENTRY_SIZE > area.len() {
        return (
            Vec::new(),
            Some(format!(
                "tagged column offset {first:#06x} outside the record"
            )),
        );
    }
    let mut tagged = Vec::with_capacity(count);
    for index in 0..count {
        let id = u16_at(area, index * TAGGED_ENTRY_SIZE).unwrap_or_default();
        let raw = raw_offset(index).unwrap_or_default();
        let start = usize::from(raw & mask);
        let end = if index + 1 < count {
            raw_offset(index + 1).map_or(area.len(), |next| usize::from(next & mask))
        } else {
            area.len()
        };
        let Some(value) = area
            .get(start..end)
            .filter(|_| start >= count * TAGGED_ENTRY_SIZE)
        else {
            let problem =
                format!("tagged column {id}: value from {start} to {end} outside the record");
            return (tagged, Some(problem));
        };
        let has_flags = extended || raw & TAGGED_HAS_FLAGS != 0;
        let (flags, data) = match value.split_first() {
            Some((&flags, data)) if has_flags => (TaggedFlags(flags), data),
            _ => (TaggedFlags::default(), value),
        };
        tagged.push(Tagged {
            id: u32::from(id),
            derived: raw & TAGGED_DERIVED != 0,
            flags,
            data,
        });
    }
    (tagged, None)
}

/// The tagged columns before format revision 9: identifier, size, flags
/// byte and data, one after the other.
fn parse_linear(mut area: &[u8]) -> (Vec<Tagged<'_>>, Option<String>) {
    let mut tagged = Vec::new();
    while !area.is_empty() {
        let id = u16_at(area, 0);
        let size = u16_at(area, 2).map(|size| usize::from(size & VARIABLE_OFFSET_MASK));
        let flags = u8_at(area, 4);
        let (Some(id), Some(size), Some(flags)) = (id, size, flags) else {
            return (tagged, Some("tagged column header cut short".to_owned()));
        };
        let Some(data) = slice_at(area, LINEAR_TAGGED_HEADER_SIZE, size) else {
            return (
                tagged,
                Some(format!("tagged column {id}: {size} bytes cut short")),
            );
        };
        tagged.push(Tagged {
            id: u32::from(id),
            derived: false,
            flags: TaggedFlags(flags),
            data,
        });
        area = &area[LINEAR_TAGGED_HEADER_SIZE + size..];
    }
    (tagged, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ColumnType;

    fn column(id: u32, column_type: ColumnType, record_offset: u16) -> Column {
        Column {
            id,
            name: format!("c{id}"),
            column_type,
            codepage: 1252,
            flags: 0,
            max_size: 0,
            record_offset,
            default: None,
            from_template: false,
        }
    }

    /// Two fixed columns (a Long at 4, NULL Short at 8), two variable
    /// columns ("ab", NULL), and tagged 256 = flags 0x01 + "xyz" and 257 =
    /// "q" (no flags byte).
    fn record() -> Vec<u8> {
        let mut bytes = vec![2, 129, 11, 0];
        bytes.extend(7u32.to_le_bytes());
        bytes.extend([0, 0]); // the NULL Short
        bytes.push(0b10); // NULL bitmap: column 2
        bytes.extend(2u16.to_le_bytes());
        bytes.extend((2u16 | VARIABLE_NULL).to_le_bytes());
        bytes.extend(b"ab");
        bytes.extend(256u16.to_le_bytes());
        bytes.extend((8u16 | TAGGED_HAS_FLAGS).to_le_bytes());
        bytes.extend(257u16.to_le_bytes());
        bytes.extend(12u16.to_le_bytes());
        bytes.extend([0x01, b'x', b'y', b'z', b'q']);
        bytes
    }

    fn parsed(bytes: &[u8]) -> Record<'_> {
        let (record, problem) =
            Record::parse(bytes, TaggedFormat::Offsets { extended: false }).unwrap();
        assert_eq!(problem, None);
        record
    }

    #[test]
    fn fixed_columns_and_their_null_bitmap() {
        let bytes = record();
        let record = parsed(&bytes);
        let long = column(1, ColumnType::Long, 4);
        assert_eq!(record.fixed(&long), Ok(Some(&7u32.to_le_bytes()[..])));
        assert_eq!(record.fixed(&column(2, ColumnType::Short, 8)), Ok(None));
        // After the last fixed column stored: absent.
        assert_eq!(record.fixed(&column(3, ColumnType::Bit, 10)), Ok(None));
    }

    #[test]
    fn variable_columns_end_at_their_offsets() {
        let bytes = record();
        let record = parsed(&bytes);
        assert_eq!(record.variable(128), Ok(Some(&b"ab"[..])));
        assert_eq!(record.variable(129), Ok(None));
        assert_eq!(record.variable(130), Ok(None));
    }

    #[test]
    fn tagged_columns_with_and_without_flags() {
        let bytes = record();
        let record = parsed(&bytes);
        let first = record.tagged(256, None).unwrap();
        assert_eq!((first.flags, first.data), (TaggedFlags(1), &b"xyz"[..]));
        let second = record.tagged(257, None).unwrap();
        assert_eq!((second.flags, second.data), (TaggedFlags(0), &b"q"[..]));
        assert!(record.tagged(258, None).is_none());
    }

    #[test]
    fn extended_tagged_values_always_carry_flags() {
        let mut bytes = vec![0, 127, 4, 0];
        bytes.extend(300u16.to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend([0x05, 1, 0, 0, 0]);
        let (record, _) = Record::parse(&bytes, TaggedFormat::Offsets { extended: true }).unwrap();
        let tagged = record.tagged(300, None).unwrap();
        assert!(tagged.flags.is_long_value());
        assert_eq!(tagged.data, [1, 0, 0, 0]);
    }

    #[test]
    fn linear_tagged_columns_follow_each_other() {
        let mut bytes = vec![0, 127, 4, 0];
        bytes.extend(256u16.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend([0, b'h', b'i']);
        let (record, problem) = Record::parse(&bytes, TaggedFormat::Linear).unwrap();
        assert_eq!(problem, None);
        assert_eq!(record.tagged(256, None).unwrap().data, b"hi");
    }

    #[test]
    fn damage_is_reported() {
        let bytes = record();
        assert!(Record::parse(&bytes[..2], TaggedFormat::Linear).is_err());
        let mut damaged = bytes.clone();
        damaged[2] = 200; // variable offsets past the end
        assert!(Record::parse(&damaged, TaggedFormat::Linear).is_err());
        let mut damaged = bytes;
        let second_offset = damaged.len() - 7;
        damaged[second_offset] = 0xff; // the first value now ends past the record
        let (record, problem) =
            Record::parse(&damaged, TaggedFormat::Offsets { extended: false }).unwrap();
        assert!(record.tagged.is_empty());
        assert!(problem.unwrap().contains("outside the record"));
    }
}
