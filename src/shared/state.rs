use std::fmt;
use std::io::{Error as IoError, ErrorKind, Result};

use crate::TRACING_ENABLED;

use super::block::{BLOCK_SIZE, Header};

#[derive(Debug)]
pub enum Error {
    ExpectingHeader,
    ExpectingEmptyBlock,
    Eof,
}

impl Error {
    #[inline]
    pub fn kind(&self) -> ErrorKind {
        ErrorKind::InvalidData
    }
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectingHeader => "expecting header".fmt(f),
            Self::ExpectingEmptyBlock => "expecting empty block".fmt(f),
            Self::Eof => "cannot process data after eof".fmt(f),
        }
    }
}

impl From<Error> for IoError {
    #[inline]
    fn from(value: Error) -> Self {
        IoError::new(value.kind(), value)
    }
}

impl<T> From<Error> for Result<T> {
    #[inline]
    fn from(value: Error) -> Self {
        Err(value.into())
    }
}

/// A type that can validate and represent any point in a TAR byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    ExpectingHeader,
    ReceivingHeader(usize, bool),
    ReceivedHeader,
    ReceivingData(u64),
    ReceivedData,
    AligningData(usize),
    AlignedData,
    ReceivingEof(usize),
    ReceivedEof,
}

impl Default for State {
    #[inline]
    fn default() -> Self {
        Self::ExpectingHeader
    }
}

impl State {
    #[inline]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::ReceivedEof | Self::ReceivingEof(0))
    }

    #[inline]
    pub fn is_marker(&self) -> bool {
        matches!(
            self,
            Self::ExpectingHeader
                | Self::ReceivedHeader
                | Self::ReceivedData
                | Self::AlignedData
                | Self::ReceivedEof
        )
    }

    /// Transitions from a marker state to the next overlapping regular state.
    /// This is like calling [Self::next] with an empty buffer.
    ///
    /// Will panic if the current state is not a marker state.
    #[inline]
    pub fn take_marker(&mut self, header: Option<&Header>) -> Result<()> {
        assert!(self.is_marker(), "not a marker: {self:?}");
        let (state, pos) = self.next(&[], header)?;
        debug_assert_eq!(pos, 0);
        *self = state;
        Ok(())
    }

    /// Takes each slice in order and transitions states as needed. Returns
    /// the final state and number of bytes read. Returns early if another
    /// header is received or EOF is reached.
    #[inline]
    pub fn take_slices<'a, I>(self, slices: I, hdr: Option<&Header>) -> Result<(Self, usize)>
    where
        I: Iterator<Item = &'a [u8]>,
    {
        let stop = [Self::ReceivedHeader, Self::ReceivedEof];
        let mut needs_next = true;
        let mut state = self;
        let mut cur = 0usize;

        for buf in slices {
            if buf.is_empty() {
                continue;
            }

            let next = state.take_until(&stop, buf, hdr)?;
            state = next.0;
            cur += next.1;
            needs_next = false;

            if stop.contains(&state) {
                break;
            }
        }

        if needs_next {
            // Make sure to call next at least once to ensure forward progress.
            let next = state.take_until(&stop, &[], hdr)?;
            state = next.0;
            cur += next.1;
        }

        Ok((state, cur))
    }

    /// Transitions states until one of the given stop states is reached or
    /// the buffer is exhausted, and returns the state and number of bytes read.
    #[inline]
    pub fn take_until(
        self,
        stop: &[Self],
        buf: &[u8],
        header: Option<&Header>,
    ) -> Result<(Self, usize)> {
        let mut state = self;
        let mut cur = 0usize;
        let mut buf = buf;

        // Call next at least once to ensure forward progress.
        loop {
            let next = state.next(buf, header)?;

            state = next.0;
            cur += next.1;
            buf = &buf[next.1..];

            if stop.contains(&state) || buf.is_empty() {
                break;
            }
        }

        Ok((state, cur))
    }

    /// Transitions to the next state and returns the state and number of
    /// bytes read.
    ///
    /// An empty buffer, despite being empty, will still lead to a state
    /// transition around a marker.
    pub fn next(self, buf: &[u8], header: Option<&Header>) -> Result<(Self, usize)> {
        fn advance(buf: &[u8], max: usize) -> usize {
            max.min(buf.len())
        }

        fn read(buf: &[u8], rem: usize) -> (usize, bool) {
            let len = advance(buf, rem);
            let empty = buf[..len].iter().all(|b| *b == 0);
            (len, empty)
        }

        let mut cur = 0usize;

        // Advance into the buffer looking for a state transition and return
        // the new state plus the offset into the buffer the transition occurs.
        let state = match self {
            Self::ExpectingHeader => Self::ReceivingHeader(BLOCK_SIZE, true),

            Self::ReceivingHeader(mut rem, mut is_empty) => {
                let (len, empty) = read(buf, rem);
                cur += len;
                rem -= len;
                is_empty = is_empty && empty;

                if rem == 0 {
                    if is_empty {
                        // Received first of two empty blocks that signify EOF.
                        Self::ReceivingEof(BLOCK_SIZE)
                    } else {
                        // Received header of next entry. Assume it is valid.
                        Self::ReceivedHeader
                    }
                } else {
                    // Waiting for header of next entry.
                    Self::ReceivingHeader(rem, is_empty)
                }
            }

            Self::ReceivedHeader => {
                assert!(header.is_some(), "header cannot be nil for state {self:?}");
                let rem = header.unwrap().entry_size()?;
                Self::ReceivingData(rem)
            }

            Self::ReceivingData(mut rem) => {
                let len = advance(buf, rem as usize);
                cur += len;
                rem -= len as u64;

                if rem == 0 {
                    // Completed reading entry data.
                    Self::ReceivedData
                } else {
                    // Reading entry data.
                    Self::ReceivingData(rem)
                }
            }

            Self::ReceivedData => {
                assert!(header.is_some(), "header cannot be nil for state {self:?}");
                let rem = header.unwrap().entry_size()?;
                let align = rem.next_multiple_of(BLOCK_SIZE as u64) - rem;
                Self::AligningData(align as usize)
            }

            Self::AligningData(mut rem) => {
                let len = advance(buf, rem);
                cur += len;
                rem -= len;

                if rem == 0 {
                    // Completed aligning entry data.
                    Self::AlignedData
                } else {
                    // Aligning entry data.
                    Self::AligningData(rem)
                }
            }

            Self::AlignedData => Self::ExpectingHeader,

            Self::ReceivingEof(mut rem) => {
                let (len, empty) = read(buf, rem);
                cur += len;
                rem -= len;

                if !empty {
                    // Received malformed data
                    return Error::ExpectingEmptyBlock.into();
                }

                if rem == 0 {
                    // Received second empty block that signifies EOF.
                    Self::ReceivedEof
                } else {
                    // Waiting for the second empty block.
                    Self::ReceivingEof(rem)
                }
            }

            // Received malformed data
            Self::ReceivedEof => return Error::Eof.into(),
        };

        if TRACING_ENABLED {
            eprintln!("     | next: {self:?} -> {state:?}");
        }

        Ok((state, cur))
    }
}

