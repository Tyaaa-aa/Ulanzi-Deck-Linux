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
    let own_label = look
        .label
        .position
        .filter(|_| look.label.visible && !look.label.text.trim().is_empty());
    if look.source == ImageSource::None && look.background == Color::BLACK && own_label.is_none() {
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
            if let Err(e) = draw_file(&mut pix, &full, Fit::Cover) {
                warnings.push(format!("image {}: {e}", full.display()));
            }
        }
        ImageSource::Text { content } => {
            if let Err(e) = super::text::draw_text(&mut pix, content, look.foreground) {
                warnings.push(format!("text: {e}"));
            }
        }
        ImageSource::Emoji { ch } => {
            if let Err(e) = super::text::draw_emoji(&mut pix, ch) {
                warnings.push(e);
            }
        }
        ImageSource::App { desktop_id } => match crate::apps::app_icon(desktop_id) {
            Some(path) => {
                if let Err(e) = draw_file(&mut pix, &path, Fit::Contain(look.icon_scale)) {
                    warnings.push(format!("icon of {desktop_id}: {e}"));
                }
            }
            None => warnings.push(format!("no icon found for app {desktop_id}")),
        },
        ImageSource::Plugin { plugin, icon } => {
            if let Err(e) = draw_plugin_icon(&mut pix, plugin, icon, look.foreground, look.icon_scale) {
                warnings.push(format!("icon {icon} of plugin {plugin}: {e}"));
            }
        }
        ImageSource::Steam { appid } => {
            match crate::steam::default_root().and_then(|r| crate::steam::art_for(&r, *appid)) {
                Some(path) => {
                    if let Err(e) = draw_file(&mut pix, &path, Fit::Cover) {
                        warnings.push(format!("Steam art for {appid}: {e}"));
                    }
                }
                None => warnings.push(format!("Steam has no artwork for game {appid} on this computer")),
            }
        }
    }
    if let Some(pos) = own_label
        && let Err(e) = super::text::draw_label(&mut pix, &look.label.text, pos)
    {
        warnings.push(format!("label: {e}"));
    }
    let png = pix.encode_png().ok()?;
    Some(Rendered { png, warnings })
}

/// Largest plugin icon file read.
const MAX_PLUGIN_ICON_BYTES: u64 = 512 * 1024;

/// A plugin's one-colour SVG tinted with `fg`; a PNG is drawn as it is.
fn draw_plugin_icon(pix: &mut Pixmap, plugin: &str, icon: &str, fg: Color, scale: f32) -> Result<(), String> {
    let path = crate::store::plugin_asset(plugin, icon).ok_or("not a file in the plugin's assets folder")?;
    let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if len > MAX_PLUGIN_ICON_BYTES {
        return Err(format!("larger than {} KiB", MAX_PLUGIN_ICON_BYTES / 1024));
    }
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        let svg = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        draw_svg(pix, &svg, fg, scale)
    } else {
        draw_image(pix, &path, Fit::Contain(scale))
    }
}

/// Plugin `plugin`'s icon file `icon` as a `px`-square PNG with a clear background: tinted with
/// `tint` (one-colour icons), else in its own colours.
pub fn plugin_icon_png(plugin: &str, icon: &str, px: u32, tint: Option<Color>) -> Option<Vec<u8>> {
    let path = crate::store::plugin_asset(plugin, icon)?;
    if std::fs::metadata(&path).ok()?.len() > MAX_PLUGIN_ICON_BYTES {
        return None;
    }
    let mut big = Pixmap::new(KEY_PX, KEY_PX)?;
    match tint {
        Some(fg) => draw_svg(&mut big, &std::fs::read_to_string(&path).ok()?, fg, 1.0).ok()?,
        None => draw_file(&mut big, &path, Fit::Contain(1.0)).ok()?,
    }
    let mut out = Pixmap::new(px, px)?;
    let k = px as f32 / KEY_PX as f32;
    out.draw_pixmap(
        0,
        0,
        big.as_ref(),
        &PixmapPaint {
            quality: tiny_skia::FilterQuality::Bicubic,
            ..PixmapPaint::default()
        },
        Transform::from_scale(k, k),
        None,
    );
    out.encode_png().ok()
}

fn draw_svg(pix: &mut Pixmap, svg: &str, fg: Color, scale: f32) -> Result<(), String> {
    let svg = super::icons::colorize(svg, fg);
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

/// How a picture fills the key.
#[derive(Debug, Clone, Copy)]
enum Fit {
    /// Fill the whole key, cropping the overflow.
    Cover,
    /// Fit inside a centred box of this share of the key.
    Contain(f32),
}

/// Draw an SVG or raster file.
fn draw_file(pix: &mut Pixmap, path: &Path, fit: Fit) -> Result<(), String> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let opts = usvg::Options {
            resources_dir: path.parent().map(Path::to_path_buf),
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_data(&data, &opts).map_err(|e| e.to_string())?;
        let share = match fit {
            Fit::Cover => 1.0,
            Fit::Contain(s) => s.clamp(0.05, 1.0),
        };
        let size = tree.size();
        let box_px = KEY_PX as f32 * share;
        let k = box_px / size.width().max(size.height());
        let (ox, oy) = (
            (KEY_PX as f32 - size.width() * k) / 2.0,
            (KEY_PX as f32 - size.height() * k) / 2.0,
        );
        resvg::render(
            &tree,
            Transform::from_scale(k, k).post_translate(ox, oy),
            &mut pix.as_mut(),
        );
        return Ok(());
    }
    draw_image(pix, path, fit)
}

fn draw_image(pix: &mut Pixmap, path: &Path, fit: Fit) -> Result<(), String> {
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
    let fitted = match fit {
        Fit::Cover => img
            .resize_to_fill(KEY_PX, KEY_PX, image::imageops::FilterType::Lanczos3)
            .to_rgba8(),
        Fit::Contain(share) => {
            let side = ((KEY_PX as f32) * share.clamp(0.05, 1.0)) as u32;
            let small = img
                .resize(side, side, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            let mut canvas = image::RgbaImage::new(KEY_PX, KEY_PX);
            let (x, y) = ((KEY_PX - small.width()) / 2, (KEY_PX - small.height()) / 2);
            image::imageops::overlay(&mut canvas, &small, x as i64, y as i64);
            canvas
        }
    };
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
