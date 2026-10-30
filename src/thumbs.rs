//! Small JPEG previews for the photo grid, cached outside the storage folders.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::Result;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader};

/// Short side of a thumbnail; tiles are square and cropped by the browser.
const THUMB_SIDE: u32 = 360;

/// Cache file for a photo. Changes when the photo's size or mtime changes.
pub fn cache_path(thumbs_dir: &Path, section: &str, rel: &str, meta: &std::fs::Metadata) -> PathBuf {
    let mut h = DefaultHasher::new();
    rel.hash(&mut h);
    meta.len().hash(&mut h);
    crate::storage::mtime_ms(meta).hash(&mut h);
    thumbs_dir.join(section).join(format!("{:016x}.jpg", h.finish()))
}

/// Largest side of a preview a browser may send for a format we can't decode (HEIC).
pub const MAX_PREVIEW_SIDE: u32 = 4096;

/// Stores a JPEG made by the browser from a HEIC/AVIF photo (Safari decodes HEIC, this
/// program can't), and a thumbnail from it. The bytes must really be a sane JPEG.
pub fn store_preview(jpeg: &[u8], preview_dst: &Path, thumb_dst: &Path) -> Result<()> {
    let reader = ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let (w, h) = reader.into_dimensions()?;
    if w == 0 || h == 0 || w > MAX_PREVIEW_SIDE || h > MAX_PREVIEW_SIDE {
        anyhow::bail!("размер превью {w}×{h}");
    }
    image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)?;
    if let Some(dir) = preview_dst.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = preview_dst.with_extension("tmp");
    std::fs::write(&tmp, jpeg)?;
    std::fs::rename(&tmp, preview_dst)?;
    generate(preview_dst, thumb_dst)
}

pub fn generate(src: &Path, dst: &Path) -> Result<()> {
    let mut decoder = ImageReader::open(src)?.with_guessed_format()?.into_decoder()?;
    // Phones store photos sideways plus an "orientation" flag. Browsers apply it to the
    // full image themselves, but our re-encoded thumbnail would lose it, so apply it here.
    let orientation = decoder.orientation()?;
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);

    let (w, h) = img.dimensions();
    let thumb = if w <= h { img.thumbnail(THUMB_SIDE, u32::MAX) } else { img.thumbnail(u32::MAX, THUMB_SIDE) };

    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = dst.with_extension("tmp");
    {
        let mut out = BufWriter::new(File::create(&tmp)?);
        JpegEncoder::new_with_quality(&mut out, 80).encode_image(&thumb.to_rgb8())?;
    }
    std::fs::rename(&tmp, dst)?;
    Ok(())
}
