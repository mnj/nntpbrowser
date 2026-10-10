//! Resize a finished image to a JPEG thumbnail. Never call this on the UI thread.

use std::{io::Cursor, path::Path};

use image::imageops::FilterType;
use image::GenericImageView;
use thiserror::Error;

mod video;
pub use video::write_video_jpeg;

const RAM_IMAGE_BUDGET: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ThumbError {
    #[error("image: {0}")]
    Image(String),
    #[error("video preview: {0}")]
    Video(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Longest edge becomes `max_edge` pixels. Returns the written pixel size.
pub fn write_jpeg(src: &Path, dest: &Path, max_edge: u32) -> Result<(u32, u32), ThumbError> {
    let (width, height, pixels) = load_rgba_scaled(src, max_edge)?;
    let resized = image::RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| ThumbError::Image("invalid image dimensions".into()))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image::DynamicImage::ImageRgba8(resized)
        .to_rgb8()
        .save(dest)
        .map_err(|err| ThumbError::Image(err.to_string()))?;
    Ok((width, height))
}

/// Decode an image file to RGBA bytes for an egui texture.
pub fn load_rgba(path: &Path) -> Result<(u32, u32, Vec<u8>), ThumbError> {
    let img = image::open(path).map_err(|err| ThumbError::Image(err.to_string()))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((w, h, rgba.into_raw()))
}

/// Decode a bounded library image and resize it before allocating its RGBA
/// texture. The longest edge never exceeds `max_edge`; small images stay small.
pub fn load_rgba_scaled(path: &Path, max_edge: u32) -> Result<(u32, u32, Vec<u8>), ThumbError> {
    if max_edge == 0 {
        return Err(ThumbError::Image(
            "image edge must be greater than zero".into(),
        ));
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|err| ThumbError::Image(err.to_string()))?;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(ThumbError::Image("empty image".into()));
    }
    let scaled = if w.max(h) > max_edge {
        img.resize(max_edge, max_edge, FilterType::Triangle)
    } else {
        img
    };
    let rgba = scaled.into_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((w, h, rgba.into_raw()))
}

/// Decode and scale RAM-cached image bytes without reading or writing files.
/// Source dimensions and decode/RGBA allocations are bounded to 64 MiB.
pub fn load_rgba_bytes_scaled(
    bytes: &[u8],
    max_edge: u32,
) -> Result<(u32, u32, Vec<u8>), ThumbError> {
    if max_edge == 0 {
        return Err(ThumbError::Image(
            "image edge must be greater than zero".into(),
        ));
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(RAM_IMAGE_BUDGET);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|error| ThumbError::Image(error.to_string()))?;
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) * 4 > RAM_IMAGE_BUDGET {
        return Err(ThumbError::Image(
            "image dimensions exceed RAM preview budget".into(),
        ));
    }
    let scaled = if width.max(height) > max_edge {
        img.resize(max_edge, max_edge, FilterType::Triangle)
    } else {
        img
    };
    let rgba = scaled.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((width, height, rgba.into_raw()))
}

/// Decode an image or an optional video poster entirely from RAM-cached bytes.
/// Call on a background thread. A failed video poster can keep its file-kind icon.
pub fn load_media_rgba_bytes(
    bytes: &[u8],
    filename: &str,
    max_edge: u32,
) -> Result<(u32, u32, Vec<u8>), ThumbError> {
    if store::classify(filename) == store::FileKind::Video {
        video::load_video_rgba_bytes(bytes, max_edge.min(256))
    } else {
        load_rgba_bytes_scaled(bytes, max_edge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_image_decode_preserves_pixels_scales_and_checks_source_limits() {
        let image = image::RgbaImage::from_pixel(1000, 400, image::Rgba([10, 20, 30, 128]));
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let (width, height, pixels) =
            load_media_rgba_bytes(encoded.get_ref(), "picture.png", 100).unwrap();
        assert_eq!((width, height), (100, 40));
        assert_eq!(pixels.len(), 100 * 40 * 4);
        assert_eq!(&pixels[..4], &[10, 20, 30, 128]);
        assert_eq!(
            load_rgba_bytes_scaled(encoded.get_ref(), 2000).unwrap().0,
            1000
        );
        assert!(load_rgba_bytes_scaled(encoded.get_ref(), 0).is_err());
        assert!(load_rgba_bytes_scaled(b"not an image", 256).is_err());

        let mut excessive = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(16_385, 1))
            .write_to(&mut excessive, image::ImageFormat::Png)
            .unwrap();
        assert!(load_rgba_bytes_scaled(excessive.get_ref(), 256).is_err());
    }

    #[test]
    fn scaled_rgba_preserves_aspect_ratio_and_does_not_upscale() {
        let path =
            std::env::temp_dir().join(format!("nntpbrowser-scale-{}.png", std::process::id()));
        let img = image::RgbaImage::from_pixel(1000, 400, image::Rgba([10, 20, 30, 128]));
        img.save(&path).unwrap();
        let (w, h, pixels) = load_rgba_scaled(&path, 100).unwrap();
        assert_eq!((w, h), (100, 40));
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        assert_eq!(&pixels[..4], &[10, 20, 30, 128]);
        let (w, h, pixels) = load_rgba_scaled(&path, 2000).unwrap();
        assert_eq!((w, h), (1000, 400));
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        assert!(load_rgba_scaled(&path, 0).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scaled_rgba_rejects_excessive_source_dimensions() {
        let path = std::env::temp_dir().join(format!(
            "nntpbrowser-scale-limit-{}.png",
            std::process::id()
        ));
        image::RgbaImage::new(16_385, 1).save(&path).unwrap();
        assert!(load_rgba_scaled(&path, 512).is_err());
        assert!(write_jpeg(&path, &path.with_extension("jpg"), 256).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
