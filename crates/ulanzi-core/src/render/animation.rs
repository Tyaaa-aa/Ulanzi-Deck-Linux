//! Keys that move: the frames of a GIF or an effect, and the GIF file the deck plays.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::gif::{GifDecoder, GifEncoder, Repeat};
use image::imageops::FilterType;
use image::{AnimationDecoder, Delay, DynamicImage, Frame, RgbaImage};

use super::effects::{LOOP_FRAMES, draw_effect, frame_ms};
use super::key::{KEY_PX, RenderCtx, key_image, look_hash};
use crate::Color;
use crate::model::{Effect, ImageSource, KeyLook};

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SOURCE_FRAMES: usize = 500;
/// A frame time under this means "as fast as you like", which old GIFs ask for with 0.
const MIN_DELAY_MS: u32 = 20;
const NO_DELAY_MS: u32 = 100;
const SHADES: [u32; 3] = [16, 8, 4];
/// Part of every cached file's name. Raise it when effects are drawn differently.
const DRAW_VERSION: u64 = 1;

/// Frames and how long each shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub frames: Vec<RgbaImage>,
    pub delays_ms: Vec<u32>,
}

/// A GIF ready for the deck. `lost_frames` says it had to drop frames to fit.
#[derive(Debug, Clone, PartialEq)]
pub struct Encoded {
    pub gif: Vec<u8>,
    pub lost_frames: bool,
}

fn gif_source(look: &KeyLook, profile_dir: &Path) -> Option<PathBuf> {
    match &look.source {
        ImageSource::Image { path } if path.to_lowercase().ends_with(".gif") => Some(profile_dir.join(path)),
        _ => None,
    }
}

fn decoder(path: &Path) -> Result<GifDecoder<Cursor<Vec<u8>>>, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_SOURCE_BYTES {
        return Err("a GIF can be up to 16 MB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())
}

fn read_gif(path: &Path) -> Result<Vec<(RgbaImage, u32)>, String> {
    let mut out = Vec::new();
    for frame in decoder(path)?.into_frames() {
        if out.len() == MAX_SOURCE_FRAMES {
            return Err(format!("a GIF can have up to {MAX_SOURCE_FRAMES} frames"));
        }
        let frame = frame.map_err(|e| e.to_string())?;
        let (n, d) = frame.delay().numer_denom_ms();
        let ms = n.checked_div(d).unwrap_or(0);
        let picture = DynamicImage::ImageRgba8(frame.into_buffer())
            .resize_to_fill(KEY_PX, KEY_PX, FilterType::Triangle)
            .to_rgba8();
        out.push((picture, if ms < MIN_DELAY_MS { NO_DELAY_MS } else { ms }));
    }
    if out.is_empty() {
        return Err("the GIF has no frames".into());
    }
    Ok(out)
}

/// Whether a key shows as a GIF: it has an effect, or its picture is a GIF that moves.
pub fn is_animated(look: &KeyLook, effect: &Effect, profile_dir: &Path) -> bool {
    !effect.is_none()
        || gif_source(look, profile_dir)
            .and_then(|path| decoder(&path).ok())
            .is_some_and(|d| d.into_frames().take(2).filter(Result::is_ok).count() == 2)
}

/// The frames of a key with `effect` drawn in `color`. A GIF keeps its own frames and timing, and
/// the effect's loop fits itself to them.
pub fn render_animation(
    look: &KeyLook,
    effect: &Effect,
    color: Option<Color>,
    ctx: &RenderCtx,
) -> Result<Animation, String> {
    let gif = match gif_source(look, ctx.profile_dir) {
        Some(path) => read_gif(&path)?,
        None => Vec::new(),
    };
    if gif.len() < 2 {
        let base = key_image(look, ctx, gif.first().map(|f| &f.0));
        let frames = (0..LOOP_FRAMES)
            .map(|f| draw_effect(&base, effect, color, f as f32 / LOOP_FRAMES as f32))
            .collect();
        return Ok(Animation {
            frames,
            delays_ms: vec![frame_ms(effect.speed); LOOP_FRAMES as usize],
        });
    }
    let total: u32 = gif.iter().map(|f| f.1).sum();
    let (mut frames, mut delays_ms, mut elapsed) = (Vec::new(), Vec::new(), 0);
    for (picture, ms) in &gif {
        let base = key_image(look, ctx, Some(picture));
        frames.push(draw_effect(&base, effect, color, elapsed as f32 / total as f32));
        delays_ms.push(*ms);
        elapsed += ms;
    }
    Ok(Animation { frames, delays_ms })
}

/// One GIF of `a` with `shades` per colour channel, `None` for all of them.
fn encode(a: &Animation, shades: Option<u32>) -> Vec<u8> {
    let round = |v: u8| match shades {
        Some(n) => {
            let n = (n - 1) as f32;
            ((v as f32 / 255.0 * n).round() / n * 255.0) as u8
        }
        None => v,
    };
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, 10);
        let looped = enc.set_repeat(Repeat::Infinite);
        debug_assert!(looped.is_ok());
        for (img, ms) in a.frames.iter().zip(&a.delays_ms) {
            let mut img = img.clone();
            for p in img.pixels_mut() {
                let [r, g, b, _] = p.0;
                *p = image::Rgba([round(r), round(g), round(b), 255]);
            }
            let frame = Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(*ms, 1));
            if enc.encode_frame(frame).is_err() {
                break;
            }
        }
    }
    out
}

/// Every second frame gone, each kept frame showing for the time of both.
fn halved(a: &Animation) -> Animation {
    let delays_ms = a.delays_ms.chunks(2).map(|c| c.iter().sum()).collect();
    let frames = a.frames.iter().step_by(2).cloned().collect();
    Animation { frames, delays_ms }
}

/// A GIF of `a` of at most `limit` bytes: fewer colours first, then fewer frames.
pub fn encode_fit(a: &Animation, limit: usize) -> Encoded {
    let gif = encode(a, None);
    if gif.len() <= limit {
        return Encoded {
            gif,
            lost_frames: false,
        };
    }
    for shades in SHADES {
        let gif = encode(a, Some(shades));
        if gif.len() <= limit {
            return Encoded {
                gif,
                lost_frames: false,
            };
        }
    }
    let mut a = a.clone();
    loop {
        a = halved(&a);
        let gif = encode(&a, SHADES.last().copied());
        if gif.len() <= limit || a.frames.len() < 2 {
            return Encoded {
                gif,
                lost_frames: true,
            };
        }
    }
}

/// Names a finished GIF: unchanged while nothing that draws it changes.
pub fn animation_hash(look: &KeyLook, effect: &Effect, color: Option<Color>, profile_dir: &Path) -> u64 {
    let mut h = DefaultHasher::new();
    look_hash(look, profile_dir).hash(&mut h);
    effect.hash(&mut h);
    color.hash(&mut h);
    DRAW_VERSION.hash(&mut h);
    h.finish()
}
