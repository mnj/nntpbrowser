//! Remove generated group caches without touching explicit downloads.
use super::*;
use std::fs;

impl Worker {
    pub(super) fn unsubscribe_group(&mut self, server_id: i64, name: &str) {
        if let Err(error) = self.remove_group_data(server_id, name) {
            self.notice(format!("unsubscribe: {error}"));
        }
        self.emit_groups(server_id);
    }

    fn remove_group_data(&mut self, server_id: i64, name: &str) -> Result<(), String> {
        let groups = self
            .catalog
            .list_groups(server_id)
            .map_err(|e| e.to_string())?;
        let Some(group) = groups.iter().find(|group| group.name == name) else {
            return Ok(());
        };
        let slug = store::group_slug(name);
        if groups
            .iter()
            .any(|other| other.id != group.id && store::group_slug(&other.name) == slug)
        {
            return Err("cache directory is shared with another group; refusing removal".into());
        }
        let post_ids = self
            .catalog
            .post_ids_for_group(group.id)
            .map_err(|e| e.to_string())?;
        let ids: HashSet<_> = post_ids.iter().copied().collect();
        self.headers
            .check_group_removal(server_id, name)
            .map_err(|e| e.to_string())?;
        let files = self.group_cache_files(&ids)?;
        let jobs = self.catalog.list_jobs().map_err(|e| e.to_string())?;
        let mut explicit_paths = Vec::new();
        for job in jobs
            .iter()
            .filter(|job| self.media_job_limit(job).is_none())
        {
            explicit_paths.push((
                manage_downloads::resolved_path(Path::new(&job.dest_dir))?,
                manage_downloads::resolved_path(Path::new(&job.dest_path))?,
            ));
        }
        // A known explicit saved file takes precedence even if its chosen
        // destination resembles one of the historical generated cache paths.
        let mut generated_files = Vec::new();
        for path in files {
            let resolved = manage_downloads::resolved_path(&path)?;
            if !explicit_paths.iter().any(|(_, file)| *file == resolved) {
                generated_files.push(path);
            }
        }
        let legacy_jobs: Vec<_> = jobs
            .iter()
            .filter(|job| {
                job.post_id.is_some_and(|id| ids.contains(&id))
                    && self.media_job_limit(job).is_some()
            })
            .collect();
        let legacy_root = self.data_dir.join("media-cache");
        let mut legacy_dirs = Vec::new();
        if checked_directory(&legacy_root)? {
            for id in &ids {
                let path = legacy_root.join(id.to_string());
                if checked_directory(&path)? {
                    // Only generated media-cache jobs establish ownership.
                    if !legacy_jobs.iter().any(|job| job.post_id == Some(*id)) {
                        return Err(format!("media cache ownership is unknown for article {id}"));
                    }
                    let resolved = manage_downloads::resolved_path(&path)?;
                    if explicit_paths.iter().any(|(dir, file)| {
                        dir.starts_with(&resolved)
                            || resolved.starts_with(dir)
                            || file.starts_with(&resolved)
                    }) {
                        return Err("media cache directory is shared with an explicit download; refusing removal".into());
                    }
                    legacy_dirs.push(path);
                }
            }
        }
        // Cancel only the obsolete automatic disk-cache jobs. Explicit file
        // downloads retain their transfer state and independent segment rows.
        for job in legacy_jobs {
            if !matches!(job.state.as_str(), "done" | "cancelled" | "failed") {
                self.cancel(job.id);
            }
        }
        for path in generated_files {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        for path in legacy_dirs {
            fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
        self.headers
            .remove_group(server_id, name)
            .map_err(|e| e.to_string())?;
        self.catalog
            .unsubscribe(server_id, name)
            .map_err(|e| e.to_string())?;
        self.purge_group_previews(&ids);
        let _ = self.evt.send(Event::GroupUnsubscribed {
            group_id: group.id,
            post_ids,
        });
        self.emit_jobs();
        self.notice(format!("unsubscribed {name}; cached group data removed"));
        Ok(())
    }

    fn group_cache_files(&self, ids: &HashSet<i64>) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        for (dir, kind) in [
            (self.data_dir.join("article-bodies"), CacheKind::Body),
            (self.data_dir.join("thumbs"), CacheKind::Thumb),
            (self.downloads.join("preview"), CacheKind::Preview),
        ] {
            // The preview directory's parent may be a separately configured
            // downloads location. Check it before following any child.
            if !checked_directory(dir.parent().expect("cache parent"))? || !checked_directory(&dir)?
            {
                continue;
            }
            for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if !owned_cache_name(&name, ids, kind) {
                    continue;
                }
                let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(format!(
                        "unsafe generated cache file: {}",
                        entry.path().display()
                    ));
                }
                files.push(entry.path());
            }
        }
        Ok(files)
    }
}

#[derive(Clone, Copy)]
enum CacheKind {
    Body,
    Thumb,
    Preview,
}

fn checked_directory(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(format!("unsafe cache directory: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn owned_cache_name(name: &str, ids: &HashSet<i64>, kind: CacheKind) -> bool {
    match kind {
        CacheKind::Thumb => name
            .strip_suffix(".jpg")
            .is_some_and(|id| owned_id(id, ids)),
        CacheKind::Preview => [".image", ".image.part"].iter().any(|suffix| {
            name.strip_suffix(suffix)
                .is_some_and(|id| owned_id(id, ids))
        }),
        CacheKind::Body => {
            let Some((id, rest)) = name.split_once('-') else {
                return false;
            };
            let Some((hash, suffix)) = rest.split_once('.') else {
                return false;
            };
            owned_id(id, ids)
                && hash.len() == 16
                && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                && matches!(
                    suffix,
                    "body" | "body.zst" | "body.zst.pending" | "message-id" | "message-id.pending"
                )
        }
    }
}

fn owned_id(text: &str, ids: &HashSet<i64>) -> bool {
    text.parse::<i64>()
        .ok()
        .is_some_and(|id| ids.contains(&id) && id.to_string() == text)
}
