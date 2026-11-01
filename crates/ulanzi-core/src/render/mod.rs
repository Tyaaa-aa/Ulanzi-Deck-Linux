//! Rendering of key images and the wide screen.

mod animation;
mod effects;
mod fonts;
mod icons;
mod key;
mod nowplaying;
pub mod screen;
mod text;

pub use animation::{Animation, Encoded, animation_hash, encode_fit, is_animated, render_animation};
pub use effects::{GIF_TARGET, LOOP_FRAMES, draw_effect, effect_color, frame_ms};
pub use fonts::{Fonts, TextStyle};
pub use icons::IconLibrary;
pub use key::{
    KEY_PX, RenderCtx, Rendered, icon_file_png, key_image, look_hash, plugin_icon_png, render_key,
    render_key_progress,
};
pub use nowplaying::{NowPlaying, render_now_playing};
