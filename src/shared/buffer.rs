use std::fmt;

pub struct Buf {
    buf: Box<[u8]>,

    /// The write pointer, incremented by writing into the buffer.
    /// `cap` determines the capacity of the buffer returned by [Self::buffered].
    cap: usize,

    /// The read pointer, incremented by reading from the buffer.
    /// It must always hold that `pos <= cap`.
    pos: usize,
}

impl fmt::Debug for Buf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Buf")
            .field("buf", &self.buf.len())
            .field("cap", &self.cap)
            .field("pos", &self.pos)
            .finish()
    }
}

impl Buf {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0u8; capacity].into_boxed_slice(),
            cap: 0,
            pos: 0,
        }
    }

    /// The number of bytes that can fit in this buffer.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Empties the buffer. Does not touch bytes in the buffer, however, after
    /// calling `clear()` both `buffered().is_empty()` and `available().is_empty()`
    /// will report `true`.
    #[inline]
    pub fn clear(&mut self) {
        self.cap = 0;
        self.pos = 0;
    }

    /// The region of the buffer extending from the start of the buffer to the
    /// end of the written region. Use [Self::available] to write data into
    /// that region.
    #[inline]
    pub fn buffered(&mut self) -> Region<'_> {
        Region {
            buf: &self.buf[..self.cap],
            pos: &mut self.pos,
        }
    }

    /// Same as `self.buffered().bytes()` without taking an exclusive reference
    /// to self or the lifetime limitations due to the `Region` temporary.
    #[inline]
    pub fn buffered_bytes(&self) -> &[u8] {
        &self.buf[self.pos..self.cap]
    }

    /// Data written into this region becomes available for reading through
    /// [Self::buffered].
    #[inline]
    pub fn available(&mut self) -> RegionMut<'_> {
        RegionMut {
            buf: &mut self.buf,
            pos: &mut self.cap,
        }
    }

    /// Same as `self.available().bytes_mut()`.
    #[inline]
    pub fn available_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf[self.cap..]
    }
}

pub trait ReadableRegion {
    /// A reference to the complete backing buffer.
    fn buf(&self) -> &[u8];

    /// The number of bytes in the buffer.
    fn len(&self) -> usize;

    /// The read/write cursor counting from the start of the buffer,
    /// up to [Self::capacity].
    fn position(&self) -> usize;
    fn set_position(&mut self, pos: usize);

    /// The total number of bytes that can fit in the buffer.
    #[inline]
    fn capacity(&self) -> usize {
        self.buf().len()
    }

    /// The free space of the buffer.
    #[inline]
    fn remaining(&self) -> usize {
        self.capacity() - self.position()
    }

    /// Whether there are no bytes in the buffer.
    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Advances [Self::position] by the given number of bytes.
    #[inline]
    fn commit(&mut self, amt: usize) {
        let pos = self.position();
        self.set_position(pos.checked_add(amt).expect("buffer cursor overflow"));
    }

    /// The slice of remaining bytes up to [Self::capacity].
    #[inline]
    fn bytes(&self) -> &[u8] {
        let start = self.position();
        &self.buf()[start..]
    }
}

pub trait WritableRegion: ReadableRegion {
    /// A mutable reference to the complete backing buffer.
    fn buf_mut(&mut self) -> &mut [u8];

    /// The slice of remaining bytes up to [Self::capacity], as mutable.
    #[inline]
    fn bytes_mut(&mut self) -> &mut [u8] {
        let start = self.position();
        &mut self.buf_mut()[start..]
    }

    fn fill(&mut self, slice: &[u8]) -> usize {
        let max = self.remaining();
        let len = slice.len().min(max);
        if len > 0 {
            self.bytes_mut()[..len].copy_from_slice(&slice[..len]);
            self.commit(len);
        }
        len
    }

    #[inline]
    fn fill_from_slices<'a, I>(&mut self, slices: I) -> usize
    where
        I: IntoIterator<Item = &'a [u8]>,
    {
        slices.into_iter().fold(0usize, |len, s| len + self.fill(s))
    }
}

pub struct Region<'a> {
    buf: &'a [u8],
    pos: &'a mut usize,
}

impl fmt::Debug for Region<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Region")
            .field("buf", &self.buf.len())
            .field("pos", &self.pos)
            .field("capacity", &self.capacity())
            .finish()
    }
}

pub struct RegionMut<'a> {
    buf: &'a mut [u8],
    pos: &'a mut usize,
}

impl fmt::Debug for RegionMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegionMut")
            .field("buf", &self.buf.len())
            .field("pos", &self.pos)
            .field("capacity", &self.capacity())
            .finish()
    }
}

impl ReadableRegion for Region<'_> {
    #[inline]
    fn buf(&self) -> &[u8] {
        self.buf
    }

    #[inline]
    fn len(&self) -> usize {
        self.remaining()
    }

    #[inline]
    fn position(&self) -> usize {
        *self.pos
    }

    #[inline]
    fn set_position(&mut self, pos: usize) {
        *self.pos = pos;
    }
}

impl ReadableRegion for RegionMut<'_> {
    #[inline]
    fn buf(&self) -> &[u8] {
        self.buf
    }

    #[inline]
    fn len(&self) -> usize {
        self.position()
    }

    #[inline]
    fn position(&self) -> usize {
        *self.pos
    }

    #[inline]
    fn set_position(&mut self, pos: usize) {
        *self.pos = pos;
    }
}

