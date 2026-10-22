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
fn small_window_has_seven_fields() {
    let w = SmallWindow {
        mode: ScreenMode::Image,
        cpu: 12,
        mem: 34,
        time: "12:34:56".into(),
        gpu: 0,
        clock_24h: true,
        date: String::new(),
    };
    let p = small_window(&w);
    let payload = b"2|12|34|12:34:56|0|24H|";
    assert_eq!(&p[..8], &[0x7c, 0x7c, 0x00, 0x06, payload.len() as u8, 0, 0, 0]);
    assert_eq!(&p[8..8 + payload.len()], payload);
}

#[test]
fn screen_mode_ids_match_ulanzi_studio() {
    assert_eq!(ScreenMode::Stats as u8, 0);
    assert_eq!(ScreenMode::Clock as u8, 1);
    assert_eq!(ScreenMode::Image as u8, 2);
    assert_eq!(ScreenMode::ClockDateWeekday as u8, 200);
    assert_eq!(ScreenMode::ClockWeekday as u8, 201);
    assert_eq!(ScreenMode::ClockDate as u8, 202);
    assert_eq!(ScreenMode::ClockLarge as u8, 203);
}

#[test]
fn small_window_twelve_hour_and_date() {
    let w = SmallWindow {
        mode: ScreenMode::ClockWeekday,
        cpu: 10,
        mem: 41,
        time: "15:10:36".into(),
        gpu: 2,
        clock_24h: false,
        date: "Fri".into(),
    };
    assert!(small_window(&w)[8..].starts_with(b"201|10|41|15:10:36|2|12H|Fri"));
}

#[test]
fn control_commands() {
    assert_eq!(&unlock()[..8], &[0x7c, 0x7c, 0x00, 0x05, 0, 0, 0, 0]);
    assert_eq!(&request_device_info()[..8], &[0x7c, 0x7c, 0x00, 0x03, 0, 0, 0, 0]);
    let s = sleep_time(15);
    assert_eq!(&s[..10], &[0x7c, 0x7c, 0x00, 0x1a, 2, 0, 0, 0, b'1', b'5']);
}

#[test]
fn report_id_prefix() {
    let p = brightness(5);
    let r = with_report_id(&p);
    assert_eq!(r.len(), 1025);
    assert_eq!(r[0], 0);
    assert_eq!(&r[1..], &p[..]);
}

#[test]
fn header_layout() {
    let p = packet(Command::LabelStyle, b"abc", 3);
    assert_eq!(&p[..8], &[0x7c, 0x7c, 0x00, 0x0b, 3, 0, 0, 0]);
    assert_eq!(&p[8..11], b"abc");
    assert!(p[11..].iter().all(|&b| b == 0));
}
