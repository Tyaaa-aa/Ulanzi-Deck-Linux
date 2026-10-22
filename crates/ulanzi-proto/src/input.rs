/// Input index of the left small button next to the dials (provisional until confirmed on hardware).
pub const SMALL_LEFT_INDEX: u8 = 15;
/// Input index of the right small button next to the dials (provisional until confirmed on hardware).
pub const SMALL_RIGHT_INDEX: u8 = 16;

const IN_BUTTON: u16 = 0x0101;
const IN_BUTTON_2: u16 = 0x0102;
const DIAL_FIRST: u8 = 17;
const DIAL_LAST: u8 = 19;

/// A physical control on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control {
    /// Screen key 0..=12.
    Key(u8),
    /// The wide screen (index 13).
    Screen,
    /// Small button next to the dials: 0 = left, 1 = right.
    Small(u8),
    /// Dial 0..=2, left to right.
    Dial(u8),
    /// Anything else the device reports.
    Unknown(u8),
}

impl Control {
    /// Stable name used on D-Bus and in logs, e.g. `key:7`.
    pub fn as_str(&self) -> String {
        match self {
            Control::Key(i) => format!("key:{i}"),
            Control::Screen => "screen".into(),
            Control::Small(i) => format!("small:{i}"),
            Control::Dial(i) => format!("dial:{i}"),
            Control::Unknown(i) => format!("unknown:{i}"),
        }
    }

    fn from_index(index: u8) -> Control {
        match index {
            0..=12 => Control::Key(index),
            13 => Control::Screen,
            SMALL_LEFT_INDEX => Control::Small(0),
            SMALL_RIGHT_INDEX => Control::Small(1),
            DIAL_FIRST..=DIAL_LAST => Control::Dial(index - DIAL_FIRST),
            other => Control::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputKind {
    Press,
    Release,
    TurnLeft,
    TurnRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    pub control: Control,
    pub kind: InputKind,
    pub raw_index: u8,
    pub raw_state: u8,
}

/// Parse one input report. Returns `None` for anything that is not a button/dial event.
pub fn parse_input(buf: &[u8]) -> Option<InputEvent> {
    if buf.len() < 12 || buf[..2] != [0x7c, 0x7c] {
        return None;
    }
    let cmd = u16::from_be_bytes([buf[2], buf[3]]);
    if cmd != IN_BUTTON && cmd != IN_BUTTON_2 {
        return None;
    }
    let [state, index, b2, b3] = [buf[8], buf[9], buf[10], buf[11]];
    let is_dial = (DIAL_FIRST..=DIAL_LAST).contains(&index) || b2 == 2;
    let kind = if is_dial {
        match b3 {
            0 => InputKind::Release,
            1 => InputKind::Press,
            2 => InputKind::TurnLeft,
            3 => InputKind::TurnRight,
            _ => return None,
        }
    } else if b3 == 1 {
        InputKind::Press
    } else {
        InputKind::Release
    };
    Some(InputEvent {
        control: Control::from_index(index),
        kind,
        raw_index: index,
        raw_state: state,
    })
}

/// Anything the deck sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceReport {
    Input(InputEvent),
    /// Reply to [`crate::request_device_info`]: a JSON object (serial number, firmware, ...).
    DeviceInfo(String),
    /// The deck asks for the label style; answer with [`crate::label_style`].
    LabelStyleRequest,
    /// Unknown report, with its command code and protocol ID as a u16.
    Other(u16),
}

pub fn parse_report(buf: &[u8]) -> DeviceReport {
    if buf.len() < 8 || buf[..2] != [0x7c, 0x7c] {
        return DeviceReport::Other(0);
    }
    let code = u16::from_be_bytes([buf[2], buf[3]]);
    match code {
        0x0303 => {
            let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
            let body = &buf[8..buf.len().min(8 + len)];
            DeviceReport::DeviceInfo(String::from_utf8_lossy(body).trim_end_matches('\0').to_string())
        }
        0x010b => DeviceReport::LabelStyleRequest,
        _ => match parse_input(buf) {
            Some(e) => DeviceReport::Input(e),
            None => DeviceReport::Other(code),
        },
    }
}
