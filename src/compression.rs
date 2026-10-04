//! Compressed values, from Windows 7: the first byte's top five bits name
//! the scheme, its low three bits serve the 7-bit schemes.
//!
//! - 7-bit ASCII (1): bytes under 0x80, seven bits each, packed least
//!   significant bit first; the low three bits of the first byte are the bits
//!   used in the last byte, minus one.
//! - 7-bit Unicode (2): the same for UTF-16 code units under 0x80, each
//!   restored to two bytes.
//! - Xpress (3): a 16-bit uncompressed size, then plain LZ77 ([MS-XCA]).
//!
//! Other schemes (Xpress9, Xpress10, scrubbed data) are reported as
//! unsupported.
//!
//! [MS-XCA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/

use crate::bytes::u16_at;
use crate::xpress;

/// The scheme is in the first byte's top five bits.
const SCHEME_SHIFT: u8 = 3;
const SEVEN_BIT_ASCII: u8 = 1;
const SEVEN_BIT_UNICODE: u8 = 2;
const XPRESS: u8 = 3;
/// The low three bits of a 7-bit scheme's first byte.
const LAST_BYTE_BITS_MASK: u8 = 0b111;
/// Bits per 7-bit unit.
const UNIT_BITS: usize = 7;
const UNIT_MASK: u32 = 0x7f;
/// Bytes of the Xpress size field.
const XPRESS_SIZE_FIELD: usize = 2;

/// The bytes a compressed value holds.
pub(crate) fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    let (&header, payload) = data.split_first().ok_or("an empty compressed value")?;
    match header >> SCHEME_SHIFT {
        SEVEN_BIT_ASCII => Ok(seven_bit(header, payload, Unit::Byte)),
        SEVEN_BIT_UNICODE => Ok(seven_bit(header, payload, Unit::Utf16)),
        XPRESS => {
            let size = u16_at(payload, 0).ok_or("an Xpress value without its size")?;
            xpress::decompress(&payload[XPRESS_SIZE_FIELD..], usize::from(size))
        }
        scheme => Err(format!(
            "compressed with unsupported scheme {scheme} (first byte 0x{header:02x})"
        )),
    }
}

/// What a 7-bit unit restores to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Byte,
    Utf16,
}

fn seven_bit(header: u8, payload: &[u8], unit: Unit) -> Vec<u8> {
    let Some(whole_bytes) = payload.len().checked_sub(1) else {
        return Vec::new();
    };
    let bits = whole_bytes * 8 + usize::from(header & LAST_BYTE_BITS_MASK) + 1;
    let count = bits / UNIT_BITS;
    let mut output = Vec::with_capacity(count * 2);
    let mut bytes = payload.iter();
    let (mut buffer, mut buffered) = (0u32, 0);
    for _ in 0..count {
        while buffered < UNIT_BITS {
            buffer |= u32::from(bytes.next().copied().unwrap_or_default()) << buffered;
            buffered += 8;
        }
        output.push((buffer & UNIT_MASK) as u8);
        if unit == Unit::Utf16 {
            output.push(0);
        }
        buffer >>= UNIT_BITS;
        buffered -= UNIT_BITS;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn seven_bit_unicode() {
        // From the libesedb format documentation: "link", 28 bits in 4 bytes.
        assert_eq!(
            decompress(&[0x13, 0xec, 0xb4, 0x7b, 0x0d]).unwrap(),
            utf16("link")
        );
        // "Users" from a Windows Search database: 35 bits in 5 bytes.
        assert_eq!(
            decompress(&[0x12, 0xd5, 0x79, 0x59, 0x3e, 0x07]).unwrap(),
            utf16("Users")
        );
    }

    #[test]
    fn seven_bit_ascii() {
        assert_eq!(
            decompress(&[0x0b, 0xec, 0xb4, 0x7b, 0x0d]).unwrap(),
            b"link"
        );
        assert_eq!(decompress(&[0x08]).unwrap(), b"");
    }

    #[test]
    fn xpress() {
        let mut data = vec![0x18, 26, 0, 0x3f, 0, 0, 0];
        data.extend(b'a'..=b'z');
        assert_eq!(
            decompress(&data).unwrap(),
            (b'a'..=b'z').collect::<Vec<_>>()
        );
    }

    #[test]
    fn unsupported_and_damaged_values_are_errors() {
        assert!(decompress(&[]).is_err());
        assert!(decompress(&[0x18]).is_err());
        let problem = decompress(&[0x28, 1, 2]).unwrap_err();
        assert!(problem.contains("unsupported scheme 5"), "{problem}");
    }
}
