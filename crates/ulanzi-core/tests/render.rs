use std::io::Write;
use std::path::Path;
use std::time::Instant;

use ulanzi_core::Color;
use ulanzi_core::model::{IconSet, ImageSource, KeyLook};
use ulanzi_core::render::{IconLibrary, KEY_PX, RenderCtx, look_hash, render_key};

fn decode(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory(png).expect("decodes").to_rgba8()
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b.iter()).take(3).all(|(x, y)| x.abs_diff(*y) <= 8)
}

fn icon_look(name: &str, scale: f32) -> KeyLook {
    KeyLook {
        source: ImageSource::Icon {
            set: IconSet::Lucide,
            name: name.into(),
        },
        background: Color::rgb(0, 0, 0),
        foreground: "#3584e4".parse().expect("color"),
        icon_scale: scale,
        ..KeyLook::default()
    }
}

fn ctx(dir: &Path) -> RenderCtx<'_> {
    RenderCtx {
        profile_dir: dir,
        icons: IconLibrary::bundled(),
    }
}

#[test]
fn library_has_lucide_icons() {
    let lib = IconLibrary::bundled();
    assert!(lib.svg(IconSet::Lucide, "play").is_some());
    assert!(lib.names(IconSet::Lucide).count() > 1500);
    assert!(lib.svg(IconSet::Lucide, "no-such-icon").is_none());
}

#[test]
fn icon_renders_196_with_bg_and_fg() {
    let d = tempfile::tempdir().expect("tmp");
    let r = render_key(&icon_look("play", 0.7), &ctx(d.path())).expect("drawn");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let img = decode(&r.png);
    assert_eq!(img.dimensions(), (KEY_PX, KEY_PX));
    assert!(near(img.get_pixel(2, 2).0, [0, 0, 0, 255]));
    let fg = [0x35, 0x84, 0xe4, 255];
    let inside = (30..166)
        .flat_map(|x| (30..166).map(move |y| (x, y)))
        .any(|(x, y)| near(img.get_pixel(x, y).0, fg));
    assert!(inside, "no foreground pixel inside the icon box");
}

#[test]
fn icon_scale_respected() {
    let d = tempfile::tempdir().expect("tmp");
    let img = decode(
        &render_key(&icon_look("play", 0.5), &ctx(d.path()))
            .expect("drawn")
            .png,
    );
    for x in 0..KEY_PX {
        for y in 0..KEY_PX {
            let outside = !(48..148).contains(&x) || !(48..148).contains(&y);
            if outside {
                assert!(
                    near(img.get_pixel(x, y).0, [0, 0, 0, 255]),
                    "pixel {x},{y} outside the icon box"
                );
            }
        }
    }
}

#[test]
fn image_cover_fit() {
    let d = tempfile::tempdir().expect("tmp");
    let mut src = image::RgbaImage::new(400, 200);
    for (x, _, p) in src.enumerate_pixels_mut() {
        *p = if x < 200 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        };
    }
    src.save(d.path().join("split.png")).expect("save");
    let look = KeyLook {
        source: ImageSource::Image {
            path: "split.png".into(),
        },
        ..KeyLook::default()
    };
    let img = decode(&render_key(&look, &ctx(d.path())).expect("drawn").png);
    assert_eq!(img.dimensions(), (KEY_PX, KEY_PX));
    // Cover fit crops the sides: top-left is red, top-right is blue, no black letterbox rows.
    assert!(near(img.get_pixel(5, 5).0, [255, 0, 0, 255]));
    assert!(near(img.get_pixel(190, 5).0, [0, 0, 255, 255]));
    assert!(!near(img.get_pixel(5, 195).0, [0, 0, 0, 255]));
}

#[test]
fn missing_image_falls_back() {
    let d = tempfile::tempdir().expect("tmp");
    let look = KeyLook {
        source: ImageSource::Image {
            path: "icons/gone.png".into(),
        },
        background: Color::rgb(10, 20, 30),
        ..KeyLook::default()
    };
    let r = render_key(&look, &ctx(d.path())).expect("drawn");
    assert_eq!(r.warnings.len(), 1);
    assert!(near(decode(&r.png).get_pixel(98, 98).0, [10, 20, 30, 255]));
}

