use ulanzi_proto::*;

fn report(cmd: u16, data: [u8; 4]) -> Vec<u8> {
    let mut v = vec![0x7c, 0x7c];
    v.extend_from_slice(&cmd.to_be_bytes());
    v.extend_from_slice(&[0, 0, 0, 0]);
    v.extend_from_slice(&data);
    v.resize(1024, 0);
    v
}

fn parsed(cmd: u16, data: [u8; 4]) -> Option<(Control, InputKind)> {
    parse_input(&report(cmd, data)).map(|e| (e.control, e.kind))
}

#[test]
fn keys_press_and_release() {
    assert_eq!(
        parsed(0x0101, [0, 7, 0, 1]),
        Some((Control::Key(7), InputKind::Press))
    );
    assert_eq!(
        parsed(0x0101, [0, 7, 0, 0]),
        Some((Control::Key(7), InputKind::Release))
    );
    assert_eq!(
        parsed(0x0101, [0, 0, 0, 1]),
        Some((Control::Key(0), InputKind::Press))
    );
    assert_eq!(
        parsed(0x0101, [0, 12, 0, 1]),
        Some((Control::Key(12), InputKind::Press))
    );
}

#[test]
fn screen_and_small_buttons() {
    assert_eq!(
        parsed(0x0102, [0, 13, 0, 0]),
        Some((Control::Screen, InputKind::Release))
    );
    assert_eq!(
        parsed(0x0101, [0, 15, 0, 1]),
        Some((Control::Small(0), InputKind::Press))
    );
    assert_eq!(
        parsed(0x0101, [0, 16, 0, 0]),
        Some((Control::Small(1), InputKind::Release))
    );
}

#[test]
fn dials() {
    assert_eq!(
        parsed(0x0101, [0, 17, 2, 1]),
        Some((Control::Dial(0), InputKind::Press))
    );
    assert_eq!(
        parsed(0x0101, [0, 18, 2, 0]),
        Some((Control::Dial(1), InputKind::Release))
    );
    assert_eq!(
        parsed(0x0101, [0, 19, 2, 2]),
        Some((Control::Dial(2), InputKind::TurnLeft))
    );
    assert_eq!(
        parsed(0x0101, [0, 17, 2, 3]),
        Some((Control::Dial(0), InputKind::TurnRight))
    );
    assert_eq!(parsed(0x0101, [0, 17, 2, 9]), None);
}

#[test]
fn rejects_other_reports() {
    assert_eq!(parsed(0x0303, [0, 7, 0, 1]), None);
    let mut bad = report(0x0101, [0, 7, 0, 1]);
    bad[0] = 0;
    assert_eq!(parse_input(&bad), None);
    assert_eq!(parse_input(&report(0x0101, [0, 7, 0, 1])[..11]), None);
}

#[test]
fn unknown_index_is_reported() {
    assert_eq!(
        parsed(0x0101, [0, 40, 0, 1]),
        Some((Control::Unknown(40), InputKind::Press))
    );
    let e = parse_input(&report(0x0101, [3, 40, 0, 1])).expect("event");
    assert_eq!((e.raw_index, e.raw_state), (40, 3));
}

#[test]
fn control_names() {
    assert_eq!(Control::Key(7).as_str(), "key:7");
    assert_eq!(Control::Screen.as_str(), "screen");
    assert_eq!(Control::Small(0).as_str(), "small:0");
    assert_eq!(Control::Dial(2).as_str(), "dial:2");
    assert_eq!(Control::Unknown(40).as_str(), "unknown:40");
}

fn frame(cc: u8, pid: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0x7c, 0x7c, cc, pid];
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v.resize(1024, 0);
    v
}

#[test]
fn device_reports() {
    let info = br#"{ "SerialNumber": "02C51A015U3672946", "Dversion": "5.1.7" }"#;
    assert_eq!(
        parse_report(&frame(3, 3, info)),
        DeviceReport::DeviceInfo(String::from_utf8_lossy(info).into_owned())
    );
    assert_eq!(
        parse_report(&frame(1, 0x0b, b"")),
        DeviceReport::LabelStyleRequest
    );
    assert!(
        matches!(parse_report(&frame(1, 1, &[0, 7, 1, 1])), DeviceReport::Input(e) if e.control == Control::Key(7))
    );
    assert_eq!(parse_report(&frame(9, 9, b"")), DeviceReport::Other(0x0909));
    assert_eq!(parse_report(&[1, 2]), DeviceReport::Other(0));
}