#[cfg(test)]
mod tests {
    use crate::shared::block::Block;
    use crate::shared::test::*;

    use super::*;

    #[test]
    fn basic() {
        let data = make_archive_data(&[("1000", 1000)]);
        let mut hdr: Option<&Header> = None;

        let state = State::default();
        assert_eq!(state, State::ExpectingHeader);
        let d = &data[..];

        let (state, pos) = state.next(d, hdr).unwrap();
        assert_eq!(state, State::ReceivingHeader(BLOCK_SIZE, true));
        assert_eq!(pos, 0);

        let n = 250usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::ReceivingHeader(BLOCK_SIZE - n, false));
        assert_eq!(pos, n);
        let d = &d[n..];

        {
            // test that the state transition can be identified midway through the buffer
            let n = 300usize;
            let (state, pos) = state.next(&d[..n], hdr).unwrap();
            assert_eq!((state, pos), (State::ReceivedHeader, 262));
        }

        let n = 262usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::ReceivedHeader);
        assert_eq!(pos, n);
        hdr = Some(Block::from_bytes(&data[..BLOCK_SIZE]).as_header().unwrap());
        let d = &d[n..];

        let (state, pos) = state.next(d, hdr).unwrap();
        assert_eq!(state, State::ReceivingData(1000));
        assert_eq!(pos, 0);

        let n = 500usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::ReceivingData((1000 - n) as u64));
        assert_eq!(pos, n);
        let d = &d[n..];

        let n = 500usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::ReceivedData);
        assert_eq!(pos, n);
        let d = &d[n..];

        let (state, pos) = state.next(d, hdr).unwrap();
        assert_eq!(state, State::AligningData(24));
        assert_eq!(pos, 0);

        let n = 10usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::AligningData(14));
        assert_eq!(pos, n);
        let d = &d[n..];

        let n = 14usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::AlignedData);
        assert_eq!(pos, n);
        hdr = None;
        let d = &d[n..];

        let (state, pos) = state.next(d, hdr).unwrap();
        assert_eq!(state, State::ExpectingHeader);
        assert_eq!(pos, 0);

        let (state, pos) = state.next(d, hdr).unwrap();
        assert_eq!(state, State::ReceivingHeader(BLOCK_SIZE, true));
        assert_eq!(pos, 0);

        let n = 256usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::ReceivingHeader(BLOCK_SIZE - n, true));
        assert_eq!(pos, n);
        let d = &d[n..];

        {
            // test that the state transition can be identified midway through the buffer
            let n = 356usize;
            let (state, pos) = state.next(&d[..n], hdr).unwrap();
            assert_eq!(state, State::ReceivingEof(BLOCK_SIZE));
            assert_eq!(pos, n - 100);
        }

        let n = 256usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::ReceivingEof(BLOCK_SIZE));
        assert_eq!(pos, n);
        let d = &d[n..];

        let n = 256usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::ReceivingEof(BLOCK_SIZE - n));
        assert_eq!(pos, n);
        let d = &d[n..];

        let n = 256usize;
        let (state, pos) = state.next(&d[..n], hdr).unwrap();
        assert_eq!(state, State::ReceivedEof);
        assert_eq!(pos, n);
        let d = &d[n..];

        assert_eq!(d.len(), 0);
    }
}
