use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("header store: {0}")]
    Header(String),
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("articles must be strictly ascending")]
    Order,
}
