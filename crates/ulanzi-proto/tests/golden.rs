use ulanzi_proto::*;

#[test]
fn brightness_matches_python() {
    assert_eq!(brightness(37), *include_bytes!("fixtures/brightness_37.bin"));
}

#[test]
fn brightness_clamps() {
    assert_eq!(&brightness(250)[8..11], b"100");
    assert_eq!(&brightness(250)[4..8], &3u32.to_le_bytes());
}

#[test]
fn label_default_matches_python() {
    assert_eq!(
        label_style(&LabelStyle::default()),
        *include_bytes!("fixtures/label_default.bin")
    );
}

#[test]
fn label_hidden_matches_python() {
    let s = LabelStyle {
        show_title: false,
        ..Default::default()
    };
    assert_eq!(label_style(&s), *include_bytes!("fixtures/label_hidden.bin"));
}

#[test]
fn small_window_matches_python() {
    let w = SmallWindow {
        mode: ScreenMode::Background,
        cpu: 12,
        mem: 34,
        time: "12:34:56".into(),
        gpu: 0,
    };
    assert_eq!(small_window(&w), *include_bytes!("fixtures/small_window.bin"));
}

#[test]
fn header_layout() {
    let p = packet(Command::LabelStyle, b"abc", 3);
    assert_eq!(&p[..8], &[0x7c, 0x7c, 0x00, 0x0b, 3, 0, 0, 0]);
    assert_eq!(&p[8..11], b"abc");
    assert!(p[11..].iter().all(|&b| b == 0));
}
