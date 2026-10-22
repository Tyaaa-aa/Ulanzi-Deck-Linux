use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use resvg::tiny_skia::{Paint, Pixmap, Rect, Transform};

/// Size, weight and colour of a line of text.
#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub size: f32,
    pub bold: bool,
    pub rgb: [u8; 3],
}

/// System fonts and a glyph cache for drawing text into pixmaps.
pub struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    family: String,
}

const PREFERRED: [&str; 4] = ["Adwaita Sans", "Cantarell", "Noto Sans", "DejaVu Sans"];

impl Fonts {
    /// Load the system's fonts (takes a moment; create once and reuse).
    pub fn system() -> Fonts {
        let system = FontSystem::new();
        let family = PREFERRED
            .iter()
            .find(|f| {
                system
                    .db()
                    .faces()
                    .any(|face| face.families.iter().any(|(n, _)| n == *f))
            })
            .map(|f| f.to_string())
            .unwrap_or_else(|| "sans-serif".into());
        Fonts {
            system,
            cache: SwashCache::new(),
            family,
        }
    }

    fn buffer(&mut self, text: &str, size: f32, bold: bool) -> Buffer {
        let mut b = Buffer::new(&mut self.system, Metrics::new(size, size * 1.2));
        let attrs = Attrs::new()
            .family(if self.family == "sans-serif" {
                Family::SansSerif
            } else {
                Family::Name(&self.family)
            })
            .weight(if bold { Weight::BOLD } else { Weight::NORMAL });
        b.set_size(None, None);
        b.set_text(text, &attrs, Shaping::Advanced, None);
        b.shape_until_scroll(&mut self.system, false);
        b
    }

    /// Width in pixels of `text` on one line.
    pub fn measure(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        let b = self.buffer(text, size, bold);
        b.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
    }

    /// Greedy word wrap to `max_w`, at most `max_lines`; an overflowing last line ends in "…".
    pub fn wrap(&mut self, text: &str, size: f32, bold: bool, max_w: f32, max_lines: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for word in text.split_whitespace() {
            let candidate = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if cur.is_empty() || self.measure(&candidate, size, bold) <= max_w {
                cur = candidate;
            } else {
                lines.push(std::mem::replace(&mut cur, word.to_string()));
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        if lines.len() > max_lines {
            lines.truncate(max_lines);
            if let Some(last) = lines.last_mut() {
                while !last.is_empty() && self.measure(&format!("{last}…"), size, bold) > max_w {
                    last.pop();
                }
                last.push('…');
            }
        }
        lines
    }

    /// Draw one line of text with its top-left at `(x, y)`. Returns the line height.
    pub fn draw(&mut self, pix: &mut Pixmap, text: &str, (x, y): (f32, f32), style: &TextStyle) -> f32 {
        let TextStyle { size, bold, rgb } = *style;
        let mut b = self.buffer(text, size, bold);
        let color = cosmic_text::Color::rgb(rgb[0], rgb[1], rgb[2]);
        let mut paint = Paint::default();
        b.draw(&mut self.system, &mut self.cache, color, |gx, gy, w, h, c| {
            if c.a() == 0 {
                return;
            }
            paint.set_color_rgba8(c.r(), c.g(), c.b(), c.a());
            if let Some(r) = Rect::from_xywh(x + gx as f32, y + gy as f32, w as f32, h as f32) {
                pix.fill_rect(r, &paint, Transform::identity(), None);
            }
        });
        size * 1.2
    }
}
