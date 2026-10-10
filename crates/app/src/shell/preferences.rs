//! Persisted application preferences, independent of individual news servers.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub(super) gallery_tile_width: f32,
    pub(super) ui_scale: f32,
    pub(super) default_gallery: bool,
    pub(super) sidebar_open: bool,
    pub(super) gallery_auto: bool,
    pub(super) preview_limit_mib: u64,
    pub(super) preview_cache_mib: usize,
    pub(super) nntp_timeout_secs: u64,
    pub(super) downloads_dir: String,
    pub(super) data_dir: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            gallery_tile_width: 220.0,
            ui_scale: 1.0,
            default_gallery: false,
            sidebar_open: true,
            gallery_auto: true,
            preview_limit_mib: 10,
            preview_cache_mib: 64,
            nntp_timeout_secs: 8,
            downloads_dir: String::new(),
            data_dir: String::new(),
        }
    }
}

impl Preferences {
    fn normalize(&mut self) {
        self.gallery_tile_width = if self.gallery_tile_width.is_finite() {
            self.gallery_tile_width.clamp(140.0, 360.0)
        } else {
            220.0
        };
        self.ui_scale = if self.ui_scale.is_finite() {
            self.ui_scale.clamp(0.75, 2.0)
        } else {
            1.0
        };
        self.preview_limit_mib = self.preview_limit_mib.clamp(1, 10);
        self.preview_cache_mib = self
            .preview_cache_mib
            .clamp(8, 64)
            .max(self.preview_limit_mib as usize);
        self.nntp_timeout_secs = self.nntp_timeout_secs.clamp(2, 120);
    }

    pub(super) fn load(dir: &Path) -> (Self, Option<String>) {
        let path = dir.join("preferences.json");
        let result = std::fs::read(path).and_then(|bytes| {
            serde_json::from_slice::<Self>(&bytes).map_err(std::io::Error::other)
        });
        let (mut preferences, notice) = match result {
            Ok(mut prefs) => {
                prefs.normalize();
                (prefs, None)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(error) => (
                Self::default(),
                Some(format!("preferences failed: {error}")),
            ),
        };
        if preferences.data_dir.trim().is_empty()
            && storage_location::configured().as_deref() == Some(dir)
        {
            preferences.data_dir = dir.to_string_lossy().into_owned();
        }
        (preferences, notice)
    }

    fn store(&self, dir: &Path) -> Result<(), std::io::Error> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let path = dir.join("preferences.json");
        let temporary = dir.join("preferences.json.tmp");
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
        std::fs::rename(temporary, path)?;
        Ok(())
    }
}

impl Browser {
    // Unlike Apply, changing tile size must preserve the current RAM previews,
    // network configuration and generation. Persist only the applied snapshot.
    pub(super) fn set_gallery_tile_width(&mut self, width: f32, persist: bool) {
        let width = if width.is_finite() {
            width.clamp(140.0, 360.0)
        } else {
            220.0
        };
        self.preferences.gallery_tile_width = width;
        self.preferences_applied.gallery_tile_width = width;
        if persist {
            let result = (|| {
                self.preferences_applied.store(self.handle.data_dir())?;
                let next_data = self.preferences_applied.data_dir.trim();
                if !next_data.is_empty() && Path::new(next_data) != self.handle.data_dir() {
                    self.preferences_applied.store(Path::new(next_data))?;
                }
                Ok::<(), std::io::Error>(())
            })();
            if let Err(error) = result {
                self.notice = format!("tile size preference failed: {error}");
                self.record_error(&self.notice.clone(), "Gallery");
            }
        }
    }

    pub(super) fn save_preferences(&mut self, ctx: &egui::Context) {
        self.preferences.normalize();
        let result = (|| {
            for value in [&self.preferences.downloads_dir, &self.preferences.data_dir] {
                if !value.trim().is_empty() {
                    let path = Path::new(value.trim());
                    if !path.is_absolute() {
                        return Err(std::io::Error::other(
                            "Choose an absolute storage directory.",
                        ));
                    }
                    std::fs::create_dir_all(path)?;
                }
            }
            self.preferences.store(self.handle.data_dir())?;
            #[cfg(feature = "ui-preview")]
            if self.preview_fixture {
                return Ok(());
            }
            let next_data = if self.preferences.data_dir.trim().is_empty() {
                platform_data_dir()
            } else {
                PathBuf::from(self.preferences.data_dir.trim())
            };
            if next_data != self.handle.data_dir() {
                std::fs::create_dir_all(&next_data)?;
                self.preferences.store(&next_data)?;
            }
            storage_location::store(&self.preferences.data_dir)
        })();
        match result {
            Ok(()) => {
                self.preferences_applied = self.preferences.clone();
                ctx.set_zoom_factor(self.preferences.ui_scale);
                self.sidebar_open = self.preferences.sidebar_open;
                self.gallery_auto = self.preferences.gallery_auto;
                self.media_generation = self.media_generation.wrapping_add(1);
                self.media_pending.clear();
                self.media_progress.clear();
                self.media_errors.clear();
                self.media_evicted.clear();
                self.handle
                    .send(Command::PreviewContext(self.media_generation));
                self.handle.send(Command::ConfigureNetwork {
                    timeout_secs: self.preferences.nntp_timeout_secs,
                });
                self.handle.send(Command::ConfigureStorage {
                    downloads_dir: self.configured_downloads_dir(),
                });
                self.trim_memory_previews();
                self.notice = "Preferences saved.".into();
            }
            Err(error) => {
                self.notice = format!("preferences failed: {error}");
                self.record_error(&self.notice.clone(), "Settings");
            }
        }
    }

    pub(super) fn configured_downloads_dir(&self) -> PathBuf {
        if self.preferences_applied.downloads_dir.trim().is_empty() {
            self.handle.data_dir().join("downloads")
        } else {
            PathBuf::from(self.preferences_applied.downloads_dir.trim())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_round_trip_and_bound_resource_controls() {
        let dir = std::env::temp_dir().join(format!("nntpbrowser-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut preferences = Preferences {
            gallery_tile_width: 900.0,
            ui_scale: f32::INFINITY,
            preview_limit_mib: 80,
            preview_cache_mib: 800,
            nntp_timeout_secs: 0,
            ..Default::default()
        };
        preferences.normalize();
        preferences.store(&dir).unwrap();
        let (loaded, error) = Preferences::load(&dir);
        assert!(error.is_none());
        assert_eq!(loaded.gallery_tile_width, 360.0);
        assert_eq!(loaded.ui_scale, 1.0);
        assert_eq!(loaded.preview_limit_mib, 10);
        assert_eq!(loaded.preview_cache_mib, 64);
        assert_eq!(loaded.nntp_timeout_secs, 2);
        preferences.preview_cache_mib = 8;
        preferences.normalize();
        assert_eq!(preferences.preview_cache_mib, 10);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_preview_concurrency_preference_does_not_override_server_limits() {
        let dir = std::env::temp_dir().join(format!(
            "nntpbrowser-prefs-legacy-concurrency-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("preferences.json"),
            br#"{"preview_concurrency":4,"preview_limit_mib":7,"nntp_timeout_secs":20}"#,
        )
        .unwrap();
        let (preferences, error) = Preferences::load(&dir);
        assert!(error.is_none());
        assert_eq!(preferences.gallery_tile_width, 220.0);
        assert_eq!(preferences.preview_limit_mib, 7);
        assert_eq!(preferences.nntp_timeout_secs, 20);
        preferences.store(&dir).unwrap();
        let saved = std::fs::read_to_string(dir.join("preferences.json")).unwrap();
        assert!(!saved.contains("preview_concurrency"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
