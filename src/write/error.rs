use std::fmt;
use std::io::{Error as IoError, ErrorKind, Result};
use std::task::Poll;

#[derive(Debug)]
pub enum WriteError {
    UnexpectedEof { expected: u64, received: u64 },
    WriteZero,
    OverlappingEntry,
}

impl WriteError {
    #[inline]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::UnexpectedEof { .. } => ErrorKind::UnexpectedEof,
            Self::WriteZero => ErrorKind::WriteZero,
            Self::OverlappingEntry => ErrorKind::Unsupported,
        }
    }
}

impl std::error::Error for WriteError {}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { expected, received } => format!(
                "expecting more data for entry; expected = {expected}, received = {received}"
            )
            .fmt(f),
            Self::WriteZero => "failed to write the buffered data".fmt(f),
            Self::OverlappingEntry => {
                "cannot write new entry while another is being written".fmt(f)
            }
        }
    }
}

impl From<WriteError> for IoError {
    #[inline]
    fn from(value: WriteError) -> Self {
        IoError::new(value.kind(), value)
    }
}

impl<T> From<WriteError> for Result<T> {
    #[inline]
    fn from(value: WriteError) -> Self {
        Err(value.into())
    }
}

impl<T> From<WriteError> for Poll<Result<T>> {
    #[inline]
    fn from(value: WriteError) -> Self {
        Poll::Ready(Err(value.into()))
    }
}
