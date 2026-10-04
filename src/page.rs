//! Pages: a header, values, and the tags that locate the values, stored
//! back to front from the end of the page.
//!
//! Tag 0 locates the page's own header value: the space header on a root
//! page, else the key prefix that entries flagged "common key" share. Every
//! other tag locates an entry: a key (prefix size, suffix size, suffix) and
//! data, a child page number on branch pages.

use crate::bytes::{slice_at, u16_at, u32_at};

/// Page header sizes.
const HEADER_SIZE: usize = 40;
const EXTENDED_HEADER_SIZE: usize = 80;
/// Page header field offsets.
const FDP_OBJECT_ID: usize = 24;
const TAG_COUNT: usize = 34;
const FLAGS: usize = 36;
/// Bytes of a tag.
const TAG_SIZE: usize = 4;
/// Masks of a tag's size and offset fields.
const TAG_FIELD_MASK: u16 = 0x1fff;
const EXTENDED_TAG_FIELD_MASK: u16 = 0x7fff;
/// Tag flags sit in the top three bits of a 16-bit field.
const TAG_FLAGS_SHIFT: u16 = 13;
/// From format revision 0x122, the top four bits of the tag count are
/// reserved.
const RESERVED_TAG_COUNT_REVISION: u32 = 0x122;
const TAG_COUNT_MASK: u16 = 0x0fff;
/// Bytes of a child page number in a branch entry.
const CHILD_SIZE: usize = 4;

/// What a page is, from its header flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageFlags(pub(crate) u32);

impl PageFlags {
    const ROOT: u32 = 0x01;
    const LEAF: u32 = 0x02;
    const EMPTY: u32 = 0x08;
    const SPACE_TREE: u32 = 0x20;

    pub(crate) fn is_root(self) -> bool {
        self.0 & Self::ROOT != 0
    }

    pub(crate) fn is_leaf(self) -> bool {
        self.0 & Self::LEAF != 0
    }

    pub(crate) fn is_empty(self) -> bool {
        self.0 & Self::EMPTY != 0
    }

    pub(crate) fn is_space_tree(self) -> bool {
        self.0 & Self::SPACE_TREE != 0
    }
}

/// Flags of an entry, from its tag (or, on extended pages, the top bits of
/// its first 16-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TagFlags(u16);

impl TagFlags {
    /// Deleted, not yet cleaned up.
    const DEFUNCT: u16 = 0x2;
    /// The key starts with part of the page's key prefix.
    const COMMON_KEY: u16 = 0x4;

    pub(crate) fn is_defunct(self) -> bool {
        self.0 & Self::DEFUNCT != 0
    }

    fn has_common_key(self) -> bool {
        self.0 & Self::COMMON_KEY != 0
    }
}

/// An entry of a page: its key, as a part of the page's prefix and a
/// suffix of its own, and its data.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Node<'a> {
    pub(crate) flags: TagFlags,
    pub(crate) prefix: &'a [u8],
    pub(crate) suffix: &'a [u8],
    pub(crate) data: &'a [u8],
}

impl Node<'_> {
    /// The whole key.
    pub(crate) fn key(&self) -> Vec<u8> {
        [self.prefix, self.suffix].concat()
    }
}

/// A page of the database, borrowed from the file.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Page<'a> {
    pub(crate) number: u32,
    pub(crate) bytes: &'a [u8],
    extended: bool,
    revision: u32,
}

impl<'a> Page<'a> {
    pub(crate) fn new(number: u32, bytes: &'a [u8], extended: bool, revision: u32) -> Self {
        Self {
            number,
            bytes,
            extended,
            revision,
        }
    }

    fn field(&self, at: usize) -> u32 {
        u32_at(self.bytes, at).unwrap_or_default()
    }

    pub(crate) fn flags(&self) -> PageFlags {
        PageFlags(self.field(FLAGS))
    }

    /// The object identifier of the tree the page belongs to.
    pub(crate) fn fdp_object_id(&self) -> u32 {
        self.field(FDP_OBJECT_ID)
    }

    /// Whether every byte is zero: a page never written.
    pub(crate) fn is_blank(&self) -> bool {
        self.bytes.iter().all(|&byte| byte == 0)
    }

    fn header_size(&self) -> usize {
        if self.extended {
            EXTENDED_HEADER_SIZE
        } else {
            HEADER_SIZE
        }
    }

    /// The tags in use, cut to those that fit between the header and the
    /// end of the page.
    pub(crate) fn tag_count(&self) -> Result<usize, String> {
        let mut count = u16_at(self.bytes, TAG_COUNT).unwrap_or_default();
        if self.revision >= RESERVED_TAG_COUNT_REVISION {
            count &= TAG_COUNT_MASK;
        }
        let room = self.bytes.len().saturating_sub(self.header_size()) / TAG_SIZE;
        if usize::from(count) > room {
            return Err(format!(
                "page {}: {count} tags declared, room for {room}",
                self.number
            ));
        }
        Ok(usize::from(count))
    }

    /// The value tag `index` locates, and the tag's flags (small pages).
    fn tag(&self, index: usize) -> Result<(&'a [u8], TagFlags), String> {
        let outside = || format!("page {}: tag {index} outside the page", self.number);
        let at = (index + 1)
            .checked_mul(TAG_SIZE)
            .and_then(|end| self.bytes.len().checked_sub(end))
            .ok_or_else(outside)?;
        let size_field = u16_at(self.bytes, at).unwrap_or_default();
        let offset_field = u16_at(self.bytes, at + 2).unwrap_or_default();
        let mask = if self.extended {
            EXTENDED_TAG_FIELD_MASK
        } else {
            TAG_FIELD_MASK
        };
        let flags = if self.extended {
            TagFlags::default()
        } else {
            TagFlags(offset_field >> TAG_FLAGS_SHIFT)
        };
        let start = self.header_size() + usize::from(offset_field & mask);
        let size = usize::from(size_field & mask);
        let tags_start = self.bytes.len() - TAG_SIZE * self.tag_count()?;
        slice_at(self.bytes, start, size)
            .filter(|_| start + size <= tags_start)
            .map(|value| (value, flags))
            .ok_or_else(|| {
                format!(
                    "page {}: tag {index} locates bytes outside the page's values",
                    self.number
                )
            })
    }

