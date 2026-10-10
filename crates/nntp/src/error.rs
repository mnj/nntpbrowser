use std::io;

use thiserror::Error;

/// Failures a caller can show in the UI. Display text never includes a password.
#[derive(Debug, Error)]
pub enum Error {
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("tls: {0}")]
    Tls(String),
    #[error("authentication failed")]
    Auth,
    #[error("nntp {code} {text}")]
    Protocol { code: u16, text: String },
    #[error("article missing")]
    Missing,
    #[error("article body exceeds {0} bytes")]
    BodyTooLarge(usize),
    #[error("group does not exist")]
    NoSuchGroup,
    #[error("no newsgroup selected")]
    NoGroupSelected,
    #[error("current article number is invalid")]
    NoCurrentArticle,
    #[error("no previous article in this group")]
    NoPreviousArticle,
    #[error("no next article in this group")]
    NoNextArticle,
    #[error("permission denied")]
    PermissionDenied,
    #[error("overview is not supported")]
    OverviewUnsupported,
    #[error("compression failed: {0}")]
    Compression(String),
    #[error("command unsupported")]
    Unsupported,
    #[error("timed out")]
    Timeout,
    #[error("{0}: timed out")]
    TimeoutAt(&'static str),
    #[error("connection closed")]
    Closed,
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

impl Error {
    pub(crate) fn from_status(code: u16, text: String) -> Self {
        match code {
            411 => Error::NoSuchGroup,
            412 => Error::NoGroupSelected,
            420 => Error::NoCurrentArticle,
            421 => Error::NoPreviousArticle,
            422 => Error::NoNextArticle,
            502 => Error::PermissionDenied,
            423 | 430 => Error::Missing,
            480..=482 => Error::Auth,
            500 | 501 => Error::Unsupported,
            _ => Error::Protocol { code, text },
        }
    }

    pub(crate) fn is_retryable_conn(&self) -> bool {
        matches!(
            self,
            Error::Io(_)
                | Error::Closed
                | Error::Timeout
                | Error::TimeoutAt(_)
                | Error::Protocol {
                    code: 400 | 401 | 503,
                    ..
                }
        )
    }

    pub(crate) fn timeout_context(self, operation: &'static str) -> Self {
        match self {
            Error::Timeout => Error::TimeoutAt(operation),
            other => other,
        }
    }
}
