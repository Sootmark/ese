//! Plain LZ77 decompression ("Xpress" without Huffman coding), as
//! [MS-XCA] section 2.4 specifies it.
//!
//! A 32-bit flag word precedes every 32 items, most significant bit first:
//! 0 for a literal byte, 1 for a match of a 16-bit token (offset − 1 in the
//! top 13 bits, length − 3 in the low 3). Length 7 continues in a shared
//! half byte, then a byte, then 16 or 32 bits. A match flag at the end of
//! the input ends the stream.
//!
//! [MS-XCA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/

use crate::bytes::{u16_at, u32_at, u8_at};

/// Items per flag word.
const FLAG_BITS: u32 = 32;
/// The shortest match.
const MIN_MATCH: usize = 3;
/// Length codes that continue in the next field.
const LENGTH_IN_HALF_BYTE: usize = 7;
const LENGTH_IN_BYTE: usize = 15;
const LENGTH_IN_WORD: usize = 255;
/// Bits of a token holding the length.
const LENGTH_BITS: u16 = 3;
const LENGTH_MASK: u16 = 0b111;

/// The input, read front to back.
struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Input<'_> {
    fn is_done(&self) -> bool {
        self.at >= self.bytes.len()
    }

    fn take<T>(&mut self, size: usize, read: fn(&[u8], usize) -> Option<T>) -> Result<T, String> {
        let value = read(self.bytes, self.at).ok_or("Xpress stream cut short")?;
        self.at += size;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, String> {
        self.take(1, u8_at)
    }

    fn word(&mut self) -> Result<u16, String> {
        self.take(2, u16_at)
    }

    fn double_word(&mut self) -> Result<u32, String> {
        self.take(4, u32_at)
    }
}

/// Decompress `input` to the `size` bytes it holds.
pub(crate) fn decompress(input: &[u8], size: usize) -> Result<Vec<u8>, String> {
    let mut input = Input {
        bytes: input,
        at: 0,
    };
    let mut output = Vec::with_capacity(size);
    let mut flags = 0u32;
    let mut flags_left = 0u32;
    // Where the half byte shared by two long matches is, once the first
    // has used its low half.
    let mut shared_half_byte = None;
    while output.len() < size {
        if flags_left == 0 {
            flags = input.double_word()?;
            flags_left = FLAG_BITS;
        }
        flags_left -= 1;
        if flags >> flags_left & 1 == 0 {
            output.push(input.byte()?);
            continue;
        }
        if input.is_done() {
            break;
        }
        let token = input.word()?;
        let offset = usize::from(token >> LENGTH_BITS) + 1;
        let length = match_length(&mut input, token, &mut shared_half_byte)?;
        if offset > output.len() {
            return Err(format!(
                "Xpress match {offset} bytes back, {} written",
                output.len()
            ));
        }
        for _ in 0..length.min(size - output.len()) {
            output.push(output[output.len() - offset]);
        }
    }
    if output.len() == size {
        Ok(output)
    } else {
        Err(format!(
            "Xpress stream holds {} bytes, not {size}",
            output.len()
        ))
    }
}

/// The length of the match `token` starts.
fn match_length(
    input: &mut Input,
    token: u16,
    shared_half_byte: &mut Option<usize>,
) -> Result<usize, String> {
    let mut length = usize::from(token & LENGTH_MASK);
    if length == LENGTH_IN_HALF_BYTE {
        length = if let Some(at) = shared_half_byte.take() {
            usize::from(u8_at(input.bytes, at).unwrap_or_default() >> 4)
        } else {
            *shared_half_byte = Some(input.at);
            usize::from(input.byte()? & 0x0f)
        };
        if length == LENGTH_IN_BYTE {
            length = usize::from(input.byte()?);
            if length == LENGTH_IN_WORD {
                length = usize::from(input.word()?);
                if length == 0 {
                    length = input.double_word()? as usize;
                }
                length = length
                    .checked_sub(LENGTH_IN_BYTE + LENGTH_IN_HALF_BYTE)
                    .ok_or("Xpress match length too short for its encoding")?;
            }
            length += LENGTH_IN_BYTE;
        }
        length += LENGTH_IN_HALF_BYTE;
    }
    Ok(length + MIN_MATCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_only() {
        // [MS-XCA] 3.1: the alphabet, no matches; the end flag follows.
        let mut input = vec![0x3f, 0x00, 0x00, 0x00];
        input.extend(b'a'..=b'z');
        assert_eq!(
            decompress(&input, 26).unwrap(),
            (b'a'..=b'z').collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_long_overlapping_match() {
        // [MS-XCA] 3.2: "abc" 100 times, one match of 297 bytes, 3 back.
        let input = [
            0xff, 0xff, 0xff, 0x1f, b'a', b'b', b'c', 0x17, 0x00, 0x0f, 0xff, 0x26, 0x01,
        ];
        assert_eq!(decompress(&input, 300).unwrap(), b"abc".repeat(100));
    }

    /// Bytes from hex.
    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    /// Streams made by `tests/oracle/xpress.py` and checked with libfwnt's
    /// decoder: literals and short matches; two long matches sharing a
    /// half byte; a 4,997-byte match, its length in 16 bits.
    #[test]
    fn streams_libfwnt_decodes() {
        let vectors = [
            (
                [&b"In ESE, compressed values: "[..], &b"abc".repeat(20), b"xyz"].concat(),
                "02000000496e204553452c20636f6d707265737365642076616c7565733a2061626317000f2078ffffff3f797a",
            ),
            (
                [&b"0123456789".repeat(3)[..], b"ABCDEFGHIJ", &b"0123456789".repeat(4), &b"ABCDEFGHIJ".repeat(3)].concat(),
                "ff072000303132333435363738394f00fa4142434445464748494a3f01058f01aa4f00",
            ),
            (vec![0; 5000], "ffffff7f0007000fff8413"),
        ];
        for (plain, compressed) in vectors {
            assert_eq!(decompress(&hex(compressed), plain.len()).unwrap(), plain);
        }
    }

    #[test]
    fn a_match_length_in_32_bits() {
        // "a", then a match 1 back: length code 7, half byte 15, byte 255,
        // 16 bits 0, 32 bits 70,000: 70,000 - 22 + 15 + 7 + 3 bytes.
        let input = hex("ffffff7f6107000fff000070110100");
        assert_eq!(decompress(&input, 70_004).unwrap(), vec![b'a'; 70_004]);
    }

    #[test]
    fn damage_is_an_error() {
        // A match before any output.
        let input = [0x00, 0x00, 0x00, 0x80, 0x08, 0x00];
        assert!(decompress(&input, 4).is_err());
        // Cut short.
        assert!(decompress(&[0x00, 0x00, 0x00, 0x00, b'a'], 5).is_err());
        // Fewer bytes than declared.
        let mut input = vec![0x3f, 0x00, 0x00, 0x00];
        input.extend(b"abc");
        assert!(decompress(&input, 26).is_err());
    }
}
