//! Header and page checksums.
//!
//! Three page formats exist. The oldest holds a 32-bit XOR of the page,
//! seeded with 0x89abcdef, then the page number. From Exchange 2003 SP1 and
//! Windows Vista (a page flag says which) the first eight bytes hold two
//! 32-bit checksums over the rest of the page: an XOR seeded with the page
//! number, and an error-correcting code (ECC) that locates a single flipped
//! bit. From Windows 7, 16 and 32 KiB pages are split into four blocks, each
//! with its own pair: the first block's in the first eight bytes, the others'
//! in the extended page header.
//!
//! The ECC is the XOR of the positions of every set bit (bit i of byte k is
//! position 8k + i, from the start of the block), masked to the bits a
//! position needs. The checksum's low 16 bits hold it; the high 16 bits hold
//! it again when the number of set bits is even, its complement when odd.

use crate::bytes::{u32_at, u64_at};

/// The seed of the header and old-format page checksums.
const XOR_SEED: u32 = 0x89ab_cdef;
/// Bytes of a checksum field.
const CHECKSUM_SIZE: usize = 8;
/// The page flag of the newer checksum format.
pub(crate) const NEW_CHECKSUM_FLAG: u32 = 0x2000;
/// Where the page flags are.
const FLAGS_OFFSET: usize = 36;
/// Where an old-format page holds its own number.
const OLD_PAGE_NUMBER_OFFSET: usize = 4;
/// Where the checksums of blocks 1 to 3 of an extended page are.
const BLOCK_CHECKSUM_OFFSETS: [usize; 3] = [40, 48, 56];
/// Where an extended page holds its own number.
const EXTENDED_PAGE_NUMBER_OFFSET: usize = 64;
/// Blocks of an extended page.
const BLOCKS: usize = 4;

/// The header page's checksum: the XOR of its 32-bit words after the first.
pub(crate) fn header(page: &[u8]) -> u32 {
    xor_words(page.get(4..).unwrap_or_default(), XOR_SEED)
}

/// What is wrong with page `number`'s checksums or the page number it
/// holds, if anything.
pub(crate) fn verify_page(page: &[u8], number: u32, extended: bool) -> Option<String> {
    let flags = u32_at(page, FLAGS_OFFSET).unwrap_or_default();
    let mismatch = if flags & NEW_CHECKSUM_FLAG == 0 {
        verify_old_format(page, number)
    } else if extended {
        verify_extended(page, number)
    } else {
        verify_block(page, number, u64_at(page, 0), true).map(|m| m.to_string())
    };
    mismatch.map(|problem| format!("page {number}: {problem}"))
}

fn verify_old_format(page: &[u8], number: u32) -> Option<String> {
    let stored = u32_at(page, 0).unwrap_or_default();
    let computed = header(page);
    if stored != computed {
        return Some(format!(
            "checksum mismatch: stored 0x{stored:08x}, computed 0x{computed:08x}"
        ));
    }
    let claimed = u32_at(page, OLD_PAGE_NUMBER_OFFSET).unwrap_or_default();
    (claimed != number).then(|| format!("holds page number {claimed}"))
}

fn verify_extended(page: &[u8], number: u32) -> Option<String> {
    let block_size = page.len() / BLOCKS;
    let mut problems = Vec::new();
    for (index, block) in page.chunks_exact(block_size).enumerate() {
        let stored = match index.checked_sub(1) {
            None => u64_at(page, 0),
            Some(slot) => u64_at(page, BLOCK_CHECKSUM_OFFSETS[slot]),
        };
        if let Some(mismatch) = verify_block(block, number, stored, index == 0) {
            problems.push(format!("block {index}: {mismatch}"));
        }
    }
    let claimed = u64_at(page, EXTENDED_PAGE_NUMBER_OFFSET).unwrap_or_default();
    if claimed != u64::from(number) {
        problems.push(format!("holds page number {claimed}"));
    }
    (!problems.is_empty()).then(|| problems.join("; "))
}

/// Which of a block's two checksums don't match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mismatch {
    Xor { stored: u32, computed: u32 },
    Ecc { stored: u32, computed: u32 },
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, stored, computed) = match *self {
            Self::Xor { stored, computed } => ("XOR", stored, computed),
            Self::Ecc { stored, computed } => ("ECC", stored, computed),
        };
        write!(
            f,
            "{kind} checksum mismatch: stored 0x{stored:08x}, computed 0x{computed:08x}"
        )
    }
}