#[test]
fn huge_image_is_limited() {
    let d = tempfile::tempdir().expect("tmp");
    // A PNG header claiming 8000x8000; the decoder must refuse before allocating.
    let mut f = std::fs::File::create(d.path().join("huge.png")).expect("create");
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend_from_slice(&8000u32.to_be_bytes());
    ihdr.extend_from_slice(&8000u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    f.write_all(b"\x89PNG\r\n\x1a\n").expect("w");
    f.write_all(&13u32.to_be_bytes()).expect("w");
    f.write_all(&ihdr).expect("w");
    f.write_all(&crc32(&ihdr).to_be_bytes()).expect("w");
    drop(f);
    let look = KeyLook {
        source: ImageSource::Image {
            path: "huge.png".into(),
        },
        ..KeyLook::default()
    };
    let t = Instant::now();
    let r = render_key(&look, &ctx(d.path())).expect("drawn");
    assert!(t.elapsed().as_secs() < 2);
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
}

#[test]
fn unknown_icon_warns() {
    let d = tempfile::tempdir().expect("tmp");
    let r = render_key(&icon_look("chrome", 0.7), &ctx(d.path())).expect("drawn");
    assert!(r.warnings.iter().any(|w| w.contains("chrome")));
}

#[test]
fn empty_look_is_none() {
    let d = tempfile::tempdir().expect("tmp");
    assert!(render_key(&KeyLook::default(), &ctx(d.path())).is_none());
    let coloured = KeyLook {
        background: Color::rgb(1, 2, 3),
        ..KeyLook::default()
    };
    assert!(render_key(&coloured, &ctx(d.path())).is_some());
}

#[test]
fn hash_changes_with_look() {
    let d = tempfile::tempdir().expect("tmp");
    let a = icon_look("play", 0.7);
    let mut b = a.clone();
    assert_eq!(look_hash(&a, d.path()), look_hash(&b, d.path()));
    b.foreground = Color::rgb(1, 1, 1);
    assert_ne!(look_hash(&a, d.path()), look_hash(&b, d.path()));
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

#[test]
fn small_icon_for_pickers() {
    use ulanzi_core::model::IconSet;
    let lib = ulanzi_core::render::IconLibrary::bundled();
    let red = ulanzi_core::Color::rgb(255, 0, 0);
    for (set, name) in [(IconSet::Lucide, "play"), (IconSet::Brands, "steam")] {
        let png = lib.icon_png(set, name, 48, red).expect(name);
        let img = image::load_from_memory(&png).expect("png").to_rgba8();
        assert_eq!(img.dimensions(), (48, 48));
        assert!(
            img.pixels().any(|p| p.0 == [255, 0, 0, 255]),
            "{name} has no solid red pixel"
        );
    }
    assert!(lib.icon_png(IconSet::Lucide, "no-such-icon", 48, red).is_none());
}

#[test]
fn label_with_own_position_is_drawn_into_the_key() {
    use ulanzi_core::model::{Label, LabelPosition};
    let dir = tempfile::tempdir().expect("dir");
    let rows_with_text = |pos| {
        let look = KeyLook {
            label: Label {
                text: "HELLO".into(),
                visible: true,
                position: Some(pos),
            },
            ..KeyLook::default()
        };
        let img = decode(&render_key(&look, &ctx(dir.path())).expect("drawn").png);
        let lit = |y0: u32, y1: u32| (y0..y1).any(|y| (0..KEY_PX).any(|x| img.get_pixel(x, y).0[0] > 200));
        (lit(0, 60), lit(68, 128), lit(136, KEY_PX))
    };
    assert_eq!(rows_with_text(LabelPosition::Top), (true, false, false));
    assert_eq!(rows_with_text(LabelPosition::Middle), (false, true, false));
    assert_eq!(rows_with_text(LabelPosition::Bottom), (false, false, true));
}

#[test]
fn the_progress_bar_grows_with_the_share() {
    let d = tempfile::tempdir().expect("tmp");
    let look = KeyLook {
        foreground: Color::rgb(255, 255, 255),
        background: Color::BLACK,
        source: ImageSource::None,
        ..KeyLook::default()
    };
    let width = |share: f32| {
        let png = ulanzi_core::render::render_key_progress(&look, &ctx(d.path()), share)
            .expect("drawn")
            .png;
        let img = decode(&png);
        (0..KEY_PX)
            .filter(|x| near(img.get_pixel(*x, KEY_PX - 3).0, [255, 255, 255, 255]))
            .count() as u32
    };
    assert_eq!(width(0.0), 0);
    assert!(width(0.5).abs_diff(KEY_PX / 2) <= 1);
    assert_eq!(width(1.0), KEY_PX);
}

mod animation {
    use image::codecs::gif::GifDecoder;
    use image::{AnimationDecoder, Rgba, RgbaImage};
    use std::io::Cursor;
    use ulanzi_core::model::{Effect, EffectKind, EffectSpeed};
    use ulanzi_core::render::{
        Animation, GIF_TARGET, LOOP_FRAMES, animation_hash, encode_fit, is_animated, render_animation,
    };

    use super::*;

    const BLUE: Option<Color> = Some(Color([80, 180, 255, 255]));

    /// `frames` of coloured 4-pixel squares at 66 ms: heavy, like noise, and different every time.
    fn noisy(frames: u32) -> Animation {
        let mut seed = 0x2545_f491u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let frames: Vec<RgbaImage> = (0..frames)
            .map(|_| {
                let cells: Vec<u32> = (0..50 * 50).map(|_| next()).collect();
                RgbaImage::from_fn(196, 196, |x, y| {
                    let v = cells[(y / 4 * 50 + x / 4) as usize].to_le_bytes();
                    Rgba([v[0], v[1], v[2], 255])
                })
            })
            .collect();
        let delays_ms = vec![66; frames.len()];
        Animation { frames, delays_ms }
    }

    fn decoded(gif: &[u8]) -> Vec<image::Frame> {
        let d = GifDecoder::new(Cursor::new(gif)).expect("a gif");
        d.into_frames().collect_frames().expect("frames")
    }

    fn total_ms(gif: &[u8]) -> u32 {
        decoded(gif)
            .iter()
            .map(|f| {
                let (n, d) = f.delay().numer_denom_ms();
                n / d
            })
            .sum()
    }

    fn small_gif(path: &Path, frames: u32, ms: u32) {
        let img = RgbaImage::from_pixel(32, 32, Rgba([200, 30, 30, 255]));
        let a = Animation {
            frames: (0..frames).map(|_| img.clone()).collect(),
            delays_ms: vec![ms; frames as usize],
        };
        std::fs::write(path, encode_fit(&a, GIF_TARGET).gif).expect("write");
    }

    fn gif_look() -> KeyLook {
        KeyLook {
            source: ImageSource::Image { path: "a.gif".into() },
            ..KeyLook::default()
        }
    }

    #[test]
    fn a_heavy_gif_loses_colours_before_frames() {
        let limit = encode_fit(&noisy(24), usize::MAX).gif.len() * 9 / 10;
        let e = encode_fit(&noisy(24), limit);
        assert!(e.gif.len() <= limit, "{} bytes", e.gif.len());
        assert!(!e.lost_frames);
        assert_eq!(decoded(&e.gif).len(), 24);
    }

    #[test]
    fn frames_go_only_when_colours_are_not_enough() {
        let e = encode_fit(&noisy(96), GIF_TARGET);
        assert!(e.gif.len() <= GIF_TARGET, "{} bytes", e.gif.len());
        assert!(e.lost_frames);
        // GIF times are in hundredths, so only close.
        assert!(
            total_ms(&e.gif).abs_diff(96 * 66) < 200,
            "the loop keeps its length"
        );
    }

    #[test]
    fn a_still_key_with_an_effect_is_a_full_loop() {
        let d = tempfile::tempdir().expect("dir");
        let e = Effect {
            kind: EffectKind::Pulse,
            color: BLUE,
            speed: EffectSpeed::Fast,
        };
        let a = render_animation(&icon_look("play", 0.7), &e, BLUE, &ctx(d.path())).expect("drawn");
        assert_eq!(a.frames.len(), LOOP_FRAMES as usize);
        assert_eq!(a.delays_ms, vec![40; 24]);
        assert_ne!(a.frames[0], a.frames[6], "it moves");
    }

    #[test]
    fn a_gif_keeps_its_own_frames_and_timing_under_an_effect() {
        let d = tempfile::tempdir().expect("dir");
        small_gif(&d.path().join("a.gif"), 5, 120);
        let e = Effect::of(EffectKind::Chase);
        let a = render_animation(&gif_look(), &e, BLUE, &ctx(d.path())).expect("drawn");
        assert_eq!(a.delays_ms, vec![120; 5]);
        assert_eq!(a.frames[0].dimensions(), (KEY_PX, KEY_PX));
    }

    #[test]
    fn a_gif_with_no_frame_time_plays_at_100_ms() {
        let d = tempfile::tempdir().expect("dir");
        small_gif(&d.path().join("a.gif"), 3, 0);
        let a = render_animation(&gif_look(), &Effect::default(), None, &ctx(d.path())).expect("drawn");
        assert_eq!(a.delays_ms, vec![100; 3]);
    }

    #[test]
    fn only_a_gif_that_moves_or_an_effect_is_animated() {
        let d = tempfile::tempdir().expect("dir");
        let none = Effect::default();
        small_gif(&d.path().join("a.gif"), 1, 100);
        assert!(!is_animated(&gif_look(), &none, d.path()));
        small_gif(&d.path().join("a.gif"), 2, 100);
        assert!(is_animated(&gif_look(), &none, d.path()));
        assert!(!is_animated(&icon_look("play", 0.7), &none, d.path()));
        assert!(is_animated(
            &icon_look("play", 0.7),
            &Effect::of(EffectKind::Glow),
            d.path()
        ));
    }

    #[test]
    fn a_broken_gif_is_an_error() {
        let d = tempfile::tempdir().expect("dir");
        let e = Effect::of(EffectKind::Glow);
        assert!(
            render_animation(&gif_look(), &e, BLUE, &ctx(d.path())).is_err(),
            "missing"
        );
        std::fs::write(d.path().join("a.gif"), b"not a gif").expect("write");
        assert!(
            render_animation(&gif_look(), &e, BLUE, &ctx(d.path())).is_err(),
            "junk"
        );
        assert!(!is_animated(&gif_look(), &Effect::default(), d.path()));
        small_gif(&d.path().join("a.gif"), 3, 100);
        let whole = std::fs::read(d.path().join("a.gif")).expect("read");
        std::fs::write(d.path().join("a.gif"), &whole[..whole.len() / 2]).expect("write");
        assert!(
            render_animation(&gif_look(), &e, BLUE, &ctx(d.path())).is_err(),
            "cut short"
        );
    }

    #[test]
    fn a_gif_over_the_limits_is_refused() {
        let d = tempfile::tempdir().expect("dir");
        small_gif(&d.path().join("a.gif"), 501, 100);
        let e = render_animation(&gif_look(), &Effect::default(), None, &ctx(d.path()));
        assert!(e.is_err_and(|m| m.contains("500")));
    }

    #[test]
    fn the_hash_follows_the_file_the_colour_and_the_effect() {
        let d = tempfile::tempdir().expect("dir");
        small_gif(&d.path().join("a.gif"), 2, 100);
        let e = Effect::of(EffectKind::Glow);
        let first = animation_hash(&gif_look(), &e, BLUE, d.path());
        assert_eq!(first, animation_hash(&gif_look(), &e, BLUE, d.path()));
        assert_ne!(
            first,
            animation_hash(&gif_look(), &e, Some(Color::WHITE), d.path())
        );
        assert_ne!(
            first,
            animation_hash(&gif_look(), &Effect::of(EffectKind::Fire), BLUE, d.path())
        );
        small_gif(&d.path().join("a.gif"), 3, 100);
        assert_ne!(first, animation_hash(&gif_look(), &e, BLUE, d.path()));
    }
}
