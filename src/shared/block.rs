use std::any;
use std::fmt;
use std::io;
use std::mem;

pub use tar::Header;

/// A TAR byte stream is a series of 512-byte blocks.
pub const BLOCK_SIZE: usize = 512;

const EMPTY_BLOCK: [u8; BLOCK_SIZE] = [0u8; BLOCK_SIZE];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct Block {
    bytes: [u8; BLOCK_SIZE],
}

impl Default for Block {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let h = self.as_header();
        if h.is_ok() {
            let h = h.unwrap();
            f.debug_struct("Block")
                .field("is_header", &true)
                .field("path", &h.path())
                .field("size", &h.size())
                .field("len", &h.entry_size())
                .field("cksum", &h.cksum())
                .field("bytes", &self.bytes)
                .finish()
        } else {
            f.debug_struct("Block")
                .field("is_header", &false)
                .field("bytes", &self.bytes)
                .finish()
        }
    }
}

impl Block {
    #[inline]
    pub const fn new() -> Self {
        Self { bytes: EMPTY_BLOCK }
    }

    #[inline]
    pub fn empty() -> &'static Self {
        Self::from_bytes(&EMPTY_BLOCK)
    }

    /// Converts a slice of bytes into a [Block] reference without copying.
    /// Panics if the slice is not of the correct size.
    #[inline]
    pub fn from_bytes(value: &[u8]) -> &Self {
        unsafe { cast_bytes(value) }
    }

    #[inline]
    pub const fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[inline]
    pub fn as_header(&self) -> io::Result<&Header> {
        let header: &Header = unsafe { cast(&self.bytes) };
        let expected = header.cksum()?;
        let actual = calc_cksum(&self.bytes);
        if expected == actual {
            Ok(header)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "expected block to be a valid header; checksum expected = {expected}, actual = {actual};",
                ),
            ))
        }
    }
}

fn calc_cksum(bytes: &[u8; BLOCK_SIZE]) -> u32 {
    bytes[..148]
        .iter()
        .chain(&bytes[156..])
        .fold(0, |a, b| a + (*b as u32))
        + 8 * 32
}

unsafe fn cast_bytes<U>(bytes: &[u8]) -> &U {
    assert_eq!(
        bytes.len(),
        mem::size_of::<U>(),
        "left: {} right: {}",
        any::type_name::<&[u8]>(),
        any::type_name::<U>()
    );
    assert_eq!(
        mem::align_of_val(bytes),
        mem::align_of::<U>(),
        "left: {} right: {}",
        any::type_name::<&[u8]>(),
        any::type_name::<U>()
    );
    unsafe { &*(bytes.as_ptr() as *const U) }
}

unsafe fn cast<T, U>(a: &T) -> &U {
    assert_eq!(
        mem::size_of_val(a),
        mem::size_of::<U>(),
        "left: {} right: {}",
        any::type_name::<T>(),
        any::type_name::<U>()
    );
    assert_eq!(
        mem::align_of_val(a),
        mem::align_of::<U>(),
        "left: {} right: {}",
        any::type_name::<T>(),
        any::type_name::<U>()
    );
    unsafe { &*(a as *const T as *const U) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_header() {
        let buf: [u8; BLOCK_SIZE] = [
            53, 48, 48, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 48, 48, 48, 48, 48, 48, 48, 48, 55, 54, 52, 0, 48, 48, 48,
            48, 48, 48, 48, 48, 48, 48, 48, 0, 48, 48, 48, 52, 49, 50, 53, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 117, 115, 116, 97, 114, 0, 48, 48, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0,
        ];
        Block::from_bytes(&buf).as_header().unwrap();
    }
}
