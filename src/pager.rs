//! Pages by number. Page 1 follows the file header and its shadow copy, so
//! page n starts at byte (n + 1) × page size.

use crate::checksum;
use crate::header::Header;
use crate::page::Page;
use crate::record::TaggedFormat;

/// Pages before page 1: the header and its shadow.
const HEADER_PAGES: usize = 2;
/// Tagged columns take the offset array format from this revision of
/// format version 0x620.
const TAGGED_OFFSETS_REVISION: u32 = 9;
const ORIGINAL_FORMAT_VERSION: u32 = 0x620;

/// The database's pages, borrowed from the file.
pub(crate) struct Pages<'a> {
    file: &'a [u8],
    page_size: usize,
    count: u32,
    extended: bool,
    revision: u32,
    tagged_format: TaggedFormat,
}

impl<'a> Pages<'a> {
    pub(crate) fn new(file: &'a [u8], header: &Header) -> Self {
        let page_size = header.page_size as usize;
        let count = (file.len() / page_size).saturating_sub(HEADER_PAGES);
        let extended = header.extended_pages();
        let linear = header.format_version == ORIGINAL_FORMAT_VERSION
            && header.format_revision < TAGGED_OFFSETS_REVISION;
        Self {
            file,
            page_size,
            count: u32::try_from(count).unwrap_or(u32::MAX),
            extended,
            revision: header.format_revision,
            tagged_format: if linear {
                TaggedFormat::Linear
            } else {
                TaggedFormat::Offsets { extended }
            },
        }
    }

    /// Pages in the file, after the header pages.
    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    /// Bytes after the last whole page, if any.
    pub(crate) fn trailing_bytes(&self) -> usize {
        self.file.len() % self.page_size
    }

    /// Page `number`, or why it can't be read.
    pub(crate) fn get(&self, number: u32) -> Result<Page<'a>, String> {
        if number == 0 || number > self.count {
            return Err(format!(
                "page {number} is outside the database's {} pages",
                self.count
            ));
        }
        // Checked: on 32-bit targets a far page's offset overflows `usize`.
        let start = (number as usize + 1).checked_mul(self.page_size);
        let bytes = start
            .and_then(|start| self.file.get(start..start.checked_add(self.page_size)?))
            .ok_or_else(|| format!("page {number} is past the end of the file"))?;
        Ok(Page::new(number, bytes, self.extended, self.revision))
    }

    /// What is wrong with `page`'s checksums, if anything.
    pub(crate) fn verify(&self, page: &Page) -> Option<String> {
        checksum::verify_page(page.bytes, page.number, self.extended)
    }

    /// How records lay out their tagged columns.
    pub(crate) fn tagged_format(&self) -> TaggedFormat {
        self.tagged_format
    }
}
