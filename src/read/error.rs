use std::fmt;
use std::io::{Error as IoError, ErrorKind, Result};
use std::task::Poll;

#[derive(Debug)]
pub enum ReadError {
    UnexpectedEof { expected: usize, received: usize },
}

impl ReadError {
    #[inline]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::UnexpectedEof { .. } => ErrorKind::UnexpectedEof,
        }
    }
}

impl std::error::Error for ReadError {}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { expected, received } => format!(
                "expecting more data for entry; expected = {expected}, received = {received}"
            )
            .fmt(f),
        }
    }
}

impl From<ReadError> for IoError {
    #[inline]
    fn from(value: ReadError) -> Self {
        IoError::new(value.kind(), value)
    }
}

impl<T> From<ReadError> for Result<T> {
    #[inline]
    fn from(value: ReadError) -> Self {
        Err(value.into())
    }
}

impl<T> From<ReadError> for Poll<Result<T>> {
    #[inline]
    fn from(value: ReadError) -> Self {
        Poll::Ready(Err(value.into()))
    }
}
