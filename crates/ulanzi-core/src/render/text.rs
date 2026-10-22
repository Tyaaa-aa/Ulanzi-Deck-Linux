//! Text and emoji key faces.

use std::sync::{Mutex, OnceLock};

use resvg::tiny_skia::Pixmap;

use super::fonts::{Fonts, TextStyle};
use crate::Color;
use crate::model::LabelPosition;

/// One font system for the process (loading system fonts is slow).
fn fonts() -> &'static Mutex<Fonts> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONTS.get_or_init(|| Mutex::new(Fonts::system()))
}

const MARGIN: f32 = 0.08;

/// Draw `text` centred, wrapped to at most two lines, as large as fits (28-64 px).
pub fn draw_text(pix: &mut Pixmap, text: &str, fg: Color) -> Result<(), String> {
    let mut f = fonts().lock().map_err(|_| "font lock poisoned")?;
    let side = pix.width() as f32;
    let max_w = side * (1.0 - 2.0 * MARGIN);
    let mut chosen = None;
    for size in (28..=64).rev().step_by(4) {
        let size = size as f32;
        let lines = f.wrap(text, size, true, max_w, 2);
        let fits = lines.iter().all(|l| f.measure(l, size, true) <= max_w)
            && lines.len() as f32 * size * 1.2 <= side * (1.0 - 2.0 * MARGIN);
        if fits && !lines.iter().any(|l| l.ends_with('…')) {
            chosen = Some((size, lines));
            break;
        }
        chosen = Some((size, lines));
    }
    let (size, lines) = chosen.ok_or("no text")?;
    let style = TextStyle {
        size,
        bold: true,
        rgb: [fg.0[0], fg.0[1], fg.0[2]],
    };
    let total = lines.len() as f32 * size * 1.2;
    let mut y = (side - total) / 2.0;
    for line in lines {
        let w = f.measure(&line, size, true);
        y += f.draw(pix, &line, ((side - w) / 2.0, y), &style);
    }
    Ok(())
}

/// Draw one emoji centred at about 70 % of the key. Errors when no colour emoji font is installed.
pub fn draw_emoji(pix: &mut Pixmap, ch: &str) -> Result<(), String> {
    let mut f = fonts().lock().map_err(|_| "font lock poisoned")?;
    if !f.has_color_emoji() {
        return Err("no colour emoji font is installed (e.g. Noto Color Emoji)".into());
    }
    let side = pix.width() as f32;
    let size = side * 0.62;
    let style = TextStyle {
        size,
        bold: false,
        rgb: [255, 255, 255],
    };
    let w = f.measure_emoji(ch, size);
    let y = (side - size * 1.2) / 2.0;
    f.draw_emoji(pix, ch, ((side - w) / 2.0, y), &style);
    Ok(())
}

/// Draw a key label at `position`: bold white with a dark outline so it reads on any image, shrunk
/// to fit one line.
pub fn draw_label(pix: &mut Pixmap, text: &str, position: LabelPosition) -> Result<(), String> {
    let mut f = fonts().lock().map_err(|_| "font lock poisoned")?;
    let side = pix.width() as f32;
    let max_w = side * 0.92;
    let mut size = 30.0;
    while size > 16.0 && f.measure(text, size, true) > max_w {
        size -= 2.0;
    }
    let line = f
        .wrap(text, size, true, max_w, 1)
        .into_iter()
        .next()
        .unwrap_or_default();
    let w = f.measure(&line, size, true);
    let h = size * 1.2;
    let y = match position {
        LabelPosition::Top => side * 0.05,
        LabelPosition::Middle => (side - h) / 2.0,
        LabelPosition::Bottom => side * 0.95 - h,
    };
    let x = (side - w) / 2.0;
    let shadow = TextStyle {
        size,
        bold: true,
        rgb: [0, 0, 0],
    };
    for (dx, dy) in [
        (-2.0, 0.0),
        (2.0, 0.0),
        (0.0, -2.0),
        (0.0, 2.0),
        (1.5, 1.5),
        (-1.5, 1.5),
        (1.5, -1.5),
        (-1.5, -1.5),
    ] {
        f.draw(pix, &line, (x + dx, y + dy), &shadow);
    }
    let style = TextStyle {
        size,
        bold: true,
        rgb: [255, 255, 255],
    };
    f.draw(pix, &line, (x, y), &style);
    Ok(())
}
