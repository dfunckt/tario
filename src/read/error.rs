use std::io::{Error as IoError, ErrorKind, Result};
use std::task::Poll;

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("expecting more data for entry; expected = {expected}, received = {received}")]
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
