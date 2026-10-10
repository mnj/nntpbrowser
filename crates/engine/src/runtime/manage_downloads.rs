//! Explicit download-history and dedicated output-directory removal.

use super::{queue::sanitize, Event, Worker};
use std::fs;
use std::path::{Component, Path, PathBuf};
use store::JobRow;

impl Worker {
    pub(super) fn delete_download(&mut self, job_id: i64, remove_files: bool) {
        if let Err(error) = self.try_delete_download(job_id, remove_files) {
            let _ = self.evt.send(Event::DownloadDeleteFailed { job_id, error });
        }
    }

    fn try_delete_download(&mut self, job_id: i64, remove_files: bool) -> Result<(), String> {
        let job = self
            .catalog
            .job_by_id(job_id)
            .map_err(|error| error.to_string())?
            .ok_or("download was not found")?;
        if !matches!(job.state.as_str(), "done" | "failed" | "cancelled") {
            return Err("cancel this download before removing it".into());
        }
        if remove_files {
            let target = self.deletion_directory(&job)?;
            let thumbnail = self.thumbnail_to_remove(&job)?;
            match fs::remove_dir_all(&target) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("could not remove download files: {error}")),
            }
            if let Some((post_id, path)) = thumbnail {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(format!("could not remove generated thumbnail: {error}"))
                    }
                }
                self.catalog
                    .remove_thumb_for_post(post_id)
                    .map_err(|error| error.to_string())?;
            }
        }
        self.catalog
            .delete_job(job_id)
            .map_err(|error| error.to_string())?;
        // Deleted highest row IDs can be reused. Retain a generation tombstone
        // and old in-flight mappings until the stale network replies drain.
        let generation = self.transfer_generation.entry(job_id).or_default();
        *generation = generation.wrapping_add(1);
        self.paused.remove(&job_id);
        self.cancelled.remove(&job_id);
        let _ = self.evt.send(Event::DownloadDeleted {
            job_id,
            remove_files,
        });
        self.emit_jobs();
        Ok(())
    }

    fn thumbnail_to_remove(&self, job: &JobRow) -> Result<Option<(i64, PathBuf)>, String> {
        let Some(post_id) = job.post_id else {
            return Ok(None);
        };
        if self
            .catalog
            .list_jobs()
            .map_err(|error| error.to_string())?
            .iter()
            .any(|other| {
                other.id != job.id && other.post_id == Some(post_id) && other.state == "done"
            })
        {
            return Ok(None);
        }
        let dir = self.data_dir.join("thumbs");
        match fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("generated thumbnail directory is not a regular directory".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("could not inspect generated thumbnails: {error}")),
        }
        Ok(Some((post_id, dir.join(format!("{post_id}.jpg")))))
    }

    fn deletion_directory(&self, job: &JobRow) -> Result<PathBuf, String> {
        let path = Path::new(&job.dest_dir);
        if !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err("download folder must be an absolute path without parent traversal".into());
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("download folder has no valid final name")?;
        if !dedicated_name(job, name) && self.media_job_limit(job).is_none() {
            return Err("this legacy download folder is not recognized as dedicated; remove the history entry and manage its files manually".into());
        }
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("download folder is a symbolic link; its files were retained".into())
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("download folder is not a directory".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("could not inspect download folder: {error}")),
        }
        let target = resolved_path(path)?;
        for reserved in [
            &self.data_dir,
            &self.data_dir.join("headers"),
            &self.data_dir.join("downloads"),
            &self.downloads,
        ] {
            let reserved = resolved_path(reserved)?;
            if reserved.starts_with(&target) {
                return Err("refusing to remove a reserved directory or its ancestor".into());
            }
        }
        if target.starts_with(resolved_path(&self.data_dir.join("headers"))?) {
            return Err("refusing to remove header-cache files".into());
        }
        for other in self
            .catalog
            .list_jobs()
            .map_err(|error| error.to_string())?
        {
            if other.id == job.id {
                continue;
            }
            let other = resolved_path(Path::new(&other.dest_dir))?;
            if other.starts_with(&target) || target.starts_with(other) {
                return Err("another download uses this folder or an overlapping folder; its files were retained".into());
            }
        }
        Ok(target)
    }
}

fn dedicated_name(job: &JobRow, folder: &str) -> bool {
    let decimal = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if let Some(post_id) = job.post_id {
        if folder == format!("{}-{post_id}", sanitize(&job.name)) {
            return true;
        }
        return folder
            .strip_prefix(&format!("saved-media-{post_id}-"))
            .is_some_and(decimal);
    }
    let clean: String = sanitize(&job.name).chars().take(120).collect();
    folder
        .strip_prefix(&format!("{clean}-"))
        .is_some_and(decimal)
}

/// Resolve existing ancestor aliases even when the output folder is missing.
pub(super) fn resolved_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                }
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                for part in suffix.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    ancestor
                        .file_name()
                        .ok_or("download folder has no existing ancestor")?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or("download folder has no existing ancestor")?;
            }
            Err(error) => return Err(format!("could not resolve download folder: {error}")),
        }
    }
}
