use crate::packet::{Command, Packet, simple};

/// Set display brightness. Values above 100 are clamped.
pub fn brightness(percent: u8) -> Packet {
    simple(Command::Brightness, percent.min(100).to_string().as_bytes())
}

/// Label style applied by the firmware to every key's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelStyle {
    pub align: String,
    pub color: u32,
    pub font_name: String,
    pub show_title: bool,
    pub size: u32,
    pub weight: u32,
}

impl Default for LabelStyle {
    fn default() -> Self {
        Self {
            align: "bottom".into(),
            color: 0xFF_FF_FF,
            font_name: "Roboto".into(),
            show_title: true,
            size: 10,
            weight: 80,
        }
    }
}

impl LabelStyle {
    /// JSON exactly as Python's `json.dumps(dict)` writes it (`", "` and `": "` separators).
    pub fn to_python_json(&self) -> String {
        format!(
            r#"{{"Align": {}, "Color": {}, "FontName": {}, "ShowTitle": {}, "Size": {}, "Weight": {}}}"#,
            crate::json::py_str(&self.align),
            self.color,
            crate::json::py_str(&self.font_name),
            self.show_title,
            self.size,
            self.weight
        )
    }
}

pub fn label_style(style: &LabelStyle) -> Packet {
    simple(Command::LabelStyle, style.to_python_json().as_bytes())
}

/// What the wide screen shows. IDs are Ulanzi Studio's (its small-window editor's radio buttons).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    /// CPU / RAM / GPU numbers drawn by the firmware.
    Stats = 0,
    /// Firmware clock.
    Clock = 1,
    /// The image uploaded for the wide screen (manifest key `3_2`), nothing on top.
    Image = 2,
    /// Firmware clock with date and weekday.
    ClockDateWeekday = 200,
    /// Firmware clock with weekday.
    ClockWeekday = 201,
    /// Firmware clock with date.
    ClockDate = 202,
    /// Large firmware clock.
    ClockLarge = 203,
}

/// The wide-screen ("small window") update. Ulanzi Studio sends it once a second, which also keeps
/// the deck from locking itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallWindow {
    pub mode: ScreenMode,
    pub cpu: u8,
    pub mem: u8,
    /// `HH:MM:SS`, 24-hour.
    pub time: String,
    pub gpu: u8,
    pub clock_24h: bool,
    /// Date text for the date/weekday clock modes, empty otherwise.
    pub date: String,
}

/// Payload `mode|cpu|mem|HH:MM:SS|gpu|24H or 12H|date`.
pub fn small_window(w: &SmallWindow) -> Packet {
    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        w.mode as u8,
        w.cpu,
        w.mem,
        w.time,
        w.gpu,
        if w.clock_24h { "24H" } else { "12H" },
        w.date
    );
    simple(Command::SmallWindow, payload.as_bytes())
}

/// Ask the deck for its info; it replies with JSON (serial number, firmware version, ...).
pub fn request_device_info() -> Packet {
    simple(Command::DeviceInfo, b"")
}

/// Dismiss the lock screen. The deck locks itself after ~2 minutes without host traffic and then
/// reports no key presses until this is sent.
pub fn unlock() -> Packet {
    simple(Command::Unlock, b"")
}

/// Deck sleep time in minutes; 0 = never.
pub fn sleep_time(minutes: u32) -> Packet {
    simple(Command::SleepTime, minutes.to_string().as_bytes())
}
