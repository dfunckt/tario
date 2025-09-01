use std::io::{Error as IoError, ErrorKind, Result};
use std::task::Poll;

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("expecting more data for entry; expected = {expected}, received = {received}")]
    UnexpectedEof { expected: u64, received: u64 },

    #[error("failed to write the buffered data")]
    WriteZero,

    #[error("cannot write new entry while another is being written")]
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
