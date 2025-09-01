use std::io::{IoSlice, Result};
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::AsyncWrite;

use crate::shared::block::{BLOCK_SIZE, Block, Header};
use crate::shared::buffer::{ReadableRegion, WritableRegion};
use crate::shared::slices::{IterBuffers, Slices, Split};
use crate::shared::state::State;

use crate::{Archive, Entry, TRACING_ENABLED};

mod error;
pub use self::error::WriteError;

impl<W: AsyncWrite> Archive<W> {
    pub(super) fn poll_write_header(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        header: &Header,
    ) -> Poll<Result<()>> {
        loop {
            if TRACING_ENABLED {
                eprintln!("     |whead: {:?}", self.state);
            }

            match self.state {
                State::ExpectingHeader => {
                    let buf = header.as_bytes();
                    let n = ready!(self.as_mut().poll_write_data(cx, buf, Some(header)))?;
                    if n == 0 {
                        return WriteError::WriteZero.into();
                    }
                    continue;
                }

                State::ReceivingHeader(rem, false) => {
                    let pos = BLOCK_SIZE - rem;
                    let buf = &header.as_bytes()[pos..];
                    let n = ready!(self.as_mut().poll_write_data(cx, buf, Some(header)))?;
                    if n == 0 && rem > 0 {
                        return WriteError::WriteZero.into();
                    }
                    continue;
                }

                State::ReceivedHeader => {
                    self.project().state.take_marker(Some(header))?;
                    return Poll::Ready(Ok(()));
                }

                State::ReceivingData(_)
                | State::ReceivedData
                | State::AligningData(_)
                | State::AlignedData => {
                    return WriteError::OverlappingEntry.into();
                }

                State::ReceivingHeader(_, true) | State::ReceivingEof(_) | State::ReceivedEof => {
                    panic!("cannot write header; invalid state: {:?}", self.state)
                }
            }
        }
    }

    fn poll_write_entry(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
        header: &Header,
    ) -> Poll<Result<usize>> {
        if TRACING_ENABLED {
            eprintln!("     |write: {:?}", self.state);
        }

        match self.state {
            State::ReceivingData(rem) => {
                let n = ready!(self.as_mut().poll_write_vectored(
                    cx,
                    bufs,
                    rem as usize,
                    Some(header)
                ))?;
                if n as u64 == rem {
                    debug_assert_eq!(bufs.bytes_len(), n);
                    debug_assert_eq!(self.state, State::ReceivedData);
                    let res = ready!(self.poll_finish_entry(cx, header));
                    debug_assert!(res.is_ok());
                }
                Poll::Ready(Ok(n))
            }

            State::ExpectingHeader
            | State::ReceivingHeader(_, _)
            | State::ReceivedHeader
            | State::ReceivedData
            | State::AligningData(_)
            | State::AlignedData
            | State::ReceivingEof(_)
            | State::ReceivedEof => {
                panic!("cannot write entry; invalid state: {:?}", self.state)
            }
        }
    }

    fn poll_finish_entry(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        header: &Header,
    ) -> Poll<Result<()>> {
        loop {
            if TRACING_ENABLED {
                eprintln!("     | fini: {:?}", self.state);
            }

            match self.state {
                State::ReceivedData => {
                    self.as_mut().project().state.take_marker(Some(header))?;
                    continue;
                }

                State::AligningData(rem) => {
                    let buf = &Block::empty().as_bytes()[..rem];
                    ready!(self.as_mut().poll_write_data(cx, buf, Some(header)))?;
                    continue;
                }

                State::AlignedData => {
                    self.project().state.take_marker(None)?;
                    return Poll::Ready(Ok(()));
                }

                State::ExpectingHeader => {
                    return Poll::Ready(Ok(()));
                }

                State::ReceivingHeader(_, _)
                | State::ReceivedHeader
                | State::ReceivingData(_)
                | State::ReceivingEof(_)
                | State::ReceivedEof => {
                    panic!("cannot finish entry; invalid state: {:?}", self.state)
                }
            }
        }
    }

    pub(super) fn poll_finish(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        loop {
            match self.state {
                State::ExpectingHeader => {
                    let buf = Block::empty().as_bytes();
                    ready!(self.as_mut().poll_write_data(cx, buf, None))?;
                    continue;
                }

                State::ReceivingHeader(rem, true) | State::ReceivingEof(rem) => {
                    let buf = &Block::empty().as_bytes()[..rem];
                    ready!(self.as_mut().poll_write_data(cx, buf, None))?;
                    continue;
                }

                State::ReceivedEof => {
                    ready!(self.as_mut().poll_flush_buffered(cx))?;
                    return self.project().io.poll_shutdown(cx);
                }

                State::ReceivingHeader(_, false)
                | State::ReceivedHeader
                | State::ReceivingData(_)
                | State::ReceivedData
                | State::AligningData(_)
                | State::AlignedData => {
                    panic!("cannot finish archive; invalid state: {:?}", self.state)
                }
            }
        }
    }

