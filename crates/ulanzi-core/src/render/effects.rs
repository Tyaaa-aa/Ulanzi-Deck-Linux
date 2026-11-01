//! The border effects. Each works out a pixel from where it is and how far through the loop it is,
//! so every loop closes on itself and any colour works.

use image::{Rgba, RgbaImage};

use crate::Color;
use crate::model::{Effect, EffectKind, EffectSpeed};

/// Frames in an effect's loop.
pub const LOOP_FRAMES: u32 = 24;
/// The most a key's GIF may weigh. The deck shows a question mark from about 300 KB.
pub const GIF_TARGET: usize = 250 * 1024;

const FALLBACK: Color = Color([0x35, 0x84, 0xe4, 255]);
/// Below this relative luminance a border can't be seen on a dark key.
const MIN_LUMINANCE: f32 = 0.15;

pub fn frame_ms(speed: EffectSpeed) -> u32 {
    match speed {
        EffectSpeed::Slow => 100,
        EffectSpeed::Normal => 66,
        EffectSpeed::Fast => 40,
    }
}

fn channel(v: u8) -> f32 {
    let v = v as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn can_be_seen(c: Color) -> bool {
    let [r, g, b, _] = c.0;
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b) >= MIN_LUMINANCE
}

/// The colour `effect` is drawn in: its own, or `own` (the key's or plugin's), then the desktop's
/// accent, then blue. `None` is Fire's own red, orange and yellow.
pub fn effect_color(effect: &Effect, own: Color, accent: Option<Color>) -> Option<Color> {
    if effect.color.is_some() || effect.kind == EffectKind::Fire {
        return effect.color;
    }
    Some(
        [Some(own), accent]
            .into_iter()
            .flatten()
            .find(|c| can_be_seen(*c))
            .unwrap_or(FALLBACK),
    )
}

/// How far `x, y` is from the edge, and how far round the border it is, 0 to 1.
fn border(x: u32, y: u32) -> (f32, f32) {
    let (x, y) = (x as f32, y as f32);
    let (right, bottom) = (195.0 - x, 195.0 - y);
    let d = x.min(right).min(y).min(bottom);
    let side = if d == y {
        x / 196.0
    } else if d == right {
        1.0 + y / 196.0
    } else if d == bottom {
        2.0 + right / 196.0
    } else {
        3.0 + bottom / 196.0
    };
    (d, side / 4.0)
}

fn rgb(c: Color) -> [f32; 3] {
    [c.0[0], c.0[1], c.0[2]].map(|v| v as f32 / 255.0)
}

/// Fire's colour at heat `v`: its own ramp, or black through `color` to near white.
fn flame(v: f32, color: Option<Color>) -> [f32; 3] {
    match color {
        None => [
            (v * 2.2).min(1.0),
            (v * 1.6 - 0.35).clamp(0.0, 1.0),
            (v * 2.0 - 1.4).clamp(0.0, 1.0),
        ],
        Some(c) => {
            let c = rgb(c);
            let (low, high) = ((v * 2.2).min(1.0), (v * 2.0 - 1.1).clamp(0.0, 1.0));
            [0, 1, 2].map(|i| (c[i] * low) * (1.0 - high * 0.9) + high * 0.9)
        }
    }
}

/// Colour and opacity of `kind` at a border position. Beyond `REACH` nothing is drawn.
fn pixel(kind: EffectKind, color: Option<Color>, d: f32, u: f32, t: f32) -> ([f32; 3], f32) {
    use std::f32::consts::TAU;
    let plain = color.map_or([1.0; 3], rgb);
    match kind {
        EffectKind::None => ([0.0; 3], 0.0),
        EffectKind::Glow => {
            let v = (1.0 - d / 26.0).clamp(0.0, 1.0);
            (plain, v * v * 0.8)
        }
        EffectKind::Pulse => {
            let v = (1.0 - d / 26.0).clamp(0.0, 1.0);
            (plain, v * v * (0.55 + 0.45 * (TAU * t).sin()))
        }
        EffectKind::Chase => {
            let v = (1.0 - d / 20.0).clamp(0.0, 1.0);
            let trail = (1.0 - (t - u).rem_euclid(1.0) / 0.35).clamp(0.0, 1.0);
            (plain, v * (0.12 + 0.88 * trail * trail))
        }
        EffectKind::Fire => {
            let h = 0.55
                + 0.25 * (TAU * (5.0 * u + 2.0 * t)).sin()
                + 0.15 * (TAU * (11.0 * u - 3.0 * t) + 1.3).sin()
                + 0.10 * (TAU * (23.0 * u + 5.0 * t) + 0.7).sin();
            let v = (1.0 - d / (46.0 * h.max(0.1))).clamp(0.0, 1.0);
            (flame(v, color), (v * 2.2).min(1.0))
        }
    }
}

