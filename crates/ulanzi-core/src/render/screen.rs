//! Widgets for the wide screen (458×196): clocks, system stats, Now Playing, timer, weather,
//! command output and pictures, plus the volume overlay. Pure: a widget and a data snapshot in,
//! a PNG out, so the app's preview and the deck use the same drawing.

// Drawing helpers take position, size and colour separately; bundling them would not help.
#![allow(clippy::too_many_arguments)]

use std::io::Cursor;
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDateTime, Timelike};
use resvg::tiny_skia::{
    self, FillRule, LineCap, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

use super::fonts::{Fonts, TextStyle};
use super::icons::IconLibrary;
use crate::Color;
use crate::model::{Background, ClockFace, IconSet, Stat, Widget};

pub const W: u32 = 458;
pub const H: u32 = 196;
const WF: f32 = W as f32;
const HF: f32 = H as f32;

/// How widget mode looks.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenStyle {
    pub background: Background,
    pub accent: Color,
    /// For background and widget pictures (paths are relative to it).
    pub profile_dir: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatsData {
    /// Percent histories, newest last.
    pub cpu: Vec<u8>,
    pub ram: Vec<u8>,
    pub gpu: Vec<u8>,
    pub temp: Option<i32>,
    /// Bytes per second.
    pub net_down: u64,
    pub net_up: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaData {
    pub title: String,
    pub artist: String,
    pub art: Option<PathBuf>,
    pub position_s: Option<u32>,
    pub length_s: Option<u32>,
    pub playing: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimerData {
    pub remaining_s: u32,
    pub total_s: u32,
    pub running: bool,
    /// "Focus" / "Break" for Pomodoro.
    pub phase: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WeatherData {
    Ok {
        temp: f32,
        /// WMO weather code.
        code: u8,
        place: String,
        high: f32,
        low: f32,
    },
    Err(String),
}

/// Everything a widget may show. Missing parts draw a short message.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenData {
    pub now: NaiveDateTime,
    pub stats: Option<StatsData>,
    pub media: Option<MediaData>,
    pub timer: Option<TimerData>,
    pub weather: Option<WeatherData>,
    pub command: Option<String>,
    /// The picture the Pictures widget shows now (absolute).
    pub picture: Option<PathBuf>,
}

impl Default for ScreenData {
    fn default() -> Self {
        ScreenData {
            now: chrono::Local::now().naive_local(),
            stats: None,
            media: None,
            timer: None,
            weather: None,
            command: None,
            picture: None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Pure helpers

/// ("09:41" or "09:41:07", "AM"/"PM" for 12-hour).
pub fn time_text(now: NaiveDateTime, hour24: bool, seconds: bool) -> (String, Option<String>) {
    let (h, suffix) = if hour24 {
        (now.hour(), None)
    } else {
        let (pm, h) = now.hour12();
        (h, Some(if pm { "PM" } else { "AM" }.to_string()))
    };
    let t = if seconds {
        format!("{h:02}:{:02}:{:02}", now.minute(), now.second())
    } else {
        format!("{h:02}:{:02}", now.minute())
    };
    (t, suffix)
}

/// Hour, minute and second hand angles in degrees clockwise from 12.
pub fn hand_angles(now: NaiveDateTime) -> (f32, f32, f32) {
    let s = now.second() as f32;
    let m = now.minute() as f32 + s / 60.0;
    let h = (now.hour() % 12) as f32 + m / 60.0;
    (h * 30.0, m * 6.0, s * 6.0)
}

/// Points of a sparkline for percent values in a `w`×`h` box (y grows down).
pub fn sparkline(values: &[u8], w: f32, h: f32) -> Vec<(f32, f32)> {
    let n = values.len();
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = if n <= 1 {
                0.0
            } else {
                i as f32 * w / (n - 1) as f32
            };
            (x, h - (*v).min(100) as f32 / 100.0 * h)
        })
        .collect()
}

/// A Lucide icon for a WMO weather code.
pub fn weather_icon(code: u8) -> &'static str {
    match code {
        0 => "sun",
        1 | 2 => "cloud-sun",
        3 => "cloud",
        45 | 48 => "cloud-fog",
        51..=57 => "cloud-drizzle",
        61..=67 | 80..=82 => "cloud-rain",
        71..=77 | 85 | 86 => "snowflake",
        95..=99 => "cloud-lightning",
        _ => "cloud",
    }
}

fn rate(bps: u64) -> String {
    match bps {
        b if b >= 1_000_000 => format!("{:.1} MB/s", b as f64 / 1e6),
        b if b >= 1_000 => format!("{:.0} kB/s", b as f64 / 1e3),
        b => format!("{b} B/s"),
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing helpers

fn paint(c: Color, alpha: f32) -> Paint<'static> {
    let mut p = Paint::default();
    let [r, g, b, a] = c.0;
    p.set_color_rgba8(r, g, b, ((a as f32) * alpha.clamp(0.0, 1.0)) as u8);
    p.anti_alias = true;
    p
}

fn rgb(c: Color) -> [u8; 3] {
    [c.0[0], c.0[1], c.0[2]]
}

const WHITE: Color = Color::WHITE;
fn dim() -> Color {
    Color::rgb(0xa8, 0xad, 0xb8)
}

fn rounded(pix: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, p: &Paint) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    if let Some(path) = pb.finish() {
        pix.fill_path(&path, p, FillRule::Winding, Transform::identity(), None);
    }
}

fn circle(pix: &mut Pixmap, cx: f32, cy: f32, r: f32, p: &Paint) {
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        pix.fill_path(&path, p, FillRule::Winding, Transform::identity(), None);
    }
}

fn line(pix: &mut Pixmap, a: (f32, f32), b: (f32, f32), width: f32, p: &Paint) {
    let mut pb = PathBuilder::new();
    pb.move_to(a.0, a.1);
    pb.line_to(b.0, b.1);
    if let Some(path) = pb.finish() {
        let s = Stroke {
            width,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        pix.stroke_path(&path, p, &s, Transform::identity(), None);
    }
}

/// An arc from 12 o'clock, clockwise, `fraction` of a full turn.
fn arc(pix: &mut Pixmap, cx: f32, cy: f32, r: f32, fraction: f32, width: f32, p: &Paint) {
    let steps = (fraction.clamp(0.0, 1.0) * 120.0).ceil() as usize;
    if steps == 0 {
        return;
    }
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let a = (i as f32 / 120.0).min(fraction) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    if let Some(path) = pb.finish() {
        let s = Stroke {
            width,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        pix.stroke_path(&path, p, &s, Transform::identity(), None);
    }
}

fn text(
    fonts: &mut Fonts,
    pix: &mut Pixmap,
    s: &str,
    (x, y): (f32, f32),
    size: f32,
    bold: bool,
    c: Color,
) -> f32 {
    fonts.draw(
        pix,
        s,
        (x, y),
        &TextStyle {
            size,
            bold,
            rgb: rgb(c),
        },
    );
    fonts.measure(s, size, bold)
}

fn text_centered(
    fonts: &mut Fonts,
    pix: &mut Pixmap,
    s: &str,
    cx: f32,
    y: f32,
    size: f32,
    bold: bool,
    c: Color,
) {
    let w = fonts.measure(s, size, bold);
    text(fonts, pix, s, (cx - w / 2.0, y), size, bold, c);
}

/// The largest size up to `max` at which `s` fits in `width`.
fn fit(fonts: &mut Fonts, s: &str, max: f32, width: f32, bold: bool) -> f32 {
    let mut size = max;
    while size > 12.0 && fonts.measure(s, size, bold) > width {
        size -= 2.0;
    }
    size
}

fn icon(pix: &mut Pixmap, name: &str, at: (f32, f32), size: f32, c: Color) {
    IconLibrary::bundled().draw_into(pix, IconSet::Lucide, name, at, size, c);
}

fn decode_cover(path: &Path, w: u32, h: u32) -> Option<image::RgbaImage> {
    let bytes = std::fs::read(path).ok()?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    Some(
        img.resize_to_fill(w, h, image::imageops::FilterType::Triangle)
            .to_rgba8(),
    )
}

fn blit(pix: &mut Pixmap, img: &image::RgbaImage, x: i32, y: i32, opacity: f32) {
    let Some(mut layer) = Pixmap::new(img.width(), img.height()) else {
        return;
    };
    for (dst, src) in layer.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    let p = PixmapPaint {
        opacity,
        ..PixmapPaint::default()
    };
    pix.draw_pixmap(x, y, layer.as_ref(), &p, Transform::identity(), None);
}

fn darken(pix: &mut Pixmap, amount: f32) {
    if let Some(r) = Rect::from_xywh(0.0, 0.0, WF, HF) {
        pix.fill_rect(r, &paint(Color::BLACK, amount), Transform::identity(), None);
    }
}

fn background(pix: &mut Pixmap, bg: &Background, profile_dir: &Path) {
    let full = Rect::from_xywh(0.0, 0.0, WF, HF);
    let gradient = |pix: &mut Pixmap, from: Color, to: Color, diagonal: bool| {
        let (a, b) = (rgb(from), rgb(to));
        let end = if diagonal {
            tiny_skia::Point::from_xy(WF, HF)
        } else {
            tiny_skia::Point::from_xy(0.0, HF)
        };
        let shader = tiny_skia::LinearGradient::new(
            tiny_skia::Point::from_xy(0.0, 0.0),
            end,
            vec![
                tiny_skia::GradientStop::new(0.0, tiny_skia::Color::from_rgba8(a[0], a[1], a[2], 255)),
                tiny_skia::GradientStop::new(1.0, tiny_skia::Color::from_rgba8(b[0], b[1], b[2], 255)),
            ],
            tiny_skia::SpreadMode::Pad,
            Transform::identity(),
        );
        if let (Some(shader), Some(r)) = (shader, full) {
            let p = Paint {
                shader,
                ..Paint::default()
            };
            pix.fill_rect(r, &p, Transform::identity(), None);
        }
    };
    match bg {
        Background::Theme => gradient(
            pix,
            Color::rgb(0x1b, 0x1d, 0x26),
            Color::rgb(0x0b, 0x0c, 0x10),
            false,
        ),
        Background::Solid { color } => {
            let [r, g, b] = rgb(*color);
            pix.fill(tiny_skia::Color::from_rgba8(r, g, b, 255));
        }
        Background::Gradient { from, to } => gradient(pix, *from, *to, true),
        Background::Picture { path } => {
            pix.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
            if let Some(img) = decode_cover(&profile_dir.join(path), W, H) {
                blit(pix, &img, 0, 0, 1.0);
                darken(pix, 0.45);
            }
        }
    }
}

fn message(fonts: &mut Fonts, pix: &mut Pixmap, glyph: &str, msg: &str) {
    icon(pix, glyph, (WF / 2.0 - 26.0, 42.0), 52.0, dim());
    let size = fit(fonts, msg, 22.0, WF - 40.0, false);
    text_centered(fonts, pix, msg, WF / 2.0, 112.0, size, false, dim());
}

// ---------------------------------------------------------------------------------------------
// Widgets

fn clock(
    fonts: &mut Fonts,
    pix: &mut Pixmap,
    face: ClockFace,
    hour24: bool,
    seconds: bool,
    show_date: bool,
    now: NaiveDateTime,
    accent: Color,
) {
    let (t, ampm) = time_text(now, hour24, seconds);
    let weekday = now.format("%A").to_string();
    let date_line = now.format("%A, %-d %B").to_string();
    match face {
        ClockFace::Digital => {
            let size = fit(fonts, &t, 96.0, WF - 70.0, true);
            let w = fonts.measure(&t, size, true);
            let ap_w = ampm
                .as_ref()
                .map(|a| fonts.measure(a, 24.0, true) + 8.0)
                .unwrap_or(0.0);
            let x = (WF - w - ap_w) / 2.0;
            let top = if show_date { 22.0 } else { 40.0 };
            text(fonts, pix, &t, (x, top), size, true, WHITE);
            if let Some(a) = &ampm {
                text(
                    fonts,
                    pix,
                    a,
                    (x + w + 8.0, top + size * 0.62),
                    24.0,
                    true,
                    accent,
                );
            }
            rounded(
                pix,
                WF / 2.0 - 36.0,
                top + size * 1.2 + 4.0,
                72.0,
                5.0,
                2.5,
                &paint(accent, 1.0),
            );
            if show_date {
                text_centered(
                    fonts,
                    pix,
                    &date_line,
                    WF / 2.0,
                    top + size * 1.2 + 22.0,
                    24.0,
                    false,
                    dim(),
                );
            }
        }
        ClockFace::Analog => {
            let (cx, cy, r) = (100.0, HF / 2.0, 82.0);
            circle(pix, cx, cy, r, &paint(Color::rgb(0x22, 0x25, 0x30), 1.0));
            for i in 0..12 {
                let a = (i as f32 * 30.0 - 90.0).to_radians();
                let major = i % 3 == 0;
                let (r0, r1) = if major {
                    (r - 18.0, r - 6.0)
                } else {
                    (r - 12.0, r - 6.0)
                };
                let p = if major {
                    paint(accent, 1.0)
                } else {
                    paint(dim(), 0.7)
                };
                line(
                    pix,
                    (cx + r0 * a.cos(), cy + r0 * a.sin()),
                    (cx + r1 * a.cos(), cy + r1 * a.sin()),
                    if major { 4.0 } else { 2.0 },
                    &p,
                );
            }
            let (h, m, s) = hand_angles(now);
            let hand = |pix: &mut Pixmap, deg: f32, len: f32, width: f32, p: &Paint| {
                let a = (deg - 90.0).to_radians();
                line(pix, (cx, cy), (cx + len * a.cos(), cy + len * a.sin()), width, p);
            };
            hand(pix, h, 44.0, 7.0, &paint(WHITE, 1.0));
            hand(pix, m, 64.0, 4.5, &paint(WHITE, 1.0));
            if seconds {
                hand(pix, s, 70.0, 2.0, &paint(accent, 1.0));
            }
            circle(pix, cx, cy, 6.0, &paint(accent, 1.0));
            let x = 210.0;
            let ws = fit(fonts, &weekday, 34.0, WF - x - 16.0, true);
            text(fonts, pix, &weekday, (x, 34.0), ws, true, WHITE);
            if show_date {
                text(
                    fonts,
                    pix,
                    &now.format("%-d %B %Y").to_string(),
                    (x, 82.0),
                    22.0,
                    false,
                    dim(),
                );
            }
            text(
                fonts,
                pix,
                &(t.clone() + ampm.as_deref().map(|a| format!(" {a}")).as_deref().unwrap_or("")),
                (x, 122.0),
                30.0,
                true,
                accent,
            );
        }
        ClockFace::Minimal => {
            let size = fit(fonts, &t, 110.0, WF - 40.0, false);
            let top = if show_date { 18.0 } else { 34.0 };
            text_centered(fonts, pix, &t, WF / 2.0, top, size, false, WHITE);
            let mut line_text = String::new();
            if let Some(a) = &ampm {
                line_text.push_str(a);
            }
            if show_date {
                if !line_text.is_empty() {
                    line_text.push_str("  ·  ");
                }
                line_text.push_str(&now.format("%a %-d %b").to_string());
            }
            if !line_text.is_empty() {
                text_centered(
                    fonts,
                    pix,
                    &line_text,
                    WF / 2.0,
                    top + size * 1.18 + 4.0,
                    22.0,
                    false,
                    accent,
                );
            }
        }
        ClockFace::Split => {
            let size = fit(fonts, &t, 84.0, 260.0, true);
            let w = fonts.measure(&t, size, true);
            text(
                fonts,
                pix,
                &t,
                ((290.0 - w) / 2.0, (HF - size * 1.2) / 2.0),
                size,
                true,
                WHITE,
            );
            if let Some(a) = &ampm {
                text_centered(fonts, pix, a, 145.0, (HF + size * 1.2) / 2.0, 20.0, true, dim());
            }
            rounded(pix, 298.0, 40.0, 5.0, HF - 80.0, 2.5, &paint(accent, 1.0));
            let x = 318.0;
            let ws = fit(fonts, &weekday, 30.0, WF - x - 10.0, true);
            text(fonts, pix, &weekday, (x, 44.0), ws, true, WHITE);
            if show_date {
                text(
                    fonts,
                    pix,
                    &now.format("%-d %b").to_string(),
                    (x, 86.0),
                    28.0,
                    false,
                    accent,
                );
                text(
                    fonts,
                    pix,
                    &now.year().to_string(),
                    (x, 124.0),
                    22.0,
                    false,
                    dim(),
                );
            }
        }
    }
}

fn spark(pix: &mut Pixmap, values: &[u8], x: f32, y: f32, w: f32, h: f32, accent: Color) {
    let pts = sparkline(values, w, h);
    if pts.len() < 2 {
        return;
    }
    let mut fill = PathBuilder::new();
    fill.move_to(x, y + h);
    for (px, py) in &pts {
        fill.line_to(x + px, y + py);
    }
    fill.line_to(x + w, y + h);
    fill.close();
    if let Some(path) = fill.finish() {
        pix.fill_path(
            &path,
            &paint(accent, 0.22),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    let mut pb = PathBuilder::new();
    for (i, (px, py)) in pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(x + px, y + py);
        } else {
            pb.line_to(x + px, y + py);
        }
    }
    if let Some(path) = pb.finish() {
        pix.stroke_path(
            &path,
            &paint(accent, 1.0),
            &Stroke {
                width: 2.5,
                ..Stroke::default()
            },
            Transform::identity(),
            None,
        );
    }
}

fn system(
    fonts: &mut Fonts,
    pix: &mut Pixmap,
    stats: &[Stat],
    graphs: bool,
    d: Option<&StatsData>,
    accent: Color,
) {
    let Some(d) = d else {
        return message(fonts, pix, "cpu", "Reading system stats…");
    };
    let shown: Vec<&Stat> = stats.iter().take(4).collect();
    if shown.is_empty() {
        return message(fonts, pix, "cpu", "Choose what to show");
    }
    let n = shown.len() as f32;
    let tile = (WF - 16.0 - 10.0 * (n - 1.0)) / n;
    for (i, st) in shown.iter().enumerate() {
        let x = 8.0 + i as f32 * (tile + 10.0);
        rounded(
            pix,
            x,
            8.0,
            tile,
            HF - 16.0,
            16.0,
            &paint(Color::rgb(0xff, 0xff, 0xff), 0.05),
        );
        let (label, value, hist): (&str, String, &[u8]) = match st {
            Stat::Cpu => (
                "CPU",
                d.cpu.last().map(|v| format!("{v}%")).unwrap_or("–".into()),
                &d.cpu,
            ),
            Stat::Ram => (
                "Memory",
                d.ram.last().map(|v| format!("{v}%")).unwrap_or("–".into()),
                &d.ram,
            ),
            Stat::Gpu => (
                "GPU",
                d.gpu.last().map(|v| format!("{v}%")).unwrap_or("–".into()),
                &d.gpu,
            ),
            Stat::Temp => (
                "Temp",
                d.temp.map(|t| format!("{t}°C")).unwrap_or("–".into()),
                &[],
            ),
            Stat::Net => ("Network", format!("↓ {}", rate(d.net_down)), &[]),
        };
        text(fonts, pix, label, (x + 14.0, 20.0), 18.0, false, dim());
        let size = fit(fonts, &value, 44.0, tile - 24.0, true);
        text(fonts, pix, &value, (x + 14.0, 46.0), size, true, WHITE);
        if **st == Stat::Net {
            text(
                fonts,
                pix,
                &format!("↑ {}", rate(d.net_up)),
                (x + 14.0, 108.0),
                18.0,
                false,
                dim(),
            );
        }
        if graphs {
            spark(pix, hist, x + 12.0, 128.0, tile - 24.0, 46.0, accent);
        }
    }
}

fn now_playing(
    fonts: &mut Fonts,
    pix: &mut Pixmap,
    progress: bool,
    blur: bool,
    d: Option<&MediaData>,
    accent: Color,
) {
    let Some(m) = d.filter(|m| !m.title.is_empty()) else {
        return message(fonts, pix, "music", "Nothing playing");
    };
    let art = m.art.as_deref().and_then(|p| decode_cover(p, 164, 164));
    if blur && let Some(path) = m.art.as_deref() {
        // A soft, dark wash of the cover behind everything.
        if let Some(small) = decode_cover(path, 23, 10) {
            let big = image::imageops::resize(&small, W, H, image::imageops::FilterType::Triangle);
            blit(pix, &big, 0, 0, 1.0);
            darken(pix, 0.5);
        }
    }
    match &art {
        Some(img) => {
            rounded(pix, 14.0, 14.0, 168.0, 168.0, 14.0, &paint(Color::BLACK, 0.35));
            blit(pix, img, 16, 16, 1.0);
        }
        None => {
            rounded(
                pix,
                16.0,
                16.0,
                164.0,
                164.0,
                14.0,
                &paint(Color::rgb(0x2a, 0x2e, 0x3a), 1.0),
            );
            icon(pix, "music", (58.0, 58.0), 80.0, dim());
        }
    }
    let x = 200.0;
    let tw = WF - x - 16.0;
    let mut y = 22.0;
    for l in fonts.wrap(&m.title, 26.0, true, tw, 2) {
        y += text(fonts, pix, &l, (x, y), 26.0, true, WHITE).min(0.0) + 32.0;
    }
    for l in fonts.wrap(&m.artist, 20.0, false, tw, 1) {
        text(fonts, pix, &l, (x, y + 4.0), 20.0, false, dim());
    }
    icon(
        pix,
        if m.playing { "play" } else { "pause" },
        (x, 128.0),
        22.0,
        accent,
    );
    if progress && let (Some(p), Some(len)) = (m.position_s, m.length_s.filter(|l| *l > 0)) {
        let bar = WF - x - 16.0;
        rounded(pix, x, 166.0, bar, 6.0, 3.0, &paint(Color::WHITE, 0.15));
        rounded(
            pix,
            x,
            166.0,
            bar * (p.min(len) as f32 / len as f32),
            6.0,
            3.0,
            &paint(accent, 1.0),
        );
        let t = format!("{}:{:02} / {}:{:02}", p / 60, p % 60, len / 60, len % 60);
        text(fonts, pix, &t, (x + 32.0, 129.0), 18.0, false, dim());
    }
}

fn timer(fonts: &mut Fonts, pix: &mut Pixmap, minutes: u32, d: Option<&TimerData>, accent: Color) {
    let fallback = TimerData {
        remaining_s: minutes * 60,
        total_s: minutes * 60,
        running: false,
        phase: None,
    };
    let t = d.unwrap_or(&fallback);
    let mm = format!("{:02}:{:02}", t.remaining_s / 60, t.remaining_s % 60);
    let size = fit(fonts, &mm, 92.0, 270.0, true);
    text(
        fonts,
        pix,
        &mm,
        (24.0, (HF - size * 1.25) / 2.0),
        size,
        true,
        WHITE,
    );
    let label = t.phase.clone().unwrap_or_else(|| {
        if t.running {
            "Running".into()
        } else if t.remaining_s == t.total_s {
            "Ready".into()
        } else {
            "Paused".into()
        }
    });
    text(
        fonts,
        pix,
        &label,
        (28.0, (HF + size * 1.25) / 2.0 - 6.0),
        22.0,
        false,
        accent,
    );
    let (cx, cy, r) = (370.0, HF / 2.0, 64.0);
    arc(pix, cx, cy, r, 1.0, 12.0, &paint(Color::WHITE, 0.12));
    let frac = if t.total_s == 0 {
        0.0
    } else {
        t.remaining_s as f32 / t.total_s as f32
    };
    arc(pix, cx, cy, r, frac, 12.0, &paint(accent, 1.0));
    icon(
        pix,
        if t.running { "pause" } else { "play" },
        (cx - 22.0, cy - 22.0),
        44.0,
        WHITE,
    );
}

fn weather(
    fonts: &mut Fonts,
    pix: &mut Pixmap,
    place: &str,
    fahrenheit: bool,
    d: Option<&WeatherData>,
    accent: Color,
) {
    if place.trim().is_empty() {
        return message(fonts, pix, "map-pin", "Choose a town in the screen settings");
    }
    match d {
        None => message(fonts, pix, "cloud", &format!("Getting the weather for {place}…")),
        Some(WeatherData::Err(e)) => message(fonts, pix, "cloud-off", &format!("Weather unavailable: {e}")),
        Some(WeatherData::Ok {
            temp,
            code,
            place,
            high,
            low,
        }) => {
            icon(pix, weather_icon(*code), (26.0, 34.0), 128.0, accent);
            let unit = if fahrenheit { "°F" } else { "°C" };
            text(
                fonts,
                pix,
                &format!("{temp:.0}{unit}"),
                (180.0, 22.0),
                72.0,
                true,
                WHITE,
            );
            let ps = fit(fonts, place, 26.0, WF - 196.0, false);
            text(fonts, pix, place, (184.0, 112.0), ps, false, WHITE);
            text(
                fonts,
                pix,
                &format!("H {high:.0}°  ·  L {low:.0}°"),
                (184.0, 148.0),
                20.0,
                false,
                dim(),
            );
        }
    }
}

fn command(fonts: &mut Fonts, pix: &mut Pixmap, title: &str, cmd: &str, out: Option<&str>, accent: Color) {
    if cmd.trim().is_empty() {
        return message(fonts, pix, "terminal", "Set a command in the screen settings");
    }
    let title = if title.trim().is_empty() { cmd } else { title };
    let ts = fit(fonts, title, 22.0, WF - 40.0, true);
    text(fonts, pix, title, (20.0, 20.0), ts, true, accent);
    let body = out
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .unwrap_or("No output yet");
    let mut y = 62.0;
    for l in fonts.wrap(&body.replace('\n', " "), 34.0, false, WF - 40.0, 2) {
        text(fonts, pix, &l, (20.0, y), 34.0, false, WHITE);
        y += 46.0;
    }
}

fn pictures(fonts: &mut Fonts, pix: &mut Pixmap, picture: Option<&Path>) {
    match picture.and_then(|p| decode_cover(p, W, H)) {
        Some(img) => blit(pix, &img, 0, 0, 1.0),
        None => message(fonts, pix, "image", "Add pictures in the screen settings"),
    }
}

/// Draw `w` with `data` and encode it as PNG (458×196).
pub fn render_widget(w: &Widget, style: &ScreenStyle, data: &ScreenData, fonts: &mut Fonts) -> Vec<u8> {
    let Some(mut pix) = Pixmap::new(W, H) else {
        return Vec::new();
    };
    background(&mut pix, &style.background, &style.profile_dir);
    let a = style.accent;
    match w {
        Widget::Blank => {}
        Widget::Clock {
            face,
            hour24,
            seconds,
            date,
        } => clock(fonts, &mut pix, *face, *hour24, *seconds, *date, data.now, a),
        Widget::NowPlaying {
            progress,
            blur_background,
        } => now_playing(
            fonts,
            &mut pix,
            *progress,
            *blur_background,
            data.media.as_ref(),
            a,
        ),
        Widget::System { stats, graphs } => system(fonts, &mut pix, stats, *graphs, data.stats.as_ref(), a),
        Widget::Timer { minutes, .. } => timer(fonts, &mut pix, *minutes, data.timer.as_ref(), a),
        Widget::Weather { place, fahrenheit } => {
            weather(fonts, &mut pix, place, *fahrenheit, data.weather.as_ref(), a)
        }
        Widget::Command { cmd, title, .. } => {
            command(fonts, &mut pix, title, cmd, data.command.as_deref(), a)
        }
        Widget::Pictures { .. } => pictures(fonts, &mut pix, data.picture.as_deref()),
    }
    pix.encode_png().unwrap_or_default()
}

/// The volume overlay drawn over `base` (or a plain backdrop when the screen shows a firmware mode).
pub fn volume_overlay(
    base: Option<&[u8]>,
    percent: u8,
    muted: bool,
    accent: Color,
    fonts: &mut Fonts,
) -> Vec<u8> {
    let Some(mut pix) = Pixmap::new(W, H) else {
        return Vec::new();
    };
    background(&mut pix, &Background::Theme, Path::new("/"));
    if let Some(img) = base.and_then(|b| image::load_from_memory(b).ok()) {
        blit(&mut pix, &img.to_rgba8(), 0, 0, 1.0);
        darken(&mut pix, 0.55);
    }
    rounded(
        &mut pix,
        24.0,
        38.0,
        WF - 48.0,
        120.0,
        28.0,
        &paint(Color::rgb(0x14, 0x15, 0x1b), 0.92),
    );
    let glyph = if muted || percent == 0 {
        "volume-x"
    } else if percent < 50 {
        "volume-1"
    } else {
        "volume-2"
    };
    icon(
        &mut pix,
        glyph,
        (50.0, 66.0),
        64.0,
        if muted { dim() } else { WHITE },
    );
    let (bx, bw) = (136.0, WF - 136.0 - 120.0);
    rounded(&mut pix, bx, 92.0, bw, 12.0, 6.0, &paint(Color::WHITE, 0.15));
    let fill = if muted {
        0.0
    } else {
        percent.min(100) as f32 / 100.0
    };
    rounded(
        &mut pix,
        bx,
        92.0,
        (bw * fill).max(0.0),
        12.0,
        6.0,
        &paint(accent, 1.0),
    );
    let label = if muted {
        "Muted".to_string()
    } else {
        format!("{percent}%")
    };
    let size = fit(fonts, &label, 34.0, 96.0, true);
    text(fonts, &mut pix, &label, (WF - 112.0, 76.0), size, true, WHITE);
    pix.encode_png().unwrap_or_default()
}
