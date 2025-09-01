//! A library to asynchronously read and write TAR archives.
//!
//! # Writing
//!
//! To write an archive, use [Archive::add_entry] to write the header and
//! get back an [Entry] handle for writing the entry's data.
//!
//! ```
//! # use std::io::Result;
//! # #[tokio::main(flavor = "current_thread")] async fn main() -> Result<()> {
//! use tokio::io::{AsyncWrite, AsyncWriteExt};
//! use tario::{Archive, Header};
//!
//! let mut io: Vec<u8> = Vec::new();
//! let mut archive = Archive::new(&mut io);
//! let mut files = [("hello.txt", "hello world!")];
//!
//! for (path, contents) in files {
//!   let mut header = Header::new_ustar();
//!   header.set_path(path)?;
//!   header.set_size(contents.len() as u64);
//!   header.set_cksum();
//!
//!   let mut entry = archive.add_entry(header).await?;
//!   entry.write(contents.as_bytes()).await?;
//! }
//!
//! archive.finish().await?;
//! # Ok(()) }
//! ```
//!
//! # Reading
//!
//! When reading an archive, use [Archive::next_entry] to get an [Entry]
//! handle for the next file entry in the archive and read the entry's data.
//!
//! ```no_run
//! # use std::io::Result;
//! # #[tokio::main(flavor = "current_thread")] async fn main() -> Result<()> {
//! use std::io;
//! use tokio::io::{AsyncRead, AsyncReadExt};
//! use tario::Archive;
//!
//! let io = io::Cursor::new(&[]); // Get a reader from somewhere
//! let mut archive = Archive::new(io);
//! let buf = &mut [0u8; 100];
//!
//! while let Some(mut entry) = archive.next_entry().await? {
//!   loop {
//!     let bytes_written = entry.read(buf).await?;
//!     if bytes_written == 0 {
//!       // Reached entry EOF
//!       break;
//!     }
//!     // do_something_with_buffer(&buf[..bytes_written]);
//!   }
//! }
//! # Ok(()) }
//! ```
//!
//! A TAR byte stream is a series of entries in order, so working with
//! multiple entries concurrently is not possible -- their data cannot be
//! interleaved. The compiler will helpfully prevent you from doing that,
//! so the following does not compile:
//!
//! ```compile_fail
//! use std::io;
//! use tario::Archive;
//!
//! let io = io::Cursor::new(&[]);
//! let mut archive = Archive::new(io);
//! let entry1 = archive.next_entry();
//! let entry2 = archive.next_entry();
//! entry1;
//! // error[E0499]: cannot borrow `archive` as mutable more than once at a time
//! ```

use std::borrow::Cow;
use std::future::poll_fn;
use std::io::Result;
use std::num::NonZeroUsize;
use std::pin::Pin;

use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite};

mod shared;
pub use shared::block::{BLOCK_SIZE, Header};

mod read;
pub use read::ReadError;

mod write;
pub use write::WriteError;

#[cfg(feature = "streams")]
use read::Entries;
use read::NextEntry;
use shared::buffer::Buf;
use shared::state::State;

const DEFAULT_BUFFER_CAPACITY: usize = 8; // x512 = 4k

pin_project! {
    /// A type that wraps an async I/O object and provides methods to read or
    /// write TAR archives.
    #[derive(Debug)]
    pub struct Archive<T> {
        buf: Buf,
        state: State,

        #[pin]
        io: T,
    }
}

impl<T> Archive<T> {
    /// Creates a new Archive with default buffer capacity.
    ///
    /// The default buffer capacity is currently 8 blocks, for a total of 4096
    /// bytes.
    pub fn new(io: T) -> Self {
        Self::with_capacity(io, NonZeroUsize::new(DEFAULT_BUFFER_CAPACITY).unwrap())
    }

    /// Creates a new Archive with the given buffer capacity.
    ///
    /// `capacity` is the number of blocks (512 bytes each) to buffer.
    ///
    /// This will panic if the capacity in bytes exceeds [usize::MAX].
    pub fn with_capacity(io: T, capacity: NonZeroUsize) -> Self {
        let cap = capacity
            .get()
            .checked_mul(BLOCK_SIZE)
            .expect("capacity overflow");

        Self {
            buf: Buf::new(cap),
            state: State::default(),
            io,
        }
    }

    /// Consumes this archive and returns the underlying I/O object.
    pub fn into_inner(self) -> T {
        self.io
    }
}

