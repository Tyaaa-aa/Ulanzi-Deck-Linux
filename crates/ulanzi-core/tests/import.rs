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
