//! Column values, decoded from their stored bytes by column type.

use crate::bytes::array_at;
use crate::schema::{Column, ColumnType};
use crate::text;

/// A column's value in a row.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// No value (NULL), or one that couldn't be read (see the problems).
    Null,
    /// A `Bit` column.
    Bool(bool),
    /// An `UnsignedByte` column.
    U8(u8),
    /// A `Short` column.
    I16(i16),
    /// An `UnsignedShort` column.
    U16(u16),
    /// A `Long` column.
    I32(i32),
    /// An `UnsignedLong` column.
    U32(u32),
    /// A `LongLong` column (in many databases a FILETIME: 100 ns since
    /// 1601).
    I64(i64),
    /// A `Currency` column: units of 1/10,000.
    Currency(i64),
    /// An `IEEESingle` column.
    F32(f32),
    /// An `IEEEDouble` column.
    F64(f64),
    /// A `DateTime` column: an OLE automation date, days since 1899-12-30
    /// (fraction: time of day).
    DateTime(f64),
    /// A `GUID` column, its 16 bytes as stored.
    Guid([u8; 16]),
    /// A `Text` or `LongText` column, decoded from its codepage.
    Text(String),
    /// A `Binary`, `LongBinary` or `SLV` column.
    Binary(Vec<u8>),
    /// Several values of a multi-valued column, in order.
    MultiValue(Vec<Value>),
}

impl Value {
    /// Whether this is NULL.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The text, if this is text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The bytes, if this is binary.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Binary(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Any integer (currency units included) as an `i64`, if this is one.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Self::U8(value) => Some(value.into()),
            Self::I16(value) => Some(value.into()),
            Self::U16(value) => Some(value.into()),
            Self::I32(value) => Some(value.into()),
            Self::U32(value) => Some(value.into()),
            Self::I64(value) | Self::Currency(value) => Some(value),
            _ => None,
        }
    }

    /// A float or date as an `f64`, if this is one.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Self::F32(value) => Some(value.into()),
            Self::F64(value) | Self::DateTime(value) => Some(value),
            _ => None,
        }
    }
}

/// `bytes` as a value of `column`.
pub(crate) fn decode(column: &Column, bytes: &[u8]) -> Result<Value, String> {
    let kind = column.column_type;
    let wrong_size = || {
        format!(
            "{} bytes for a {kind:?} value of {} bytes",
            bytes.len(),
            kind.size().unwrap_or_default()
        )
    };
    if kind.size().is_some_and(|size| size != bytes.len()) {
        return Err(wrong_size());
    }
    Ok(match kind {
        ColumnType::Bit => Value::Bool(u8::from_le_bytes(le(bytes)) != 0),
        ColumnType::UnsignedByte => Value::U8(u8::from_le_bytes(le(bytes))),
        ColumnType::Short => Value::I16(i16::from_le_bytes(le(bytes))),
        ColumnType::UnsignedShort => Value::U16(u16::from_le_bytes(le(bytes))),
        ColumnType::Long => Value::I32(i32::from_le_bytes(le(bytes))),
        ColumnType::UnsignedLong => Value::U32(u32::from_le_bytes(le(bytes))),
        ColumnType::IeeeSingle => Value::F32(f32::from_le_bytes(le(bytes))),
        ColumnType::LongLong => Value::I64(i64::from_le_bytes(le(bytes))),
        ColumnType::Currency => Value::Currency(i64::from_le_bytes(le(bytes))),
        ColumnType::IeeeDouble => Value::F64(f64::from_le_bytes(le(bytes))),
        ColumnType::DateTime => Value::DateTime(f64::from_le_bytes(le(bytes))),
        ColumnType::Guid => Value::Guid(le(bytes)),
        ColumnType::Text | ColumnType::LongText => {
            Value::Text(text::decode(bytes, column.codepage))
        }
        ColumnType::Binary
        | ColumnType::LongBinary
        | ColumnType::Slv
        | ColumnType::Nil
        | ColumnType::Unknown(_) => Value::Binary(bytes.to_vec()),
    })
}

/// The first `N` bytes of `bytes`, whose size the caller has checked.
fn le<const N: usize>(bytes: &[u8]) -> [u8; N] {
    array_at(bytes, 0).unwrap_or([0; N])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(column_type: ColumnType, codepage: u32) -> Column {
        Column {
            id: 1,
            name: "c".to_owned(),
            column_type,
            codepage,
            flags: 0,
            max_size: 0,
            record_offset: 4,
            default: None,
            from_template: false,
        }
    }

    fn decoded(column_type: ColumnType, bytes: &[u8]) -> Result<Value, String> {
        decode(&column(column_type, 1252), bytes)
    }

    #[test]
    fn numbers_are_little_endian() {
        assert_eq!(decoded(ColumnType::Bit, &[0xff]), Ok(Value::Bool(true)));
        assert_eq!(
            decoded(ColumnType::Short, &[0xfe, 0xff]),
            Ok(Value::I16(-2))
        );
        assert_eq!(
            decoded(ColumnType::UnsignedShort, &[0xfe, 0xff]),
            Ok(Value::U16(65534))
        );
        assert_eq!(
            decoded(ColumnType::Long, &(-5i32).to_le_bytes()),
            Ok(Value::I32(-5))
        );
        assert_eq!(
            decoded(ColumnType::Currency, &7i64.to_le_bytes()),
            Ok(Value::Currency(7))
        );
        assert_eq!(
            decoded(ColumnType::DateTime, &1.5f64.to_le_bytes()),
            Ok(Value::DateTime(1.5))
        );
        assert_eq!(
            decoded(ColumnType::Guid, &[9; 16]),
            Ok(Value::Guid([9; 16]))
        );
    }

    #[test]
    fn text_follows_the_codepage() {
        assert_eq!(
            decode(&column(ColumnType::Text, 1200), &[b'h', 0, b'i', 0]),
            Ok(Value::Text("hi".to_owned()))
        );
        assert_eq!(
            decode(&column(ColumnType::LongText, 1252), &[0x80]),
            Ok(Value::Text("\u{20ac}".to_owned()))
        );
    }

    #[test]
    fn a_wrong_size_is_an_error() {
        let problem = decoded(ColumnType::Long, &[1, 2]).unwrap_err();
        assert_eq!(problem, "2 bytes for a Long value of 4 bytes");
        assert!(decoded(ColumnType::Bit, &[]).is_err());
    }

    #[test]
    fn accessors_widen() {
        assert_eq!(Value::U32(7).as_i64(), Some(7));
        assert_eq!(Value::F32(0.5).as_f64(), Some(0.5));
        assert_eq!(Value::Text("a".to_owned()).as_i64(), None);
    }
}
