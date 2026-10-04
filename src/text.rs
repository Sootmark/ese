//! Text in the codepages ESE columns use: UTF-16 little-endian (1200),
//! Windows Latin-1 (1252, also the default) and ASCII (20127). Bytes that
//! don't decode become U+FFFD. The terminating NULs applications often
//! store are dropped.
//!
//! Some applications store 8-bit text in UTF-16 columns (Windows Search's
//! `__NameTable__`, for one): a value of odd size in such a column can't be
//! UTF-16 and is read as Windows-1252, as libesedb reads it.

/// Codepages.
pub(crate) const UTF_16LE: u32 = 1200;
pub(crate) const WINDOWS_1252: u32 = 1252;
pub(crate) const US_ASCII: u32 = 20127;
/// No codepage set: Windows Latin-1.
const UNSET: u32 = 0;

/// Windows-1252 0x80 to 0x9f, where it differs from Latin-1. The five
/// unassigned bytes map to the C1 controls of the same value, as the WHATWG
/// encoding standard does.
const WINDOWS_1252_HIGH: [char; 32] = [
    '\u{20ac}', '\u{0081}', '\u{201a}', '\u{0192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02c6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008d}', '\u{017d}', '\u{008f}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02dc}', '\u{2122}', '\u{0161}', '\u{203a}', '\u{0153}', '\u{009d}', '\u{017e}', '\u{0178}',
];
const WINDOWS_1252_HIGH_START: u8 = 0x80;

/// Whether text in `codepage` is decoded as such (others read as
/// Windows-1252).
pub(crate) fn is_supported(codepage: u32) -> bool {
    matches!(codepage, UTF_16LE | WINDOWS_1252 | US_ASCII | UNSET)
}

/// `bytes` as text in `codepage`, without trailing NULs.
pub(crate) fn decode(bytes: &[u8], codepage: u32) -> String {
    let mut text = decode_all(bytes, codepage);
    let kept = text.trim_end_matches('\0').len();
    text.truncate(kept);
    text
}

fn decode_all(bytes: &[u8], codepage: u32) -> String {
    match codepage {
        UTF_16LE if bytes.len() % 2 == 0 => utf16le(bytes),
        US_ASCII => bytes
            .iter()
            .map(|&byte| {
                if byte.is_ascii() {
                    char::from(byte)
                } else {
                    char::REPLACEMENT_CHARACTER
                }
            })
            .collect(),
        _ => bytes.iter().map(|&byte| windows_1252(byte)).collect(),
    }
}

fn utf16le(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

fn windows_1252(byte: u8) -> char {
    match byte.checked_sub(WINDOWS_1252_HIGH_START) {
        Some(index) if usize::from(index) < WINDOWS_1252_HIGH.len() => {
            WINDOWS_1252_HIGH[usize::from(index)]
        }
        _ => char::from(byte),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_with_surrogates_and_damage() {
        assert_eq!(decode(&[b'h', 0, b'i', 0], UTF_16LE), "hi");
        assert_eq!(decode(&[0x3d, 0xd8, 0x00, 0xde], UTF_16LE), "\u{1f600}");
        assert_eq!(decode(&[0x00, 0xd8, b'a', 0], UTF_16LE), "\u{fffd}a");
    }

    #[test]
    fn odd_sized_utf16_is_8_bit_text() {
        assert_eq!(decode(b"Index_\0", UTF_16LE), "Index_");
    }

    #[test]
    fn windows_1252_differs_from_latin1_in_0x80_to_0x9f() {
        assert_eq!(
            decode(&[0x80, 0x99, 0xe9, 0x81], WINDOWS_1252),
            "\u{20ac}\u{2122}\u{e9}\u{81}"
        );
        assert_eq!(decode(b"abc", UNSET), "abc");
    }

    #[test]
    fn terminating_nuls_are_dropped() {
        assert_eq!(decode(&[b'a', 0, 0, 0, 0, 0], UTF_16LE), "a");
        assert_eq!(decode(b"a\0b\0", WINDOWS_1252), "a\0b");
    }

    #[test]
    fn ascii_rejects_high_bytes() {
        assert_eq!(decode(&[b'a', 0xe9], US_ASCII), "a\u{fffd}");
    }
}
