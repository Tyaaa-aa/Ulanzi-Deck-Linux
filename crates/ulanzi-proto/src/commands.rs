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

/// What the wide screen shows when the firmware draws it.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    Stats = 0,
    Clock = 1,
    Background = 2,
}

/// Data for the firmware-drawn wide screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallWindow {
    pub mode: ScreenMode,
    pub cpu: u8,
    pub mem: u8,
    pub time: String,
    pub gpu: u8,
}

pub fn small_window(w: &SmallWindow) -> Packet {
    let payload = format!("{}|{}|{}|{}|{}", w.mode as u8, w.cpu, w.mem, w.time, w.gpu);
    simple(Command::SmallWindow, payload.as_bytes())
}
