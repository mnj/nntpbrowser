//! Cached media previews and finished download thumbnails.

use super::{Event, Worker};
use crate::{decode, thumbs};
use nntp::ArticleRef;
use std::path::{Path, PathBuf};
use store::FileKind;

impl Worker {
    pub(super) fn write_thumbs(&self, dir: &Path, post_id: Option<i64>) {
        let Some(post_id) = post_id else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let kind = store::classify(&path.to_string_lossy());
            if !matches!(kind, FileKind::Image | FileKind::Video) {
                continue;
            }
            let dest = self.data_dir.join("thumbs").join(format!("{post_id}.jpg"));
            let thumbnail = match kind {
                FileKind::Video => thumbs::write_video_jpeg(&path, &dest, 256),
                _ => thumbs::write_jpeg(&path, &dest, 256),
            };
            if let Ok((width, height)) = thumbnail {
                let _ = self
                    .catalog
                    .insert_thumb(post_id, &dest, width as i64, height as i64);
                let _ = self.evt.send(Event::Thumb {
                    post_id,
                    path: dest,
                });
                break;
            }
        }
    }

    pub(super) async fn preview(&mut self, post_id: i64) {
        if self
            .catalog
            .list_jobs()
            .ok()
            .into_iter()
            .flatten()
            .any(|job| job.state == "fetching")
        {
            self.notice("preview: deferred while downloads are active".into());
            return;
        }
        if let Ok(Some(thumb)) = self.catalog.thumb_for_post(post_id) {
            if Path::new(&thumb.path).is_file() {
                let _ = self.evt.send(Event::Thumb {
                    post_id,
                    path: PathBuf::from(thumb.path),
                });
                return;
            }
        }
        let Some((server, group, post)) = self.find_post(post_id) else {
            self.notice("preview: article was not found".into());
            return;
        };
        let files = self.catalog.files_for_post(post_id).unwrap_or_default();
        let Some(image) = files
            .iter()
            .find(|file| file.kind == FileKind::Image && file.expected_segments <= 1)
        else {
            self.notice("preview: no single-part image in this article".into());
            return;
        };
        let segments = match self.segments_for_key(server.id, &group.name, &post.group_key) {
            Ok(segments) => segments,
            Err(err) => {
                self.notice(format!("preview: {err}"));
                return;
            }
        };
        let mut images = segments
            .into_iter()
            .filter(|segment| segment.filename == image.name);
        let Some(segment) = images.next() else {
            self.notice("preview: image article was not found".into());
            return;
        };
        if images.next().is_some() {
            self.notice("preview: image needs a full download".into());
            return;
        }
        let bytes = match self
            .body_from_farm(&ArticleRef::MessageId(segment.message_id))
            .await
        {
            Ok((_, bytes)) => bytes,
            Err(err) => {
                self.notice(format!("preview: {err}"));
                return;
            }
        };
        let part = match decode::decode_body(&bytes) {
            Ok(part) => part,
            Err(err) => {
                self.notice(format!("preview: {err}"));
                return;
            }
        };
        let dest = self
            .downloads
            .join("preview")
            .join(format!("{post_id}.image"));
        // A preview gets a unique path independent of untrusted subject filenames.
        if let Err(err) = decode::write_part(&dest, &part) {
            self.notice(format!("preview: {err}"));
            return;
        }
        let thumb = self.data_dir.join("thumbs").join(format!("{post_id}.jpg"));
        match thumbs::write_jpeg(&dest, &thumb, 256) {
            Ok((width, height)) => {
                let _ = self
                    .catalog
                    .insert_thumb(post_id, &thumb, width as i64, height as i64);
                let _ = self.evt.send(Event::Thumb {
                    post_id,
                    path: thumb,
                });
            }
            Err(err) => self.notice(format!("preview: {err}")),
        }
    }
}