impl WritableRegion for RegionMut<'_> {
    #[inline]
    fn buf_mut(&mut self) -> &mut [u8] {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn fill_buf(buf: &mut Buf) {
        let data = [0, 1, 2, 3];
        let mut available = buf.available();
        available.fill(&data);
    }

    fn test_writable<W: WritableRegion>(mut wr: W) {
        let data = [0, 1, 2, 3];

        assert_eq!(wr.len(), 0);
        assert_eq!(wr.position(), 0);
        assert_eq!(wr.remaining(), 5);
        assert_eq!(wr.capacity(), 5);
        assert!(wr.is_empty());

        let n = wr.bytes_mut().write(&data).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&wr.bytes(), &[0, 1, 2, 3, 0]);

        wr.commit(1);
        assert_eq!(wr.bytes(), &[1, 2, 3, 0]);
        assert_eq!(wr.capacity(), 5);
        assert_eq!(wr.remaining(), 4);
        assert_eq!(wr.position(), 1);
        assert_eq!(wr.len(), 1);
        assert!(!wr.is_empty());

        wr.commit(3);
        assert_eq!(wr.bytes(), &[0]);
        assert_eq!(wr.capacity(), 5);
        assert_eq!(wr.remaining(), 1);
        assert_eq!(wr.position(), 4);
        assert_eq!(wr.len(), 4);
        assert!(!wr.is_empty());

        wr.commit(1);
        assert_eq!(wr.bytes(), &[]);
        assert_eq!(wr.capacity(), 5);
        assert_eq!(wr.remaining(), 0);
        assert_eq!(wr.position(), 5);
        assert_eq!(wr.len(), 5);
        assert!(!wr.is_empty());
    }

    fn test_readable<R: ReadableRegion>(mut rd: R) {
        assert_eq!(rd.bytes(), &[0, 1, 2, 3]);
        assert_eq!(rd.capacity(), 4);
        assert_eq!(rd.remaining(), 4);
        assert_eq!(rd.position(), 0);
        assert_eq!(rd.len(), 4);
        assert!(!rd.is_empty());

        rd.commit(1);
        assert_eq!(rd.bytes(), &[1, 2, 3]);
        assert_eq!(rd.capacity(), 4);
        assert_eq!(rd.remaining(), 3);
        assert_eq!(rd.position(), 1);
        assert_eq!(rd.len(), 3);
        assert!(!rd.is_empty());

        rd.commit(2);
        assert_eq!(rd.bytes(), &[3]);
        assert_eq!(rd.capacity(), 4);
        assert_eq!(rd.remaining(), 1);
        assert_eq!(rd.position(), 3);
        assert_eq!(rd.len(), 1);
        assert!(!rd.is_empty());

        rd.commit(1);
        assert_eq!(rd.bytes(), &[]);
        assert_eq!(rd.capacity(), 4);
        assert_eq!(rd.remaining(), 0);
        assert_eq!(rd.position(), 4);
        assert_eq!(rd.len(), 0);
        assert!(rd.is_empty());
    }

    #[test]
    fn available() {
        let mut buf = Buf::new(5);
        let available = buf.available();
        test_writable(available);

        buf.clear();
        let available = buf.available();
        assert_eq!(available.bytes(), &[0, 1, 2, 3, 0]);
        assert_eq!(available.capacity(), 5);
        assert_eq!(available.remaining(), 5);
        assert_eq!(available.position(), 0);
        assert_eq!(available.len(), 0);
        assert!(available.is_empty());
    }

    #[test]
    fn buffered() {
        let mut buf = Buf::new(5);
        fill_buf(&mut buf);

        let buffered = buf.buffered();
        test_readable(buffered);

        buf.clear();
        let buffered = buf.buffered();
        assert_eq!(buffered.bytes(), &[]);
        assert_eq!(buffered.capacity(), 0);
        assert_eq!(buffered.remaining(), 0);
        assert_eq!(buffered.position(), 0);
        assert_eq!(buffered.len(), 0);
        assert!(buffered.is_empty());
    }

    #[test]
    fn available_and_buffered() {
        let data = [0, 1, 2, 3];
        let mut buf = Buf::new(5);
        let mut available = buf.available();
        let n = available.bytes_mut().write(&data).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&available.bytes(), &[0, 1, 2, 3, 0]);
        available.commit(4);
        assert_eq!(&available.bytes(), &[0]);

        let mut buffered = buf.buffered();
        assert_eq!(&buffered.bytes(), &[0, 1, 2, 3]);
        buffered.commit(1);
        assert_eq!(&buffered.bytes(), &[1, 2, 3]);

        let available = buf.available();
        assert_eq!(available.capacity(), 5);
        assert_eq!(available.remaining(), 1);
        assert_eq!(available.position(), 4);
        assert_eq!(available.len(), 4);
        assert_eq!(available.bytes(), &[0]);
        assert!(!available.is_empty());
        buf.clear();

        let buffered = buf.buffered();
        assert!(buffered.is_empty());
        assert_eq!(&buffered.bytes(), &[]);
    }
}
