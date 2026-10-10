//! Small bootstrap configuration, allowing the main data directory to move.
use super::*;

fn config_path() -> PathBuf {
    directories::ProjectDirs::from("org", "nntpbrowser", "NNTPBrowser")
        .map(|dirs| dirs.config_dir().join("storage.json"))
        .unwrap_or_else(|| PathBuf::from("nntpbrowser-storage.json"))
}

pub(super) fn configured() -> Option<PathBuf> {
    let bytes = std::fs::read(config_path()).ok()?;
    let path: String = serde_json::from_slice(&bytes).ok()?;
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

pub(super) fn store(value: &str) -> Result<(), std::io::Error> {
    let path = config_path();
    let value = value.trim();
    if value.is_empty() {
        return match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    if !Path::new(value).is_absolute() {
        return Err(std::io::Error::other("Data directory must be absolute."));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    use std::io::Write;
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)
}
