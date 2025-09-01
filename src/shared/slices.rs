//! Utility types for working with arrays of [io::IoSlice] without copying.

use std::io::IoSlice;

pub trait IterBuffers {
    fn iter_buffers(&self) -> impl Iterator<Item = &[u8]>;
}

pub trait IterSlices {
    fn iter_slices(&self) -> impl Iterator<Item = &IoSlice>;
}

pub trait IterSlicesExt {
    fn bytes_len(self) -> usize;
}

pub trait IntoBuffersIterator {
    fn into_buffers(self) -> IntoBuffersIter<Self>;
}

pub trait Slices: IterSlices + IterBuffers {
    fn as_prefix(&self) -> Prefix;
    fn split_at_index(&self, index: usize, offset: usize) -> (Prefix, Suffix);

    fn split_at_byte_offset(&self, offset: usize) -> (Prefix, Suffix) {
        let mut rem = offset;

        self.iter_slices()
            .enumerate()
            .find_map(|(index, slice)| {
                let len = slice.len();
                if rem >= len {
                    rem -= len;
                    None
                } else {
                    Some((index, rem))
                }
            })
            .map(|(index, offset)| self.split_at_index(index, offset))
            .unwrap_or_else(|| (self.as_prefix(), Suffix::empty()))
    }

    #[inline]
    fn bytes_len(&self) -> usize {
        self.iter_slices().bytes_len()
    }

    #[inline]
    fn take_prefix(&self, len: usize) -> Prefix {
        self.split_at_byte_offset(len).0
    }
}

impl<'a> IterSlices for &'a [IoSlice<'a>] {
    #[inline]
    fn iter_slices(&self) -> impl Iterator<Item = &IoSlice> {
        self.iter()
    }
}

impl<'a> IterBuffers for &'a [IoSlice<'a>] {
    #[inline]
    fn iter_buffers(&self) -> impl Iterator<Item = &[u8]> {
        self.iter_slices().into_buffers()
    }
}

impl<'a> Slices for &'a [IoSlice<'a>] {
    #[inline]
    fn as_prefix(&self) -> Prefix {
        Prefix::from_parts(self, &[])
    }

    fn split_at_index(&self, index: usize, offset: usize) -> (Prefix, Suffix) {
        let (prefix, suffix) = self.split_at(index);
        if offset == 0 {
            (
                Prefix::from_parts(prefix, &[]),
                Suffix::from_parts(suffix, &[]),
            )
        } else {
            let (buf, suffix) = suffix.split_at(1);
            (
                Prefix::from_parts(prefix, &buf[0][..offset]),
                Suffix::from_parts(suffix, &buf[0][offset..]),
            )
        }
    }
}

#[derive(Debug)]
pub struct Prefix<'a>(SplitInner<'a>);

impl IterSlices for Prefix<'_> {
    #[inline]
    fn iter_slices(&self) -> impl Iterator<Item = &IoSlice> {
        self.0
            .slices()
            .iter()
            .chain(self.0.remainder_slices().iter())
    }
}

impl IterBuffers for Prefix<'_> {
    #[inline]
    fn iter_buffers(&self) -> impl Iterator<Item = &[u8]> {
        self.iter_slices().into_buffers()
    }
}

impl<'a> Slices for Prefix<'a> {
    #[inline]
    fn as_prefix(&self) -> Prefix {
        Prefix::from_parts(self.slices(), self.remainder())
    }

    fn split_at_index(&self, index: usize, offset: usize) -> (Prefix, Suffix) {
        if index == self.0.slices().len() {
            // index points to our remainder buffer
            (
                Prefix::from_parts(self.slices(), &self.remainder()[..offset]),
                Suffix::from_parts([].as_slice(), &self.remainder()[offset..]),
            )
        } else {
            // FIXME: This could be simplified as `self.0.slices().split_at_index(..)`
            // but the borrow checker complains about the borrowed slices.
            let (prefix, suffix) = self.0.slices().split_at(index);
            if offset == 0 {
                (
                    Prefix::from_parts(prefix, &[]),
                    Suffix::from_parts(suffix, &[]),
                )
            } else {
                let (buf, suffix) = suffix.split_at(1);
                (
                    Prefix::from_parts(prefix, &buf[0][..offset]),
                    Suffix::from_parts(suffix, &buf[0][offset..]),
                )
            }
        }
    }
}

#[derive(Debug)]
pub struct Suffix<'a>(SplitInner<'a>);

impl IterSlices for Suffix<'_> {
    #[inline]
    fn iter_slices(&self) -> impl Iterator<Item = &IoSlice> {
        self.0
            .remainder_slices()
            .iter()
            .chain(self.0.slices().iter())
    }
}

impl IterBuffers for Suffix<'_> {
    #[inline]
    fn iter_buffers(&self) -> impl Iterator<Item = &[u8]> {
        self.iter_slices().into_buffers()
    }
}

pub trait Split<'a>: IterSlices + IterBuffers {
    fn from_parts(slices: &'a [IoSlice], remainder: &'a [u8]) -> Self;
    fn slices(&self) -> &[IoSlice];
    fn remainder_slices(&self) -> &[IoSlice; 1];

    #[inline]
    fn remainder(&self) -> &[u8] {
        &self.remainder_slices()[0]
    }

    #[inline]
    fn empty() -> Self
    where
        Self: Sized,
    {
        Self::from_parts([].as_slice(), &[])
    }
}