    /// The key prefix entries flagged "common key" share: tag 0's value,
    /// except on a root page, where tag 0 holds the space header.
    fn prefix(&self) -> &'a [u8] {
        if self.flags().is_root() {
            return &[];
        }
        self.tag(0).map(|(value, _)| value).unwrap_or_default()
    }

    /// Entry `index` (1 and up: tag 0 is the page's header value).
    pub(crate) fn node(&self, index: usize) -> Result<Node<'a>, String> {
        let (value, tag_flags) = self.tag(index)?;
        let cut = || format!("page {}: entry {index} cut short", self.number);
        let first = u16_at(value, 0).ok_or_else(cut)?;
        let (flags, first) = if self.extended {
            (TagFlags(first >> TAG_FLAGS_SHIFT), first & TAG_FIELD_MASK)
        } else {
            (tag_flags, first)
        };
        let (prefix_size, suffix_size, suffix_at) = if flags.has_common_key() {
            let suffix_size = u16_at(value, 2).ok_or_else(cut)?;
            (usize::from(first), usize::from(suffix_size), 4)
        } else {
            (0, usize::from(first), 2)
        };
        let page_prefix = self.prefix();
        let prefix = page_prefix.get(..prefix_size).ok_or_else(|| {
            format!(
                "page {}: entry {index} shares {prefix_size} key bytes, the page's prefix has {}",
                self.number,
                page_prefix.len()
            )
        })?;
        let suffix = slice_at(value, suffix_at, suffix_size).ok_or_else(cut)?;
        Ok(Node {
            flags,
            prefix,
            suffix,
            data: &value[suffix_at + suffix_size..],
        })
    }

    /// The child page a branch entry points to.
    pub(crate) fn child(&self, node: &Node) -> Result<u32, String> {
        u32_at(node.data, 0)
            .filter(|_| node.data.len() == CHILD_SIZE)
            .ok_or_else(|| {
                format!(
                    "page {}: branch entry of {} data bytes, not a page number",
                    self.number,
                    node.data.len()
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_SIZE: usize = 4096;

    /// A small page with `values` (tag 0 first), each placed in order after
    /// the header, `flags` in the header and `tag_flags` on every tag.
    fn page(flags: u32, values: &[&[u8]], tag_flags: u16) -> Vec<u8> {
        let mut bytes = vec![0; PAGE_SIZE];
        bytes[FLAGS..FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
        bytes[TAG_COUNT..TAG_COUNT + 2].copy_from_slice(&(values.len() as u16).to_le_bytes());
        let mut offset = 0;
        for (index, value) in values.iter().enumerate() {
            let start = HEADER_SIZE + offset;
            bytes[start..start + value.len()].copy_from_slice(value);
            let at = PAGE_SIZE - TAG_SIZE * (index + 1);
            bytes[at..at + 2].copy_from_slice(&(value.len() as u16).to_le_bytes());
            let field = offset as u16 | (tag_flags << TAG_FLAGS_SHIFT);
            bytes[at + 2..at + 4].copy_from_slice(&field.to_le_bytes());
            offset += value.len();
        }
        bytes
    }

    #[test]
    fn entries_share_the_page_prefix() {
        // Leaf, not root: tag 0 is the prefix "ab".
        let bytes = page(PageFlags::LEAF, &[b"ab", &[1, 0, 1, 0, b'c', 9]], 0x4);
        let page = Page::new(5, &bytes, false, 0x14);
        let node = page.node(1).unwrap();
        assert_eq!(node.key(), b"ac");
        assert_eq!(node.data, [9]);
    }

    #[test]
    fn entries_without_common_key_stand_alone() {
        let bytes = page(PageFlags::LEAF, &[b"", &[2, 0, b'x', b'y', 7, 7]], 0);
        let node = Page::new(5, &bytes, false, 0x14).node(1).unwrap();
        assert_eq!(node.key(), b"xy");
        assert_eq!(node.data, [7, 7]);
    }

    #[test]
    fn branch_entries_point_to_children() {
        let bytes = page(0, &[b"", &[1, 0, b'k', 42, 0, 0, 0]], 0);
        let page = Page::new(5, &bytes, false, 0x14);
        assert_eq!(page.child(&page.node(1).unwrap()), Ok(42));
    }

    #[test]
    fn damaged_tags_are_errors() {
        let mut bytes = page(PageFlags::LEAF, &[b"", &[9, 0, b'k']], 0);
        assert!(Page::new(5, &bytes, false, 0x14).node(1).is_err());
        bytes[TAG_COUNT..TAG_COUNT + 2].copy_from_slice(&5000u16.to_le_bytes());
        let problem = Page::new(5, &bytes, false, 0x14).tag_count().unwrap_err();
        assert!(problem.contains("5000 tags declared"), "{problem}");
    }

    #[test]
    fn a_prefix_longer_than_the_page_prefix_is_an_error() {
        let bytes = page(PageFlags::LEAF, &[b"a", &[3, 0, 0, 0, 1]], 0x4);
        assert!(Page::new(5, &bytes, false, 0x14).node(1).is_err());
    }
}
