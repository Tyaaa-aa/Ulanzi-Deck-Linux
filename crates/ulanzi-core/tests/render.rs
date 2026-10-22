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
