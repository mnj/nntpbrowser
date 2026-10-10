//! Exclusive ownership of the catalog and header cache.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

pub(super) fn lock_data_dir(path: &Path) -> Result<File, String> {
    std::fs::create_dir_all(path).map_err(|err| format!("create data directory: {err}"))?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(path.join(".engine.lock"))
        .map_err(|err| format!("open data directory lock: {err}"))?;
    lock.try_lock().map_err(|err| match err {
        TryLockError::WouldBlock => {
            "this data directory is already in use; close its other app or CLI process".into()
        }
        TryLockError::Error(err) => format!("lock data directory: {err}"),
    })?;
    Ok(lock)
}
