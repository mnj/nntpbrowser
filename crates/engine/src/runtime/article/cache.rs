//! Fast, checksummed article-body compression with bounded cache reads.

use super::{Worker, ARTICLE_CAP};
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::PathBuf;

const COMPRESSED_CAP: usize = ARTICLE_CAP + 64 * 1024;
const WINDOW_LOG_MAX: u32 = 22; // At most a 4 MiB decoder window.

impl Worker {
    pub(in crate::runtime) fn store_article_body(
        &self,
        post_id: i64,
        message_id: &str,
        body: &[u8],
    ) -> Result<(), String> {
        if body.len() > ARTICLE_CAP || message_id.len() > 4096 {
            return Err("article cache item exceeds its size limit".into());
        }
        let legacy = self.article_cache_path(post_id, message_id);
        fs::create_dir_all(legacy.parent().expect("article cache directory"))
            .map_err(|error| error.to_string())?;
        let mut encoder =
            zstd::stream::write::Encoder::new(Vec::new(), 1).map_err(|error| error.to_string())?;
        encoder
            .include_checksum(true)
            .map_err(|error| error.to_string())?;
        encoder
            .window_log(WINDOW_LOG_MAX)
            .map_err(|error| error.to_string())?;
        encoder.write_all(body).map_err(|error| error.to_string())?;
        let compressed = encoder.finish().map_err(|error| error.to_string())?;
        let path = legacy.with_extension("body.zst");
        let pending = path.with_extension("zst.pending");
        fs::write(&pending, compressed).map_err(|error| error.to_string())?;
        fs::rename(&pending, &path).map_err(|error| error.to_string())?;
        let marker = legacy.with_extension("message-id");
        let pending = marker.with_extension("message-id.pending");
        fs::write(&pending, message_id).map_err(|error| error.to_string())?;
        fs::rename(pending, marker).map_err(|error| error.to_string())?;
        // Successful refresh replaces any prior uncompressed version.
        let _ = fs::remove_file(legacy);
        Ok(())
    }

    /// Legacy filename and marker identity remain stable across cache versions.
    pub(in crate::runtime) fn article_cache_path(&self, post_id: i64, message_id: &str) -> PathBuf {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        message_id.hash(&mut hash);
        self.data_dir
            .join("article-bodies")
            .join(format!("{post_id}-{:016x}.body", hash.finish()))
    }

    pub(in crate::runtime) fn cached_article_body(
        &self,
        post_id: i64,
        message_id: &str,
        max_bytes: usize,
    ) -> Option<Vec<u8>> {
        let legacy = self.article_cache_path(post_id, message_id);
        // Check the full message ID, rather than trusting a reused catalog ID.
        let mut marker = String::new();
        File::open(legacy.with_extension("message-id"))
            .ok()?
            .take(4097)
            .read_to_string(&mut marker)
            .ok()?;
        if marker.len() > 4096 || marker != message_id {
            return None;
        }
        let limit = max_bytes.min(ARTICLE_CAP);
        let compressed = legacy.with_extension("body.zst");
        match File::open(compressed) {
            Ok(file) => {
                if file.metadata().ok()?.len() > COMPRESSED_CAP as u64 {
                    return None;
                }
                let mut decoder =
                    zstd::stream::read::Decoder::new(file.take(COMPRESSED_CAP as u64)).ok()?;
                decoder.window_log_max(WINDOW_LOG_MAX).ok()?;
                let mut body = Vec::new();
                // Read one extra byte so an oversized output cannot masquerade
                // as a complete file; EOF also validates frame/checksum endings.
                decoder.take(limit as u64 + 1).read_to_end(&mut body).ok()?;
                (body.len() <= limit).then_some(body)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut body = Vec::new();
                File::open(legacy)
                    .ok()?
                    .take(limit as u64 + 1)
                    .read_to_end(&mut body)
                    .ok()?;
                if body.len() > limit {
                    return None;
                }
                // Migration is an optional cache optimization: a write failure
                // must not turn a valid legacy cache hit into a network request.
                let _ = self.store_article_body(post_id, message_id, &body);
                Some(body)
            }
            Err(_) => None,
        }
    }
}