    /// Send data in our main buffer into the inner writer, looping as
    /// necessary until either it's all been sent or an error occurs.
    fn poll_flush_buffered(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let mut this = self.project();
        let mut buf = this.buf.buffered();

        while !buf.is_empty() {
            let bytes = buf.bytes();
            let bytes_written = ready!(this.io.as_mut().poll_write(cx, bytes))?;
            if bytes_written == 0 {
                // Because all the data in the buffer has been reported to
                // our owner as "successfully written" (by returning nonzero
                // success values from `write`), any 0-length writes from
                // `inner` must be reported as i/o errors from this method.
                return WriteError::WriteZero.into();
            }
            buf.commit(bytes_written);
        }

        this.buf.clear();

        Poll::Ready(Ok(()))
    }

    fn poll_write_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
        header: Option<&Header>,
    ) -> Poll<Result<usize>> {
        let max = buf.len();
        let slice = [IoSlice::new(buf)];
        self.poll_write_vectored(cx, &slice, max, header)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
        max: usize,
        header: Option<&Header>,
    ) -> Poll<Result<usize>> {
        let prefix = bufs.take_prefix(max);
        let prefix_len = prefix.bytes_len();

        // Check that bufs contain valid data before we go ahead and write them.
        let next = {
            let this = self.as_mut().project();
            this.state.take_slices(prefix.iter_buffers(), header)?
        };
        assert_eq!(next.1, prefix_len);

        // See if we can pass the slices through to the underlying writer
        // unbuffered. For this to happen, bufs total must exceed our buffer
        // capacity and the writer must support vectored writes.
        let can_pass_through = {
            let this = self.as_mut().project();
            prefix_len >= this.buf.capacity() && this.io.is_write_vectored()
        };

        if can_pass_through || prefix_len > self.as_mut().project().buf.available().remaining() {
            // Flush our buffer so we don't write data out of order if we're
            // passing slices through, or make some space in our buffer so the
            // slices can fit.
            ready!(self.as_mut().poll_flush_buffered(cx))?;
        }

        let mut this = self.as_mut().project();

        let bytes_written = if can_pass_through {
            let slices_len = prefix_len - prefix.remainder().len();
            // Pass the slices through.
            match ready!(this.io.as_mut().poll_write_vectored(cx, prefix.slices()))? {
                // If we wrote the complete slice array then write the remainder.
                n if n == slices_len => match this.io.as_mut().poll_write(cx, prefix.remainder()) {
                    // Uphold AsyncWrite semantics for vectored writes.
                    // We already wrote some data, so we must not return an error
                    // or pending, so in either case return what we wrote this far.
                    Poll::Ready(Ok(w)) => n + w,
                    Poll::Ready(Err(_)) => n,
                    Poll::Pending => n,
                },
                n => n,
            }
        } else {
            // Fill our buffer with as much data from the slices as we can fit.
            this.buf.available().fill_from_slices(prefix.iter_buffers())
        };

        // Update our state based on the actual slice of data that we've written.
        *this.state = if bytes_written == prefix_len {
            next.0
        } else {
            let prefix = prefix.take_prefix(bytes_written);
            // This cannot fail because we've already checked every slice within bufs.
            let next = this
                .state
                .take_slices(prefix.iter_buffers(), header)
                .expect("this slice should have already been checked");
            assert_eq!(next.1, bytes_written);
            next.0
        };

        Poll::Ready(Ok(bytes_written))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        ready!(self.as_mut().poll_flush_buffered(cx))?;
        self.project().io.poll_flush(cx)
    }

    fn is_write_vectored(&self) -> bool {
        true
    }
}

impl<W: AsyncWrite> AsyncWrite for Entry<'_, W> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let slice = [IoSlice::new(buf)];
        self.poll_write_vectored(cx, &slice)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize>> {
        if TRACING_ENABLED {
            eprintln!("write: '{}', size = {}", self.path_lossy(), self.size());
        }
        let this = self.project();
        let header = this.header;
        this.archive.as_mut().poll_write_entry(cx, bufs, header)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.project().archive.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if TRACING_ENABLED {
            eprintln!("finsh: '{}', size = {}", self.path_lossy(), self.size());
        }
        let this = self.project();
        let header = this.header;
        ready!(this.archive.as_mut().poll_finish_entry(cx, header))?;
        this.archive.as_mut().poll_flush(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.archive.is_write_vectored()
    }
}

#[cfg(test)]
mod tests;
