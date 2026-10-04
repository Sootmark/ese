//! Little-endian integers read without panicking: every reader returns
//! `None` when the bytes run out.

pub(crate) fn u8_at(bytes: &[u8], at: usize) -> Option<u8> {
    bytes.get(at).copied()
}

pub(crate) fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(array_at(bytes, at)?))
}

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(array_at(bytes, at)?))
}

pub(crate) fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(array_at(bytes, at)?))
}

pub(crate) fn array_at<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    slice_at(bytes, at, N)?.try_into().ok()
}

/// `len` bytes from `at`, if `bytes` holds them.
pub(crate) fn slice_at(bytes: &[u8], at: usize, len: usize) -> Option<&[u8]> {
    bytes.get(at..at.checked_add(len)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_are_little_endian() {
        let bytes = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(u16_at(&bytes, 0), Some(0x0201));
        assert_eq!(u32_at(&bytes, 4), Some(0x0807_0605));
        assert_eq!(u64_at(&bytes, 0), Some(0x0807_0605_0403_0201));
    }

    #[test]
    fn readers_past_the_end_are_none() {
        assert_eq!(u32_at(&[1, 2, 3], 0), None);
        assert_eq!(u16_at(&[1, 2], usize::MAX), None);
        assert_eq!(slice_at(&[1, 2], 1, usize::MAX), None);
    }
}