const REACH: f32 = 50.0;

/// `base` with `effect` drawn over it, `t` of the way through the loop (0 to 1).
pub fn draw_effect(base: &RgbaImage, effect: &Effect, color: Option<Color>, t: f32) -> RgbaImage {
    let mut out = base.clone();
    if effect.is_none() {
        return out;
    }
    for (x, y, p) in out.enumerate_pixels_mut() {
        let (d, u) = border(x, y);
        if d >= REACH {
            continue;
        }
        let (c, a) = pixel(effect.kind, color, d, u, t);
        let under = [p.0[0], p.0[1], p.0[2]].map(|v| v as f32 / 255.0);
        let [r, g, b] = [0, 1, 2].map(|i| ((under[i] * (1.0 - a) + c[i] * a) * 255.0) as u8);
        *p = Rgba([r, g, b, 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLUE: Color = Color([80, 180, 255, 255]);

    fn dark() -> RgbaImage {
        RgbaImage::from_pixel(196, 196, Rgba([20, 20, 20, 255]))
    }

    /// The largest change of any channel between two images.
    fn step(a: &RgbaImage, b: &RgbaImage) -> u32 {
        a.pixels()
            .zip(b.pixels())
            .flat_map(|(p, q)| (0..3).map(move |i| p.0[i].abs_diff(q.0[i]) as u32))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn every_effect_closes_its_loop() {
        for kind in [EffectKind::Glow, EffectKind::Pulse, EffectKind::Chase, EffectKind::Fire] {
            let e = Effect::of(kind);
            let at = |f: u32| draw_effect(&dark(), &e, Some(BLUE), f as f32 / LOOP_FRAMES as f32);
            let (last, first, next) = (at(LOOP_FRAMES - 1), at(0), at(1));
            assert!(step(&last, &first) <= step(&first, &next) + 40, "{kind:?}");
        }
    }

    #[test]
    fn effects_stay_on_the_border() {
        for kind in [EffectKind::Glow, EffectKind::Pulse, EffectKind::Chase, EffectKind::Fire] {
            let out = draw_effect(&dark(), &Effect::of(kind), Some(BLUE), 0.3);
            assert_eq!(out.get_pixel(98, 98), dark().get_pixel(98, 98), "{kind:?}");
        }
    }

    #[test]
    fn no_effect_changes_nothing() {
        assert_eq!(draw_effect(&dark(), &Effect::default(), None, 0.5), dark());
    }

    #[test]
    fn fire_takes_any_colour() {
        let black = RgbaImage::from_pixel(196, 196, Rgba([0, 0, 0, 255]));
        let blue = Some(Color([0, 0, 255, 255]));
        let e = Effect::of(EffectKind::Fire);
        let [r, _, b, _] = draw_effect(&black, &e, blue, 0.0).get_pixel(98, 3).0;
        assert!(b > r);
        let [r, _, b, _] = draw_effect(&black, &e, None, 0.0).get_pixel(98, 3).0;
        assert!(r > b, "its own ramp is warm");
    }

    #[test]
    fn a_dark_colour_falls_to_the_accent_and_then_blue() {
        let e = Effect::of(EffectKind::Pulse);
        let accent = Color([255, 120, 0, 255]);
        assert_eq!(effect_color(&e, accent, None), Some(accent));
        assert_eq!(effect_color(&e, Color::BLACK, Some(accent)), Some(accent));
        assert_eq!(effect_color(&e, Color::BLACK, None), Some(FALLBACK));
        assert_eq!(effect_color(&e, Color::BLACK, Some(Color::BLACK)), Some(FALLBACK));
    }

    #[test]
    fn a_chosen_colour_wins_and_fire_keeps_its_own() {
        let mut e = Effect::of(EffectKind::Fire);
        assert_eq!(effect_color(&e, Color::WHITE, None), None);
        e.color = Some(Color([1, 2, 3, 255]));
        assert_eq!(effect_color(&e, Color::WHITE, None), e.color);
    }

    #[test]
    fn speeds_are_frame_times() {
        assert_eq!(frame_ms(EffectSpeed::Slow), 100);
        assert_eq!(frame_ms(EffectSpeed::Normal), 66);
        assert_eq!(frame_ms(EffectSpeed::Fast), 40);
    }
}
