//! OSF1: append-only zstd blocks of overview records, indexed by article number.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::error::Error;

mod cleanup;
mod format;
mod manifest;
mod recovery;
use format::*;
use manifest::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub article: u64,
    pub date_unix: i64,
    pub date_raw: Option<String>,
    pub bytes: u64,
    pub lines: u32,
    pub message_id: String,
    pub subject: String,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadSpan {
    pub first: u64,
    pub last: u64,
}

pub struct HeaderStore {
    root: PathBuf,
}

impl HeaderStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        set_dir_mode(&root)?;
        Ok(Self { root })
    }

    /// Remove a deleted server's overview cache so SQLite ID reuse cannot
    /// associate another provider with its old article numbers and watermark.
    /// Finished downloads are stored separately and are not affected.
    pub fn remove_server(&self, server_id: i64) -> Result<(), Error> {
        match fs::remove_dir_all(self.root.join(server_id.to_string())) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn append_range(
        &self,
        server_id: i64,
        group: &str,
        records: &[Record],
    ) -> Result<(), Error> {
        self.append_range_sized(server_id, group, records, DEFAULT_BLOCK)
    }

    pub(crate) fn append_range_sized(
        &self,
        server_id: i64,
        group: &str,
        records: &[Record],
        block_target: usize,
    ) -> Result<(), Error> {
        if records.is_empty() {
            return Ok(());
        }
        for pair in records.windows(2) {
            if pair[1].article <= pair[0].article {
                return Err(Error::Order);
            }
        }
        let dir = self.group_dir(server_id, group)?;
        let mut manifest = read_manifest(&dir)?;
        let low = records[0].article;
        let high = records[records.len() - 1].article;
        if manifest
            .files
            .iter()
            .any(|file| low <= file.high && file.low <= high)
        {
            return Err(Error::Header(format!(
                "range {low}-{high} overlaps a stored span"
            )));
        }
        let name = unique_name(&dir, low, high);
        let path = dir.join(&name);
        write_osf(&path, low, high, records, block_target.max(1))?;
        manifest.files.push(ManifestFile { name, low, high });
        manifest.watermark = manifest.watermark.max(high);
        write_manifest(&dir, &manifest)?;
        Ok(())
    }

    pub fn watermark(&self, server_id: i64, group: &str) -> Result<u64, Error> {
        let dir = self.group_dir(server_id, group)?;
        Ok(read_manifest(&dir)?.watermark)
    }

    /// Inclusive spans already stored, sorted by article number. Empty spaces
    /// inside a span represent missing server articles, not unsynced headers.
    pub fn stored_ranges(&self, server_id: i64, group: &str) -> Result<Vec<(u64, u64)>, Error> {
        let dir = self.group_dir(server_id, group)?;
        let mut ranges: Vec<_> = read_manifest(&dir)?
            .files
            .into_iter()
            .map(|file| (file.low, file.high))
            .collect();
        ranges.sort_unstable();
        Ok(ranges)
    }

    pub fn for_each<F>(
        &self,
        server_id: i64,
        group: &str,
        mut on_record: F,
    ) -> Result<Vec<BadSpan>, Error>
    where
        F: FnMut(Record),
    {
        self.for_range(server_id, group, 0, u64::MAX, &mut on_record)
    }

    pub fn for_range<F>(
        &self,
        server_id: i64,
        group: &str,
        low: u64,
        high: u64,
        on_record: &mut F,
    ) -> Result<Vec<BadSpan>, Error>
    where
        F: FnMut(Record),
    {
        let dir = self.group_dir(server_id, group)?;
        let manifest = read_manifest(&dir)?;
        let mut bad = Vec::new();
        for file in &manifest.files {
            if file.high < low || file.low > high {
                continue;
            }
            let spans = read_osf_range(&dir.join(&file.name), low, high, on_record)?;
            bad.extend(spans);
        }
        Ok(bad)
    }

    pub fn sweep_orphans(&self, server_id: i64, group: &str) -> Result<usize, Error> {
        let dir = self.group_dir(server_id, group)?;
        let manifest = read_manifest(&dir)?;
        let mut removed = 0;
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.ends_with(".osf") {
                continue;
            }
            if manifest.files.iter().any(|file| file.name == name) {
                continue;
            }
            fs::remove_file(entry.path())?;
            removed += 1;
        }
        Ok(removed)
    }

    pub fn compact_if_needed(&self, server_id: i64, group: &str) -> Result<bool, Error> {
        let dir = self.group_dir(server_id, group)?;
        let manifest = read_manifest(&dir)?;
        if manifest.files.len() <= 16 {
            return Ok(false);
        }
        // Only join adjacent spans. Joining across an unsynced hole would make
        // stored_ranges claim historical headers that were never downloaded.
        let mut runs: Vec<Vec<ManifestFile>> = Vec::new();
        for source in &manifest.files {
            if let Some(run) = runs.last_mut() {
                if source.low <= run.last().unwrap().high.saturating_add(1) {
                    run.push(source.clone());
                    continue;
                }
            }
            runs.push(vec![source.clone()]);
        }
        if runs.iter().all(|run| run.len() == 1) {
            return Ok(false);
        }
        let mut replacement = Manifest {
            watermark: manifest.watermark,
            files: Vec::new(),
        };
        let mut created = Vec::new();
        let merge = (|| {
            for run in runs {
                if run.len() == 1 {
                    replacement.files.push(run[0].clone());
                    continue;
                }
                let low = run[0].low;
                let high = run.last().unwrap().high;
                let name = unique_name(&dir, low, high);
                let path = dir.join(&name);
                created.push(path.clone());
                // Keep one decoded input block and one output block in memory.
                let mut writer = OsfWriter::new(&path, low, high, DEFAULT_BLOCK)?;
                for source in &run {
                    let mut write_error = None;
                    let bad =
                        read_osf_range(&dir.join(&source.name), 0, u64::MAX, &mut |record| {
                            if write_error.is_none() {
                                write_error = writer.push(&record).err();
                            }
                        })?;
                    if let Some(error) = write_error {
                        return Err(error);
                    }
                    if !bad.is_empty() {
                        return Err(Error::Header("cannot compact damaged header blocks".into()));
                    }
                }
                writer.finish()?;
                replacement.files.push(ManifestFile { name, low, high });
            }
            write_manifest(&dir, &replacement)
        })();
        if let Err(error) = merge {
            for path in created {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        for source in manifest.files {
            if !replacement
                .files
                .iter()
                .any(|file| file.name == source.name)
            {
                let _ = fs::remove_file(dir.join(source.name));
            }
        }
        Ok(true)
    }

    fn group_dir(&self, server_id: i64, group: &str) -> Result<PathBuf, Error> {
        let dir = self
            .root
            .join(server_id.to_string())
            .join(group_slug(group));
        fs::create_dir_all(&dir)?;
        set_dir_mode(&dir)?;
        set_dir_mode(&self.root.join(server_id.to_string()))?;
        Ok(dir)
    }
}

pub fn group_slug(name: &str) -> String {
    let safe = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'));
    if safe {
        name.to_string()
    } else {
        format!("g_{:016x}", xxh3_64(name.as_bytes()))
    }
}

fn unique_name(dir: &Path, low: u64, high: u64) -> String {
    let base = format!("r_{low}_{high}.osf");
    if !dir.join(&base).exists() {
        return base;
    }
    let mut n = 2u32;
    loop {
        let name = format!("r_{low}_{high}_{n}.osf");
        if !dir.join(&name).exists() {
            return name;
        }
        n += 1;
    }
}

fn create_file(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    set_file_mode(path)?;
    Ok(file)
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
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests;
