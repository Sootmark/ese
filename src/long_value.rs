//! Long values: values too large for their record, stored in the table's
//! long value tree and referred to by a long value identifier (LID) of 4 or
//! 8 bytes, little-endian in the record.
//!
//! The tree is keyed by the LID big-endian: alone for the long value's root
//! (a reference count and the total size, 32 bits each), followed by a
//! big-endian 32-bit offset for each segment of the data. When the column's
//! value is compressed, each segment is compressed on its own.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::btree::Cursor;
use crate::bytes::u32_at;
use crate::compression;
use crate::pager::Pages;

/// Where a long value's root holds its size.
const ROOT_SIZE_OFFSET: usize = 4;
/// Bytes of a segment key's offset.
const SEGMENT_OFFSET_SIZE: usize = 4;

/// A table's long value tree, read whole: key to data, borrowed from the
/// file.
pub(crate) struct LongValues<'a> {
    entries: BTreeMap<Vec<u8>, &'a [u8]>,
}

impl<'a> LongValues<'a> {
    /// Read the tree at `root`, and the damage met.
    pub(crate) fn read(pages: &'a Pages<'a>, root: u32) -> (Self, Vec<String>) {
        let mut cursor = Cursor::new(pages, root);
        let entries = cursor
            .by_ref()
            .map(|entry| (entry.node.key(), entry.node.data))
            .collect();
        (Self { entries }, cursor.problems)
    }

    /// The data of the long value `id` (as the record stores it), and what
    /// cut it short, if anything (the bytes before the damage are kept).
    pub(crate) fn get(
        &self,
        id: &[u8],
        compressed: bool,
    ) -> Result<(Vec<u8>, Option<String>), String> {
        let name = lid_name(id);
        let key: Vec<u8> = id.iter().rev().copied().collect();
        let root = self
            .entries
            .get(&key)
            .ok_or_else(|| format!("long value {name} not in the long value tree"))?;
        let size = u32_at(root, ROOT_SIZE_OFFSET)
            .ok_or_else(|| format!("long value {name}: root of {} bytes", root.len()))?
            as usize;
        let read = self.assemble(&name, &key, size, compressed);
        if compressed || read.1.is_none() {
            return Ok(read);
        }
        // Windows 10 compresses some long values without flagging them in
        // the record: when the segments read plain fall short of the size
        // the root declares, read them as compressed, and keep that only
        // when it gives exactly that size.
        let decompressed = self.assemble(&name, &key, size, true);
        Ok(if decompressed.1.is_none() {
            decompressed
        } else {
            read
        })
    }

    /// The segments of `key` joined (each decompressed if `compressed`), up
    /// to `size` bytes, and what cut them short, if anything.
    fn assemble(
        &self,
        name: &str,
        key: &[u8],
        size: usize,
        compressed: bool,
    ) -> (Vec<u8>, Option<String>) {
        let mut data = Vec::new();
        for (offset, segment) in self.segments(key) {
            if offset != data.len() {
                let problem = format!(
                    "long value {name}: segment at {offset}, expected {}",
                    data.len()
                );
                return (data, Some(problem));
            }
            if compressed {
                match compression::decompress(segment) {
                    Ok(plain) => data.extend(plain),
                    Err(problem) => return (data, Some(format!("long value {name}: {problem}"))),
                }
            } else {
                data.extend_from_slice(segment);
            }
            if data.len() >= size {
                data.truncate(size);
                return (data, None);
            }
        }
        let problem = format!("long value {name}: {} of {size} bytes found", data.len());
        (data, Some(problem))
    }

