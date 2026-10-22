use ulanzi_core::render::{Fonts, NowPlaying, render_now_playing};

const BG: [u8; 3] = [0x13, 0x1c, 0x2e];

fn differs(p: &image::Rgba<u8>) -> bool {
    p.0.iter().zip(BG.iter()).any(|(a, b)| a.abs_diff(*b) > 12)
}

fn info() -> NowPlaying {
    NowPlaying {
        title: "Midnight City and a very long second line".into(),
        artist: "M83".into(),
    }
}

#[test]
fn nowplaying_renders_size_and_text_area() {
    let mut fonts = Fonts::system();
    let png = render_now_playing(&info(), None, &mut fonts, (196, 196));
    let img = image::load_from_memory(&png).expect("png").to_rgba8();
    assert_eq!(img.dimensions(), (196, 196));
    let right_half_text = (100..196)
        .flat_map(|x| (0..196).map(move |y| (x, y)))
        .any(|(x, y)| differs(img.get_pixel(x, y)));
    assert!(right_half_text, "no text drawn on the right half");
}

#[test]
fn nowplaying_other_sizes() {
    let mut fonts = Fonts::system();
    let png = render_now_playing(&info(), None, &mut fonts, (392, 196));
    assert_eq!(
        image::load_from_memory(&png)
            .expect("png")
            .to_rgba8()
            .dimensions(),
        (392, 196)
    );
}

#[test]
fn nowplaying_without_art_draws_placeholder() {
    let mut fonts = Fonts::system();
    let png = render_now_playing(&info(), None, &mut fonts, (392, 196));
    let img = image::load_from_memory(&png).expect("png").to_rgba8();
    assert!(differs(img.get_pixel(98 - 50, 98)), "disc placeholder missing");
}

#[test]
fn nowplaying_uses_art() {
    let d = tempfile::tempdir().expect("tmp");
    let art = d.path().join("art.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([250, 10, 10, 255]))
        .save(&art)
        .expect("save");
    let mut fonts = Fonts::system();
    let png = render_now_playing(&info(), Some(&art), &mut fonts, (392, 196));
    let img = image::load_from_memory(&png).expect("png").to_rgba8();
    let p = img.get_pixel(60, 60).0;
    assert!(p[0] > 200 && p[1] < 60, "{p:?}");
}
