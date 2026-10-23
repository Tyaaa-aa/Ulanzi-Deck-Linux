//! Bringing the user's own pictures into a profile: decoded with limits, sized for a key and stored
//! under the profile's `icons/` directory by content hash.

use std::io::Cursor;
use std::path::Path;

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

use crate::render::KEY_PX;

/// Largest side accepted when importing (big photos are downscaled, not refused).
const MAX_IMPORT_PX: u32 = 16_384;
const MAX_IMPORT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("That file isn't an image")]
    NotAnImage,
    #[error("That image is too large")]
    TooLarge,
    #[error("Couldn't read the file: {0}")]
    Read(#[from] std::io::Error),
    #[error("Couldn't save the image: {0}")]
    Save(String),
}

/// Import `src` into `profile_dir`, returning the path to store in the profile (relative to it).
pub fn import_image(src: &Path, profile_dir: &Path) -> Result<String, ImportError> {
    import_sized(src, profile_dir, (KEY_PX, KEY_PX))
}

/// Size of pictures for the wide screen (twice its 458×196, so they stay sharp in the app too).
pub const SCREEN_PICTURE: (u32, u32) = (916, 392);

/// Import a picture for the wide screen (widget background or the Pictures widget).
pub fn import_screen_image(src: &Path, profile_dir: &Path) -> Result<String, ImportError> {
    import_sized(src, profile_dir, SCREEN_PICTURE)
}

fn import_sized(src: &Path, profile_dir: &Path, (w, h): (u32, u32)) -> Result<String, ImportError> {
    if std::fs::metadata(src)?.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let bytes = std::fs::read(src)?;
    let is_svg = src
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));
    let img = if is_svg {
        render_svg(&bytes, (w, h))?
    } else {
        decode_raster(&bytes, (w, h))?
    };
    let mut png = Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| ImportError::Save(e.to_string()))?;
    let png = png.into_inner();
    let rel = format!("icons/{:016x}.png", fnv1a(&png));
    let dest = profile_dir.join(&rel);
    if !dest.exists() {
        std::fs::create_dir_all(profile_dir.join("icons"))?;
        let tmp = dest.with_extension("png.tmp");
        std::fs::write(&tmp, &png)?;
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(rel)
}

fn decode_raster(bytes: &[u8], (w, h): (u32, u32)) -> Result<image::RgbaImage, ImportError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ImportError::NotAnImage)?;
    if reader.format().is_none() {
        return Err(ImportError::NotAnImage);
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMPORT_PX);
    limits.max_image_height = Some(MAX_IMPORT_PX);
    limits.max_alloc = Some(MAX_IMPORT_BYTES);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => ImportError::TooLarge,
        _ => ImportError::NotAnImage,
    })?;
    Ok(img
        .resize_to_fill(w, h, image::imageops::FilterType::Lanczos3)
        .to_rgba8())
}

/// Rasterise an SVG (no external files) to fit inside `w`×`h`.
fn render_svg(bytes: &[u8], (w, h): (u32, u32)) -> Result<image::RgbaImage, ImportError> {
    let opts = usvg::Options {
        resources_dir: None,
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &opts).map_err(|_| ImportError::NotAnImage)?;
    let size = tree.size();
    let k = (w as f32 / size.width()).min(h as f32 / size.height());
    let (ox, oy) = (
        (w as f32 - size.width() * k) / 2.0,
        (h as f32 - size.height() * k) / 2.0,
    );
    let mut pix = Pixmap::new(w, h).ok_or(ImportError::TooLarge)?;
    resvg::render(
        &tree,
        Transform::from_scale(k, k).post_translate(ox, oy),
        &mut pix.as_mut(),
    );
    let mut out = image::RgbaImage::new(w, h);
    for (dst, src) in out.pixels_mut().zip(pix.pixels()) {
        let c = src.demultiply();
        dst.0 = [c.red(), c.green(), c.blue(), c.alpha()];
    }
    Ok(out)
}

/// FNV-1a: a stable content hash for file names (not for security).
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}
