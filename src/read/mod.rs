use std::io::{Error as IoError, ErrorKind, IoSlice, Result};
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

#[cfg(feature = "streams")]
use futures_core::Stream;
use tokio::io::{AsyncBufRead, AsyncRead, ReadBuf};

use crate::shared::block::{Block, Header};
use crate::shared::buffer::ReadableRegion;
use crate::shared::slices::IntoBuffersIterator;
use crate::shared::state::State;

use crate::{Archive, BLOCK_SIZE, Entry, TRACING_ENABLED};

mod error;
pub use self::error::ReadError;

impl<R: AsyncRead> Archive<R> {
    /// Reads from the source object and fills the internal buffer, until one
    /// of the given stop states is reached. Returns the new state and the offset
    /// into our buffer the transition occurs.
    fn poll_next_state(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        header: Option<&Header>,
    ) -> Poll<Result<(State, usize)>> {
        ready!(self.as_mut().poll_fill_buf(cx))?;

        let this = self.as_mut().project();
        let buf = this.buf.buffered_bytes();
        Poll::Ready(this.state.next(buf, header))
    }

    /// Reads from the source object until the next entry header is received
    /// or EOF is reached.
    ///
    /// This will panic if called while an entry is being read.
    fn poll_next_entry(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<Entry<'_, R>>>> {
        if self.state.is_terminal() {
            return Poll::Ready(Ok(None));
        }

        loop {
            if TRACING_ENABLED {
                eprintln!("     |entry: {:?}", self.state);
            }

            let (state, amt) = ready!(self.as_mut().poll_next_state(cx, None))?;

            match state {
                State::ReceivedHeader => {
                    let this = self.as_mut().project();
                    let buf = this.buf.buffered_bytes();
                    let block = Block::from_bytes(&buf[..BLOCK_SIZE]);
                    let header = block.as_header()?.to_owned();
                    self.as_mut().consume(amt, Some(&header));
                    let entry = Entry::new(self, header)?;
                    return Poll::Ready(Ok(Some(entry)));
                }

                State::ReceivedEof => {
                    self.consume(amt, None);
                    return Poll::Ready(Ok(None));
                }

                State::ReceivingHeader(_, _) | State::ReceivingEof(_) => {
                    self.as_mut().consume(amt, None);
                    continue;
                }

                State::AligningData(_) | State::AlignedData => {
                    // Finishing off a previous entry.
                    self.as_mut().consume(amt, None);
                    continue;
                }

                s => {
                    panic!("cannot read next entry while another entry is being read ({s:?})");
                }
            }
        }
    }

    /// Reads from the source object and returns buffers of entry data until
    /// all entry data is consumed. It is necessary to call [Self::consume]
    /// afterwards in order to get buffers with new data.
    ///
    /// This will panic if called while no entry is being read.
    fn poll_read_entry(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        header: &Header,
    ) -> Poll<Result<&[u8]>> {
        loop {
            if TRACING_ENABLED {
                eprintln!("     |read: {:?}", self.state);
            }

            let (state, amt) = ready!(self.as_mut().poll_next_state(cx, Some(header)))?;

            match state {
                State::ReceivingData(_) | State::ReceivedData => {
                    let this = self.project();
                    let buf = this.buf.buffered_bytes();
                    // Our caller will consume as much as they need.
                    return Poll::Ready(Ok(&buf[..amt]));
                }

                State::AligningData(_) => {
                    self.as_mut().consume(amt, None);
                    continue;
                }

                State::AlignedData => {
                    self.as_mut().consume(amt, Some(header));
                    return Poll::Ready(Ok(&[]));
                }

                s => {
                    // [Entry] prevents us from entering here.
                    unreachable!("cannot read entry: invalid state: {s:?}");
                }
            }
        }
    }

    /// Reads from the source object and consumes all remaining entry data.
    fn poll_skip_entry(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        header: &Header,
    ) -> Poll<Result<()>> {
        loop {
            let buf = ready!(self.as_mut().poll_read_entry(cx, header))?;
            let amt = buf.len();
            if amt == 0 {
                assert_eq!(self.state, State::ExpectingHeader);
                return Poll::Ready(Ok(()));
            }
            self.as_mut().consume(amt, Some(header));
        }
    }

    /// Reads from the source object and fills the internal buffer.
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let mut this = self.project();

        if this.buf.buffered_bytes().is_empty() {
            // Try to fill our buffer
            let buf = this.buf.available_bytes_mut();
            let mut buf = ReadBuf::new(buf);
            ready!(this.io.as_mut().poll_read(cx, &mut buf))?;

            let bytes_read = buf.filled().len();

            // If the underlying reader returned zero bytes, this means that
            // either our buffer is full or we've reached EOF. See which case
            // it is and return an error if this was not expected.
            if bytes_read == 0 {
                assert!(!this.buf.available_bytes_mut().is_empty());
                if !this.state.is_terminal() {
                    let err = IoError::from(ErrorKind::UnexpectedEof);
                    return Poll::Ready(Err(err));
                }
            }

            this.buf.available().commit(bytes_read);
        }