impl<'a> Split<'a> for Prefix<'a> {
    #[inline]
    fn from_parts(slices: &'a [IoSlice], remainder: &'a [u8]) -> Self {
        Self(SplitInner(slices, [IoSlice::new(remainder)]))
    }

    #[inline]
    fn slices(&self) -> &[IoSlice] {
        self.0.slices()
    }

    #[inline]
    fn remainder_slices(&self) -> &[IoSlice; 1] {
        self.0.remainder_slices()
    }
}

impl<'a> Split<'a> for Suffix<'a> {
    #[inline]
    fn from_parts(slices: &'a [IoSlice], remainder: &'a [u8]) -> Self {
        Self(SplitInner(slices, [IoSlice::new(remainder)]))
    }

    #[inline]
    fn slices(&self) -> &[IoSlice] {
        self.0.slices()
    }

    #[inline]
    fn remainder_slices(&self) -> &[IoSlice; 1] {
        self.0.remainder_slices()
    }
}

#[derive(Debug)]
struct SplitInner<'a>(&'a [IoSlice<'a>], [IoSlice<'a>; 1]);
impl SplitInner<'_> {
    #[inline]
    fn slices(&self) -> &[IoSlice] {
        self.0
    }

    #[inline]
    fn remainder_slices(&self) -> &[IoSlice; 1] {
        &self.1
    }
}

impl<'a, I> IntoBuffersIterator for I
where
    I: Iterator<Item = &'a IoSlice<'a>>,
{
    #[inline]
    fn into_buffers(self) -> IntoBuffersIter<Self> {
        IntoBuffersIter(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IntoBuffersIter<I: ?Sized>(I);
impl<'a, I> Iterator for IntoBuffersIter<I>
where
    I: Iterator<Item = &'a IoSlice<'a>>,
{
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(slice) = self.0.next() {
            Some(&slice[..])
        } else {
            None
        }
    }
}

impl<'a, I> IterSlicesExt for I
where
    I: Iterator<Item = &'a IoSlice<'a>>,
{
    #[inline]
    fn bytes_len(self) -> usize {
        self.fold(0usize, |acc, b| acc.saturating_add(b.len()))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TakeBytesLen<I>(I, usize);
impl<'a, I> Iterator for TakeBytesLen<I>
where
    I: Iterator<Item = &'a [u8]>,
{
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.1 == 0 {
            return None;
        }
        match self.0.next() {
            Some(buf) => {
                let len = buf.len();

                Some(if self.1 >= len {
                    self.1 -= len;
                    buf
                } else {
                    let buf = &buf[..self.1];
                    self.1 = 0;
                    buf
                })
            }
            None => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SkipBytesLen<I>(I, usize);
impl<'a, I> Iterator for SkipBytesLen<I>
where
    I: Iterator<Item = &'a [u8]>,
{
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.1 == 0 {
            return self.0.next();
        }

        let mut rem = self.1;

        loop {
            match self.0.next() {
                Some(buf) => {
                    let len = buf.len();
                    if rem >= len {
                        rem -= len;
                        continue;
                    } else {
                        self.1 = 0;
                        return Some(&buf[rem..]);
                    }
                }
                None => {
                    self.1 = 0;
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Data = [[u8; 5]; 5];

    const DATA: Data = [
        [0, 1, 2, 3, 4],
        [5, 6, 7, 8, 9],
        [10, 11, 12, 13, 14],
        [15, 16, 17, 18, 19],
        [20, 21, 22, 23, 24],
    ];

    fn make_data() -> Vec<IoSlice<'static>> {
        Vec::from_iter(DATA.iter().map(|s| IoSlice::new(s)))
    }

    fn assert_slice_eq(left: &IoSlice<'_>, idx: usize) {
        assert_eq!(left.as_ref(), &DATA[idx]);
    }

    #[test]
    fn basic() {
        let data = make_data();
        let slices = data.as_slice();
        assert_eq!(slices.bytes_len(), 25);

        let (prefix, suffix) = slices.split_at_byte_offset(10);
        assert_eq!(prefix.slices().len(), 2);
        assert_eq!(suffix.slices().len(), 3);
        assert_eq!(prefix.remainder().len(), 0);
        assert_eq!(suffix.remainder().len(), 0);
        assert_slice_eq(&prefix.slices()[0], 0);
        assert_slice_eq(&prefix.slices()[1], 1);
        assert_slice_eq(&suffix.slices()[0], 2);
        assert_slice_eq(&suffix.slices()[1], 3);
        assert_slice_eq(&suffix.slices()[2], 4);

        let slices = &slices[..2];
        let (prefix, suffix) = slices.split_at_byte_offset(10);
        assert_eq!(prefix.slices().len(), 2);
        assert_eq!(suffix.slices().len(), 0);
        assert_eq!(prefix.remainder().len(), 0);
        assert_eq!(suffix.remainder().len(), 0);
        assert_slice_eq(&prefix.slices()[0], 0);
        assert_slice_eq(&prefix.slices()[1], 1);

        let slices = &slices[..1];
        let (prefix, suffix) = slices.split_at_byte_offset(10);
        assert_eq!(prefix.slices().len(), 1);
        assert_eq!(suffix.slices().len(), 0);
        assert_eq!(prefix.remainder().len(), 0);
        assert_eq!(suffix.remainder().len(), 0);
        assert_slice_eq(&prefix.slices()[0], 0);
    }
}