impl<R: AsyncRead + Unpin> Archive<R> {
    /// Returns a future that resolves to the next [entry][Entry] or [None]
    /// if EOF is reached.
    #[inline]
    pub fn next_entry(&mut self) -> NextEntry<'_, R> {
        NextEntry::new(self)
    }

    /// Returns a stream yielding [entries][Entry] until EOF is reached.
    ///
    /// This is only available when the `streams` feature is enabled.
    ///
    /// A TAR byte stream is a series of entries in order, so working with
    /// multiple entries concurrently is not possible -- their data cannot be
    /// interleaved. The compiler will helpfully prevent you from doing that,
    /// so the following does not compile:
    ///
    /// ```compile_fail
    /// use std::io;
    /// use futures_util::StreamExt;
    /// use tario::Archive;
    ///
    /// let io = io::Cursor::new(&[]);
    /// let mut archive = Archive::new(io);
    /// let mut entries = archive.entries();
    /// let entry1 = entries.next();
    /// let entry2 = entries.next();
    /// entry1;
    /// // error[E0499]: cannot borrow `archive` as mutable more than once at a time
    /// ```
    #[cfg(feature = "streams")]
    #[inline]
    pub fn entries(&mut self) -> Entries<'_, R> {
        Entries::new(self)
    }
}

impl<W: AsyncWrite + Unpin> Archive<W> {
    #[inline]
    pub async fn add_entry(&mut self, header: Header) -> Result<Entry<'_, W>> {
        let mut pin = Pin::new(self);
        poll_fn(|cx| pin.as_mut().poll_write_header(cx, &header)).await?;
        Entry::new(pin, header)
    }

    /// Writes the last two consecutive empty blocks that signify EOF.
    ///
    /// This will panic if an entry is currently being written.
    #[inline]
    pub async fn finish(&mut self) -> Result<()> {
        let mut pin = Pin::new(self);
        poll_fn(|cx| pin.as_mut().poll_finish(cx)).await
    }
}

pin_project! {
    /// A handle to a file entry in a TAR archive, that provides methods to
    /// read or write its data.
    #[derive(Debug)]
    pub struct Entry<'a, T> {
        archive: Pin<&'a mut Archive<T>>,
        header: Header,
    }
}

impl<'a, T> Entry<'a, T> {
    fn new(archive: Pin<&'a mut Archive<T>>, header: Header) -> Result<Self> {
        let cksum = header.cksum()?;
        assert!(cksum > 0, "header must be finalized before creating entry");

        let _ = header.size()?;

        Ok(Self { archive, header })
    }

    /// Returns the header of this entry.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Returns the file size of this entry.
    pub fn size(&self) -> u64 {
        // This cannot fail because we'd have already errored in [Self::new].
        self.header.size().unwrap()
    }

    /// Returns the number of bytes this entry occupies in the archive.
    pub fn len(&self) -> u64 {
        // This cannot fail because we'd have already errored in [Self::new].
        self.header.entry_size().unwrap()
    }

    /// Returns whether this entry has no data.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the pathname of this entry, with any `\` characters converted
    /// to directory separators.
    pub fn path(&self) -> Cow<[u8]> {
        self.header.path_bytes()
    }

    /// Gets the path in a "lossy" way; only useful for reference.
    pub fn path_lossy(&self) -> String {
        String::from_utf8_lossy(&self.header.path_bytes()).to_string()
    }
}

impl<R: AsyncRead + Unpin> Entry<'_, R> {
    /// Reads until the end of this entry. All entries must be fully consumed
    /// so this is necessary to call if you don't care about this entry's data
    /// and just need to skip to the next one.
    #[inline]
    pub async fn skip(&mut self) -> Result<()> {
        let mut pin = Pin::new(self);
        poll_fn(|cx| pin.as_mut().poll_skip(cx)).await
    }
}

impl<W: AsyncWrite + Unpin> Entry<'_, W> {
    #[inline]
    pub async fn finish(&mut self) -> Result<()> {
        let mut pin = Pin::new(self);
        poll_fn(|cx| pin.as_mut().poll_shutdown(cx)).await
    }
}

/// Re-export of [tar-rs][1] providing types for synchronous I/O.
///
/// [1]: https://github.com/alexcrichton/tar-rs
#[doc(no_inline)]
pub use tar as sync;

// enables cheap print debugging
#[cfg(debug_assertions)]
const TRACING_ENABLED: bool = true;
#[cfg(not(debug_assertions))]
const TRACING_ENABLED: bool = false;

#[cfg(test)]
#[test]
fn assert_autotraits() {
    fn is_unpin<T: Unpin>() {}
    is_unpin::<Archive<()>>();
    is_unpin::<Entry<()>>();

    fn is_send<T: Send>() {}
    is_send::<Archive<()>>();
    is_send::<Entry<()>>();
    is_send::<ReadError>();
    is_send::<WriteError>();

    fn is_sync<T: Sync>() {}
    is_sync::<Archive<()>>();
    is_sync::<Entry<()>>();
    is_sync::<ReadError>();
    is_sync::<WriteError>();
}
