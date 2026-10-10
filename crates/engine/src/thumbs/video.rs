//! Optional local video posters. Missing ffmpeg leaves the caller's video icon.

use super::{load_rgba_bytes_scaled, write_jpeg, ThumbError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn ffmpeg_command() -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new("ffmpeg");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

const POSTER_TIMEOUT: Duration = Duration::from_secs(5);
const POSTER_BYTE_CAP: u64 = 1024 * 1024;

/// Generate a bounded poster using only stdin/stdout pipes: no temporary files.
/// Seek-dependent containers can fail; callers should retain their video icon.
pub(super) fn load_video_rgba_bytes(
    bytes: &[u8],
    max_edge: u32,
) -> Result<(u32, u32, Vec<u8>), ThumbError> {
    if max_edge == 0 || max_edge > 256 || bytes.len() > 64 * 1024 * 1024 {
        return Err(ThumbError::Video("video exceeds RAM poster limits".into()));
    }
    let mut child = ffmpeg_command()
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-max_alloc",
            "67108864",
            "-threads",
            "1",
            "-protocol_whitelist",
            "pipe",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-frames:v",
            "1",
            "-vf",
        ])
        .arg(format!(
            "scale={max_edge}:{max_edge}:force_original_aspect_ratio=decrease"
        ))
        .args([
            "-threads",
            "1",
            "-c:v",
            "mjpeg",
            "-q:v",
            "3",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("piped ffmpeg stdin");
    let stdout = child.stdout.take().expect("piped ffmpeg stdout");
    let encoded = std::thread::scope(|scope| {
        // Write and drain concurrently so neither pipe can deadlock the process.
        let sender = scope.spawn(move || stdin.write_all(bytes));
        let receiver = scope.spawn(move || {
            let mut encoded = Vec::new();
            stdout.take(POSTER_BYTE_CAP + 1).read_to_end(&mut encoded)?;
            if encoded.len() as u64 > POSTER_BYTE_CAP {
                return Err(ThumbError::Video("video poster exceeds byte limit".into()));
            }
            Ok(encoded)
        });
        let finished = wait_bounded(&mut child, POSTER_TIMEOUT);
        // ffmpeg may exit successfully after one frame before consuming all
        // input; a writer BrokenPipe then does not invalidate the valid poster.
        let sent = sender.join();
        let received = receiver.join();
        finished?;
        let _ = sent.map_err(|_| ThumbError::Video("video input worker failed".into()))?;
        received.map_err(|_| ThumbError::Video("video poster worker failed".into()))?
    })?;
    load_rgba_bytes_scaled(&encoded, max_edge)
}

/// Extract one JPEG poster, bounded to 256 pixels, from an already cached video.
/// Never call this on the UI thread. ffmpeg is optional and uses no network protocols.
pub fn write_video_jpeg(src: &Path, dest: &Path, max_edge: u32) -> Result<(u32, u32), ThumbError> {
    if max_edge == 0 || max_edge > 256 {
        return Err(ThumbError::Video(
            "poster edge must be between 1 and 256".into(),
        ));
    }
    let source = std::fs::canonicalize(src)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = TemporaryPoster(dest.with_extension(format!(
        "{}.{}.poster.jpg",
        std::process::id(),
        stamp
    )));
    let mut command = ffmpeg_command();
    command
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-max_alloc",
            "67108864",
            "-threads",
            "1",
            "-protocol_whitelist",
            "file",
            "-i",
        ])
        .arg(source)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-frames:v",
            "1",
            "-vf",
        ])
        .arg(format!(
            "scale={max_edge}:{max_edge}:force_original_aspect_ratio=decrease"
        ))
        .args([
            "-threads", "1", "-c:v", "mjpeg", "-q:v", "3", "-f", "image2",
        ])
        .arg(&temporary.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    wait_bounded(&mut child, POSTER_TIMEOUT)?;
    write_jpeg(&temporary.0, dest, max_edge)
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> Result<(), ThumbError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(ThumbError::Video("ffmpeg could not decode video".into())),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => {
                // Always kill and reap on timeout or a wait error.
                let _ = child.kill();
                let _ = child.wait();
                return match result {
                    Err(error) => Err(ThumbError::Io(error)),
                    _ => Err(ThumbError::Video("ffmpeg timed out".into())),
                };
            }
        }
    }
}

struct TemporaryPoster(PathBuf);

impl Drop for TemporaryPoster {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_ffmpeg_creates_a_bounded_video_poster_and_rejects_bad_input() {
        let Ok(mut version) = ffmpeg_command()
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        wait_bounded(&mut version, POSTER_TIMEOUT).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "nntpbrowser-video-poster-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("clip.mp4");
        let poster = dir.join("poster.jpg");
        let mut fixture = ffmpeg_command()
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=640x240:d=0.1",
                "-frames:v",
                "1",
                "-threads",
                "1",
                "-c:v",
                "mpeg4",
                "-movflags",
                "+faststart",
            ])
            .arg(&source)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_bounded(&mut fixture, POSTER_TIMEOUT).unwrap();
        let bytes = std::fs::read(&source).unwrap();
        let (width, height, pixels) =
            super::super::load_media_rgba_bytes(&bytes, "clip.MP4", 512).unwrap();
        assert_eq!((width, height), (256, 96));
        assert_eq!(pixels.len(), 256 * 96 * 4);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        assert!(load_video_rgba_bytes(b"not a video", 256).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let (width, height) = write_video_jpeg(&source, &poster, 256).unwrap();
        assert_eq!((width, height), (256, 96));
        assert_eq!(image::image_dimensions(&poster).unwrap(), (256, 96));
        assert!(write_video_jpeg(&source, &poster, 0).is_err());
        let broken = dir.join("broken.mp4");
        std::fs::write(&broken, b"not a video").unwrap();
        assert!(write_video_jpeg(&broken, &poster, 256).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);

        let mut endless = ffmpeg_command()
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-re",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=16x16",
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert!(matches!(
            wait_bounded(&mut endless, Duration::from_millis(100)),
            Err(ThumbError::Video(message)) if message == "ffmpeg timed out"
        ));
        assert!(endless.try_wait().unwrap().is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
