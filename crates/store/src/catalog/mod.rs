//! SQLite catalog for servers, grouped posts, the queue, and thumbnails.
//! Overview rows do not live here.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Error;
use crate::grouper::FileKind;

mod accounts;
mod bulk_queue;
mod file_alias;
mod posts;
mod queue;
mod repair;
mod repair_types;
mod schema;
mod thumbs;
mod types;
mod unsubscribe;

pub use repair_types::*;
use schema::prepare;
pub use types::*;

pub struct Catalog {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Catalog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent)?;
                set_dir_mode(parent)?;
            }
        }
        let conn = Connection::open(&path)?;
        set_file_mode(&path)?;
        prepare(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("catalog lock")
    }
}
fn set_dir_mode(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

fn set_file_mode(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests;
