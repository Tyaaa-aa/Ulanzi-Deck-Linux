use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Cursor;
use std::path::Path;

use resvg::tiny_skia::{self, Pixmap, PixmapPaint, Transform};
use resvg::usvg;

use super::icons::IconLibrary;
use crate::Color;
use crate::model::{ImageSource, KeyLook};

/// Key images are square, this many pixels per side.
pub const KEY_PX: u32 = 196;
/// Largest source image dimension accepted before decoding.
const MAX_SOURCE_PX: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;

pub struct RenderCtx<'a> {
    /// Relative image paths resolve against this directory.
    pub profile_dir: &'a Path,
    pub icons: &'a IconLibrary,
}

#[derive(Debug, Clone)]
pub struct Rendered {
    pub png: Vec<u8>,
    pub warnings: Vec<String>,
}

/// Draw a key at [`KEY_PX`]². `None` means there is nothing to draw (no image and a black background),
/// so the device should show the key empty.
pub fn render_key(look: &KeyLook, ctx: &RenderCtx) -> Option<Rendered> {
    if look.source == ImageSource::None && look.background == Color::BLACK {
        return None;
    }
    let mut pix = Pixmap::new(KEY_PX, KEY_PX)?;
    let [r, g, b, a] = look.background.0;
    pix.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
    let mut warnings = Vec::new();
    match &look.source {
        ImageSource::None => {}
        ImageSource::Icon { set, name } => match ctx.icons.svg(*set, name) {
            Some(svg) => {
                if let Err(e) = draw_svg(&mut pix, svg, look.foreground, look.icon_scale) {
                    warnings.push(format!("icon \"{name}\" could not be drawn: {e}"));
                }
            }
            None => warnings.push(format!("icon \"{name}\" is not in the bundled icon set")),
        },
        ImageSource::Image { path } => {
            let full = ctx.profile_dir.join(path);
            if let Err(e) = draw_image(&mut pix, &full) {
                warnings.push(format!("image {}: {e}", full.display()));
            }
        }
        ImageSource::Text { .. }
        | ImageSource::App { .. }
        | ImageSource::Steam { .. }
        | ImageSource::Emoji { .. } => {
            warnings.push("this image source arrives with the GUI (M2)".into());
        }
    }
    let png = pix.encode_png().ok()?;
    Some(Rendered { png, warnings })
}

fn draw_svg(pix: &mut Pixmap, svg: &str, fg: Color, scale: f32) -> Result<(), String> {
    let svg = svg.replace(
        "currentColor",
        &format!("#{:02x}{:02x}{:02x}", fg.0[0], fg.0[1], fg.0[2]),
    );
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let size = tree.size();
    let box_px = KEY_PX as f32 * scale.clamp(0.05, 1.0);
    let k = box_px / size.width().max(size.height());
    let offset = (KEY_PX as f32 - box_px) / 2.0;
    let opacity = fg.0[3] as f32 / 255.0;
    if opacity >= 1.0 {
        resvg::render(
            &tree,
            Transform::from_scale(k, k).post_translate(offset, offset),
            &mut pix.as_mut(),
        );
    } else {
        let mut layer = Pixmap::new(KEY_PX, KEY_PX).ok_or("no layer")?;
        resvg::render(
            &tree,
            Transform::from_scale(k, k).post_translate(offset, offset),
            &mut layer.as_mut(),
        );
        let paint = PixmapPaint {
            opacity,
            ..PixmapPaint::default()
        };
        pix.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::identity(), None);
    }
    Ok(())
}

fn draw_image(pix: &mut Pixmap, path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_PX);
    limits.max_image_height = Some(MAX_SOURCE_PX);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    let fitted = img
        .resize_to_fill(KEY_PX, KEY_PX, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let mut layer = Pixmap::new(KEY_PX, KEY_PX).ok_or("no layer")?;
    for (dst, src) in layer.pixels_mut().iter_mut().zip(fitted.pixels()) {
        let [r, g, b, a] = src.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    pix.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    Ok(())
}

/// Cache key for a look. Image sources also hash the file's size and mtime, so replacing the file
/// re-renders the key.
pub fn look_hash(look: &KeyLook, profile_dir: &Path) -> u64 {
    let mut h = DefaultHasher::new();
    format!("{look:?}").hash(&mut h);
    if let ImageSource::Image { path } = &look.source
        && let Ok(meta) = std::fs::metadata(profile_dir.join(path))
    {
        meta.len().hash(&mut h);
        meta.modified().ok().hash(&mut h);
    }
    h.finish()
}
