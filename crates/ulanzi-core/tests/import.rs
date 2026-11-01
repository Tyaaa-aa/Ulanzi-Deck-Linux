use ulanzi_core::import::{ImportError, import_image};

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("test setup");
    out.into_inner()
}

#[test]
fn import_image_downscales_and_rejects_non_images() {
    let dir = tempfile::tempdir().expect("test setup");
    let profile = dir.path().join("profile");
    std::fs::create_dir_all(&profile).expect("test setup");

    // A photo larger than the decode limit for drawing is still accepted and downscaled.
    let big = dir.path().join("big.png");
    std::fs::write(&big, png(5000, 300)).expect("test setup");
    let rel = import_image(&big, &profile).expect("test setup");
    assert!(rel.starts_with("icons/") && rel.ends_with(".png"), "{rel}");
    let saved = image::open(profile.join(&rel)).expect("test setup");
    assert_eq!((saved.width(), saved.height()), (196, 196));

    // Same content → same file.
    assert_eq!(import_image(&big, &profile).expect("test setup"), rel);

    // SVG is rasterised.
    let svg = dir.path().join("logo.svg");
    std::fs::write(
        &svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#,
    )
    .expect("test setup");
    let rel_svg = import_image(&svg, &profile).expect("test setup");
    let saved = image::open(profile.join(&rel_svg))
        .expect("test setup")
        .to_rgba8();
    assert_eq!(saved.dimensions(), (196, 196));
    assert_eq!(saved.get_pixel(98, 98).0, [255, 0, 0, 255]);

    // Not an image.
    let txt = dir.path().join("notes.png");
    std::fs::write(&txt, b"hello, not a picture").expect("test setup");
    let err = import_image(&txt, &profile).expect_err("should fail");
    assert!(matches!(err, ImportError::NotAnImage));
    assert_eq!(err.to_string(), "That file isn't an image");
}

#[test]
fn screen_pictures_are_screen_sized() {
    let dir = tempfile::tempdir().expect("test setup");
    let profile = dir.path().join("profile");
    std::fs::create_dir_all(&profile).expect("test setup");
    let photo = dir.path().join("photo.png");
    std::fs::write(&photo, png(1600, 1200)).expect("test setup");
    let rel = ulanzi_core::import::import_screen_image(&photo, &profile).expect("import");
    let saved = image::open(profile.join(&rel)).expect("open");
    assert_eq!((saved.width(), saved.height()), (916, 392));
}

fn gif(frames: u32) -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, Rgba, RgbaImage};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).expect("test setup");
        for f in 0..frames {
            let img = RgbaImage::from_pixel(32, 32, Rgba([(f % 5) as u8 * 40, 30, 30, 255]));
            let frame = Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(100, 1));
            enc.encode_frame(frame).expect("test setup");
        }
    }
    out
}

#[test]
fn a_gif_stays_a_gif() {
    let dir = tempfile::tempdir().expect("test setup");
    let src = dir.path().join("moving.gif");
    let bytes = gif(3);
    std::fs::write(&src, &bytes).expect("test setup");
    let rel = import_image(&src, dir.path()).expect("imports");
    assert!(rel.starts_with("icons/") && rel.ends_with(".gif"), "{rel}");
    assert_eq!(std::fs::read(dir.path().join(&rel)).expect("copied"), bytes);
}

#[test]
fn a_gif_is_told_by_its_content_not_its_name() {
    let dir = tempfile::tempdir().expect("test setup");
    let src = dir.path().join("really.png");
    std::fs::write(&src, gif(2)).expect("test setup");
    assert!(import_image(&src, dir.path()).expect("imports").ends_with(".gif"));
    let fake = dir.path().join("fake.gif");
    std::fs::write(&fake, png(8, 8)).expect("test setup");
    assert!(
        import_image(&fake, dir.path())
            .expect("imports")
            .ends_with(".png")
    );
}

#[test]
fn a_broken_gif_is_refused() {
    let dir = tempfile::tempdir().expect("test setup");
    let src = dir.path().join("cut.gif");
    let whole = gif(2);
    std::fs::write(&src, &whole[..20]).expect("test setup");
    assert!(import_image(&src, dir.path()).is_err());
}

#[test]
fn a_gif_too_big_to_play_is_refused() {
    let dir = tempfile::tempdir().expect("test setup");
    let src = dir.path().join("huge.gif");
    let mut bytes = gif(2);
    bytes.resize(16 * 1024 * 1024 + 1, 0);
    std::fs::write(&src, &bytes).expect("test setup");
    assert!(matches!(
        import_image(&src, dir.path()),
        Err(ImportError::GifTooBig)
    ));
}

#[test]
fn a_gif_with_a_huge_canvas_is_refused() {
    let dir = tempfile::tempdir().expect("test setup");
    let src = dir.path().join("wide.gif");
    let mut bytes = gif(2);
    bytes[6..10].fill(0xff);
    std::fs::write(&src, &bytes).expect("test setup");
    assert!(matches!(
        import_image(&src, dir.path()),
        Err(ImportError::TooLarge)
    ));
}

#[test]
fn a_gif_with_too_many_frames_is_refused() {
    let dir = tempfile::tempdir().expect("test setup");
    let src = dir.path().join("long.gif");
    std::fs::write(&src, gif(501)).expect("test setup");
    let err = import_image(&src, dir.path()).expect_err("refused");
    assert!(matches!(err, ImportError::GifTooBig));
    assert!(err.to_string().contains("500 frames"));
    std::fs::write(&src, gif(500)).expect("test setup");
    assert!(import_image(&src, dir.path()).is_ok());
}

#[test]
fn an_icons_colour_is_its_colourful_part() {
    use image::{Rgba, RgbaImage};
    use ulanzi_core::apps::dominant_color;
    let img = RgbaImage::from_fn(10, 10, |x, _| match x {
        0..=4 => Rgba([255, 255, 255, 255]),
        5..=7 => Rgba([200, 20, 20, 255]),
        _ => Rgba([0, 0, 255, 0]),
    });
    assert_eq!(dominant_color(&img), Some(ulanzi_core::Color::rgb(200, 20, 20)));
    let grey = RgbaImage::from_pixel(4, 4, Rgba([128, 128, 128, 255]));
    assert_eq!(dominant_color(&grey), None);
}