    /// The segments of the long value keyed `key`, with their offsets, in
    /// order.
    fn segments<'s>(&'s self, key: &'s [u8]) -> impl Iterator<Item = (usize, &'a [u8])> + 's {
        self.entries
            .range::<[u8], _>((Bound::Excluded(key), Bound::Unbounded))
            .take_while(move |(segment_key, _)| segment_key.starts_with(key))
            .filter(move |(segment_key, _)| segment_key.len() == key.len() + SEGMENT_OFFSET_SIZE)
            .map(move |(segment_key, &data)| {
                let offset = segment_key[key.len()..]
                    .iter()
                    .fold(0usize, |offset, &byte| (offset << 8) | usize::from(byte));
                (offset, data)
            })
    }
}

/// A long value identifier for messages: its number.
fn lid_name(id: &[u8]) -> String {
    let number = id
        .iter()
        .rev()
        .fold(0u64, |number, &byte| (number << 8) | u64::from(byte));
    number.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Long value 1 (10 bytes in segments of 4, 4 and 2) and long value 2,
    /// whose second segment is missing.
    fn tree() -> LongValues<'static> {
        let mut entries = BTreeMap::new();
        let root: &'static [u8] = &[1, 0, 0, 0, 10, 0, 0, 0];
        entries.insert(vec![0, 0, 0, 1], root);
        entries.insert(vec![0, 0, 0, 1, 0, 0, 0, 0], &b"abcd"[..]);
        entries.insert(vec![0, 0, 0, 1, 0, 0, 0, 4], &b"efgh"[..]);
        entries.insert(vec![0, 0, 0, 1, 0, 0, 0, 8], &b"ij"[..]);
        entries.insert(vec![0, 0, 0, 2], root);
        entries.insert(vec![0, 0, 0, 2, 0, 0, 0, 0], &b"abcd"[..]);
        entries.insert(vec![0, 0, 0, 2, 0, 0, 0, 8], &b"ij"[..]);
        // Long value 3: UTF-16 "link" (8 bytes) compressed to 5 (7-bit
        // Unicode) though the record doesn't say so, as Windows 10 writes
        // some; long value 4: the same bytes, but a root declaring 10 bytes,
        // which they don't give either way.
        let link: &'static [u8] = &[0x13, 0xec, 0xb4, 0x7b, 0x0d];
        entries.insert(vec![0, 0, 0, 3], &[1, 0, 0, 0, 8, 0, 0, 0][..]);
        entries.insert(vec![0, 0, 0, 3, 0, 0, 0, 0], link);
        entries.insert(vec![0, 0, 0, 4], &[1, 0, 0, 0, 10, 0, 0, 0][..]);
        entries.insert(vec![0, 0, 0, 4, 0, 0, 0, 0], link);
        LongValues { entries }
    }

    #[test]
    fn segments_join_in_order() {
        assert_eq!(
            tree().get(&[1, 0, 0, 0], false),
            Ok((b"abcdefghij".to_vec(), None))
        );
    }

    #[test]
    fn a_gap_keeps_the_data_before_it() {
        let (data, problem) = tree().get(&[2, 0, 0, 0], false).unwrap();
        assert_eq!(data, b"abcd");
        assert_eq!(
            problem.as_deref(),
            Some("long value 2: segment at 8, expected 4")
        );
    }

    #[test]
    fn unflagged_compression_is_read_when_it_gives_the_size() {
        assert_eq!(
            tree().get(&[3, 0, 0, 0], false),
            Ok((b"l\0i\0n\0k\0".to_vec(), None))
        );
        // Decompressed, 8 bytes, not the 10 declared: kept as read, with the
        // shortfall reported.
        let (data, problem) = tree().get(&[4, 0, 0, 0], false).unwrap();
        assert_eq!(data, [0x13, 0xec, 0xb4, 0x7b, 0x0d]);
        assert_eq!(
            problem.as_deref(),
            Some("long value 4: 5 of 10 bytes found")
        );
    }

    #[test]
    fn a_missing_long_value_is_an_error() {
        assert_eq!(
            tree().get(&[9, 0, 0, 0], false),
            Err("long value 9 not in the long value tree".to_owned())
        );
    }
}