/// Check one block against its stored checksum pair. The first block of a
/// page holds the pair, so its first eight bytes aren't covered.
fn verify_block(
    block: &[u8],
    number: u32,
    stored: Option<u64>,
    holds_checksum: bool,
) -> Option<Mismatch> {
    let stored = stored.unwrap_or_default();
    let (stored_xor, stored_ecc) = (stored as u32, (stored >> 32) as u32);
    let skip = if holds_checksum { CHECKSUM_SIZE } else { 0 };
    let (xor, ecc) = xor_and_ecc(block, skip, number);
    if xor != stored_xor {
        Some(Mismatch::Xor {
            stored: stored_xor,
            computed: xor,
        })
    } else if ecc != stored_ecc {
        Some(Mismatch::Ecc {
            stored: stored_ecc,
            computed: ecc,
        })
    } else {
        None
    }
}

/// The XOR (seeded with `seed`) and ECC checksums of `block`, its first
/// `skip` bytes left out.
fn xor_and_ecc(block: &[u8], skip: usize, seed: u32) -> (u32, u32) {
    let mut xor = seed;
    let mut position = 0u32;
    let mut odd_bits = false;
    for (index, word) in words(block).enumerate().skip(skip / 4) {
        xor ^= word;
        if word.count_ones() % 2 == 1 {
            // Every set bit of the word contributes the word's first
            // position; an odd count leaves it once.
            position ^= (index as u32) * 32;
            odd_bits = !odd_bits;
        }
    }
    // Each bit position within a word set in an odd number of words
    // contributes once: the positions of the set bits of the column parity.
    let column_parity = xor ^ seed;
    for bit in (0..32).filter(|bit| column_parity >> bit & 1 == 1) {
        position ^= bit;
    }
    let mask = (block.len() as u32 * 8).wrapping_sub(1);
    let low = position & mask;
    let high = if odd_bits { !position & mask } else { low };
    (xor, (high << 16) | low)
}

fn xor_words(bytes: &[u8], seed: u32) -> u32 {
    words(bytes).fold(seed, |xor, word| xor ^ word)
}

/// The little-endian 32-bit words of `bytes` (a partial last word ignored).
fn words(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4 KiB new-format page numbered `number` with a byte set at
    /// `offset`, its checksums computed.
    fn page_with_byte(number: u32, offset: usize, byte: u8) -> Vec<u8> {
        let mut page = vec![0; 4096];
        page[FLAGS_OFFSET..FLAGS_OFFSET + 4].copy_from_slice(&NEW_CHECKSUM_FLAG.to_le_bytes());
        page[offset] = byte;
        let (xor, ecc) = xor_and_ecc(&page, CHECKSUM_SIZE, number);
        page[..4].copy_from_slice(&xor.to_le_bytes());
        page[4..8].copy_from_slice(&ecc.to_le_bytes());
        page
    }

    #[test]
    fn an_empty_block_checks_to_its_seed() {
        assert_eq!(xor_and_ecc(&[0; 8192], 0, 7), (7, 0));
    }

    #[test]
    fn the_ecc_locates_single_bits() {
        // One bit: its position, then its complement (odd count).
        let mut block = [0u8; 4096];
        block[100] = 0b0000_0100;
        let position = 100 * 8 + 2;
        assert_eq!(
            xor_and_ecc(&block, 0, 0).1,
            ((!position & 0x7fff) << 16) | position
        );
        // Two bits: the XOR of their positions, twice (even count).
        block[3] = 1;
        let position = position ^ (3 * 8);
        assert_eq!(xor_and_ecc(&block, 0, 0).1, (position << 16) | position);
    }

    #[test]
    fn a_page_with_valid_checksums_verifies() {
        let page = page_with_byte(9, 1000, 0x5a);
        assert_eq!(verify_page(&page, 9, false), None);
        // The page number seeds the XOR: the wrong page doesn't verify.
        assert!(verify_page(&page, 10, false)
            .unwrap()
            .contains("XOR checksum mismatch"));
    }

    #[test]
    fn a_flipped_bit_is_reported() {
        let mut page = page_with_byte(9, 1000, 0x5a);
        page[2000] ^= 0x10;
        let problem = verify_page(&page, 9, false).unwrap();
        assert!(
            problem.starts_with("page 9: XOR checksum mismatch"),
            "{problem}"
        );
    }

    #[test]
    fn old_format_pages_hold_their_number() {
        let mut page = vec![0; 4096];
        page[OLD_PAGE_NUMBER_OFFSET..OLD_PAGE_NUMBER_OFFSET + 4]
            .copy_from_slice(&5u32.to_le_bytes());
        let checksum = header(&page);
        page[..4].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(verify_page(&page, 5, false), None);
        assert_eq!(
            verify_page(&page, 6, false).as_deref(),
            Some("page 6: holds page number 5")
        );
    }
}