        Poll::Ready(Ok(()))
    }

    /// Consumes `amt` from the internal buffer advancing into the archive
    /// and updating the internal state accordingly.
    fn consume(self: Pin<&mut Self>, amt: usize, header: Option<&Header>) {
        let this = self.project();

        let mut buffered = this.buf.buffered();

        let available = buffered.len();
        assert!(
            available >= amt,
            "cannot consume more than available; amt = {amt}, available = {available}",
        );

        // Actually update our state. This should not fail because every slice
        // within our buffer has already been checked in [Self::poll_next_state].
        let slices = [IoSlice::new(&buffered.bytes()[..amt])];
        let (state, pos) = this
            .state
            .take_slices(slices.iter().into_buffers(), header)
            .expect("this slice should have already been checked");

        // This is a bit of a catch-all for states we may land but don't care
        // to handle as edge cases in the individual methods.
        //
        // More importantly, this handles the case where our owner consumes
        // the last bits of entry data and we need to transition to reading the
        // entry alignment bytes. We can't do that in `poll_next_entry` because
        // we don't have a valid header at that point, so we make that transition
        // here while we have the entry header (since we're being called from
        // [Entry::consume]).
        let state = match state {
            State::ReceivingHeader(0, false)
            | State::ReceivedHeader
            | State::ReceivingData(0)
            | State::ReceivedData
            | State::AligningData(0)
            | State::AlignedData
            | State::ReceivingEof(0) => {
                // This cannot fail because either we don't need the header
                // to make the transition, or the header has been checked
                // to be valid already.
                state.next(&[], header).unwrap().0
            }
            _ => state,
        };

        assert_eq!(
            pos, amt,
            "cannot consume past another entry; amt = {amt}, offset = {pos}",
        );

        if TRACING_ENABLED {
            eprintln!("     | cnsm: {amt} / {:?} -> {state:?}", *this.state);
        }

        *this.state = state;

        // Advance our read pointer
        buffered.commit(amt);

        // Reset our buffer if we've read it all to make as much room
        // as possible for further data.
        if buffered.is_empty() {
            this.buf.clear();
        }
    }
}

impl<R: AsyncRead> AsyncRead for Entry<'_, R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let bytes = ready!(self.as_mut().poll_fill_buf(cx))?;
        let len = bytes.len().min(buf.remaining());
        buf.put_slice(&bytes[..len]);
        self.consume(len);
        Poll::Ready(Ok(()))
    }
}

impl<R: AsyncRead> AsyncBufRead for Entry<'_, R> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        if TRACING_ENABLED {
            eprintln!(" fill: '{}', size = {}", self.path_lossy(), self.size());
        }
        let this = self.project();
        let header = this.header;
        this.archive.as_mut().poll_read_entry(cx, header)
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        if TRACING_ENABLED {
            eprintln!("consm: '{}', size = {}", self.path_lossy(), self.size());
        }
        let this = self.project();
        let header = this.header;
        this.archive.as_mut().consume(amt, Some(header));
    }
}

impl<R: AsyncRead> Entry<'_, R> {
    pub(super) fn poll_skip(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if TRACING_ENABLED {
            eprintln!(" skip: '{}', size = {}", self.path_lossy(), self.size());
        }
        let this = self.project();
        let header = this.header;
        this.archive.as_mut().poll_skip_entry(cx, header)
    }
}

#[derive(Debug)]
pub struct NextEntry<'a, R>(&'a mut Archive<R>);

impl<'a, R> NextEntry<'a, R> {
    pub(super) fn new(archive: &'a mut Archive<R>) -> Self {
        Self(archive)
    }
}

impl<'a, R> Future for NextEntry<'a, R>
where
    R: AsyncRead + Unpin,
{
    type Output = Result<Option<Entry<'a, R>>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = Pin::new(&mut *self.get_mut().0);
        if let Some(entry) = ready!(pin.poll_next_entry(cx))? {
            let entry = unsafe {
                // We have an exclusive reference to archive for 'a.
                mem::transmute::<Entry<'_, R>, Entry<'a, R>>(entry)
            };
            Poll::Ready(Ok(Some(entry)))
        } else {
            Poll::Ready(Ok(None))
        }
    }
}

#[cfg(feature = "streams")]
#[derive(Debug)]
pub struct Entries<'a, R>(&'a mut Archive<R>);

#[cfg(feature = "streams")]
impl<'a, R> Entries<'a, R> {
    pub(super) fn new(archive: &'a mut Archive<R>) -> Self {
        Self(archive)
    }
}

#[cfg(feature = "streams")]
impl<'a, R: AsyncRead + Unpin> Stream for Entries<'a, R> {
    type Item = Result<Entry<'a, R>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut pin = unsafe {
            // The original Archive exists elsewhere and is guaranteed to
            // have a stable location for 'a. Besides, we are only polling
            // it through the reference and never move the value.
            self.map_unchecked_mut(|s| s.0)
        };

        if let Some(entry) = ready!(pin.as_mut().poll_next_entry(cx))? {
            let entry = unsafe {
                // We have an exclusive reference for 'a.
                mem::transmute::<Entry<'_, R>, Entry<'a, R>>(entry)
            };
            Poll::Ready(Some(Ok(entry)))
        } else {
            Poll::Ready(None)
        }
    }
}

#[cfg(test)]
mod tests;
