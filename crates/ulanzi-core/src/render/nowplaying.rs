//! The Now Playing image for the wide screen (same layout as the Python version).

use std::io::Cursor;
use std::path::Path;

use resvg::tiny_skia::{self, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use super::fonts::{Fonts, TextStyle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
}

const W: u32 = 392;
const H: u32 = 196;
const BG: [u8; 3] = [0x13, 0x1c, 0x2e];
const ARTIST: [u8; 3] = [0xb4, 0xc8, 0xf0];
const TEXT_X: f32 = 200.0;
const TEXT_W: f32 = 180.0;

/// Draw on a 392×196 canvas, then scale to `out` (w, h) and encode as PNG.
pub fn render_now_playing(
    info: &NowPlaying,
    art: Option<&Path>,
    fonts: &mut Fonts,
    out: (u32, u32),
) -> Vec<u8> {
    let Some(mut pix) = Pixmap::new(W, H) else {
        return Vec::new();
    };
    pix.fill(tiny_skia::Color::from_rgba8(BG[0], BG[1], BG[2], 255));
    if !art.is_some_and(|a| draw_art(&mut pix, a)) {
        draw_disc(&mut pix);
    }
    let title = if info.title.is_empty() {
        "No media"
    } else {
        &info.title
    };
    let mut y = 40.0;
    for line in fonts.wrap(title, 24.0, true, TEXT_W, 2) {
        y += fonts.draw(
            &mut pix,
            &line,
            (TEXT_X, y),
            &TextStyle {
                size: 24.0,
                bold: true,
                rgb: [255, 255, 255],
            },
        );
    }
    let mut y = (y + 4.0).max(110.0);
    for line in fonts.wrap(&info.artist, 20.0, false, TEXT_W, 2) {
        y += fonts.draw(
            &mut pix,
            &line,
            (TEXT_X, y),
            &TextStyle {
                size: 20.0,
                bold: false,
                rgb: ARTIST,
            },
        );
    }
    scale_png(&pix, out)
}

fn draw_art(pix: &mut Pixmap, path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let Ok(reader) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format() else {
        return false;
    };
    let mut reader = reader;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    reader.limits(limits);
    let Ok(img) = reader.decode() else {
        return false;
    };
    let art = img
        .resize_to_fill(176, 176, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let Some(mut layer) = Pixmap::new(176, 176) else {
        return false;
    };
    for (d, s) in layer.pixels_mut().iter_mut().zip(art.pixels()) {
        *d = tiny_skia::ColorU8::from_rgba(s[0], s[1], s[2], s[3]).premultiply();
    }
    pix.draw_pixmap(
        10,
        10,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    true
}

fn circle(cx: f32, cy: f32, r: f32) -> Option<tiny_skia::Path> {
    PathBuilder::from_circle(cx, cy, r)
}

fn draw_disc(pix: &mut Pixmap) {
    let (cx, cy) = (98.0, 98.0);
    let mut fill = Paint::default();
    let mut edge = Paint::default();
    edge.set_color_rgba8(100, 120, 160, 255);
    let stroke = Stroke {
        width: 2.0,
        ..Stroke::default()
    };
    for (r, rgb) in [(80.0, [40, 50, 75]), (25.0, BG), (8.0, [100, 120, 160])] {
        if let Some(c) = circle(cx, cy, r) {
            fill.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
            pix.fill_path(&c, &fill, FillRule::Winding, Transform::identity(), None);
            pix.stroke_path(&c, &edge, &stroke, Transform::identity(), None);
        }
    }
    let mut pb = PathBuilder::new();
    pb.move_to(cx - 4.0, cy - 8.0);
    pb.line_to(cx + 8.0, cy);
    pb.line_to(cx - 4.0, cy + 8.0);
    pb.close();
    if let Some(tri) = pb.finish() {
        fill.set_color_rgba8(255, 255, 255, 255);
        pix.fill_path(&tri, &fill, FillRule::Winding, Transform::identity(), None);
    }
}

fn scale_png(pix: &Pixmap, (w, h): (u32, u32)) -> Vec<u8> {
    let (w, h) = (w.max(1), h.max(1));
    let rgba = image::RgbaImage::from_fn(W, H, |x, y| {
        let p = pix
            .pixel(x, y)
            .map(|p| p.demultiply())
            .unwrap_or(tiny_skia::ColorU8::from_rgba(0, 0, 0, 0));
        image::Rgba([p.red(), p.green(), p.blue(), p.alpha()])
    });
    let scaled = if (w, h) == (W, H) {
        rgba
    } else {
        image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Lanczos3)
    };
    let mut out = Vec::new();
    let _ =
        image::DynamicImage::ImageRgba8(scaled).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png);
    out
}
