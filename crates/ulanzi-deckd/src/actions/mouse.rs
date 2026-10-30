//! The virtual mouse macros play through, and the input interface the player uses.

use std::sync::{Arc, Mutex};

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent, KeyCode, PropType, RelativeAxisCode,
    UinputAbsSetup,
};
use ulanzi_core::model::MouseButton;

use super::ActionError;
use super::keys::VirtualKeyboard;

/// The range of the absolute pointer. The desktop maps it onto its whole area.
const ABS_MAX: i32 = 32767;

/// What the macro player presses. Faked in tests.
pub trait InputSink: Send {
    fn key(&mut self, key: &str, down: bool) -> Result<(), ActionError>;
    fn button(&mut self, button: MouseButton, down: bool) -> Result<(), ActionError>;
    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), ActionError>;
    fn move_by(&mut self, dx: i32, dy: i32) -> Result<(), ActionError>;
    fn move_to(&mut self, x: f32, y: f32) -> Result<(), ActionError>;
}

pub fn button_code(button: MouseButton) -> KeyCode {
    match button {
        MouseButton::Left => KeyCode::BTN_LEFT,
        MouseButton::Right => KeyCode::BTN_RIGHT,
        MouseButton::Middle => KeyCode::BTN_MIDDLE,
        MouseButton::Back => KeyCode::BTN_SIDE,
        MouseButton::Forward => KeyCode::BTN_EXTRA,
    }
}

/// A share of the desktop as an absolute pointer value.
pub fn abs_value(share: f32) -> i32 {
    (share.clamp(0.0, 1.0) * ABS_MAX as f32).round() as i32
}

const BUTTONS: [MouseButton; 5] = [
    MouseButton::Left,
    MouseButton::Right,
    MouseButton::Middle,
    MouseButton::Back,
    MouseButton::Forward,
];

fn unavailable(e: std::io::Error) -> ActionError {
    ActionError::Unavailable(format!(
        "macros need access to /dev/uinput ({e}); run the permission step in setup"
    ))
}

fn failed(e: std::io::Error) -> ActionError {
    ActionError::Failed(format!("could not send input: {e}"))
}

/// A virtual mouse for buttons, scrolling and relative movement, and a separate absolute pointer:
/// a device with both kinds of axis is not treated as a mouse.
pub struct VirtualMouse {
    mouse: VirtualDevice,
    pointer: VirtualDevice,
}

impl VirtualMouse {
    pub fn new() -> Result<VirtualMouse, ActionError> {
        let mut buttons = AttributeSet::<KeyCode>::new();
        for b in BUTTONS {
            buttons.insert(button_code(b));
        }
        let mut axes = AttributeSet::<RelativeAxisCode>::new();
        for a in [
            RelativeAxisCode::REL_X,
            RelativeAxisCode::REL_Y,
            RelativeAxisCode::REL_WHEEL,
            RelativeAxisCode::REL_HWHEEL,
        ] {
            axes.insert(a);
        }
        let mouse = VirtualDevice::builder()
            .map_err(unavailable)?
            .name("Ulanzi Deck virtual mouse")
            .with_keys(&buttons)
            .map_err(unavailable)?
            .with_relative_axes(&axes)
            .map_err(unavailable)?
            .build()
            .map_err(unavailable)?;
        let info = AbsInfo::new(0, 0, ABS_MAX, 0, 0, 0);
        let mut left = AttributeSet::<KeyCode>::new();
        left.insert(KeyCode::BTN_LEFT);
        let mut pointer_prop = AttributeSet::<PropType>::new();
        pointer_prop.insert(PropType::POINTER);
        let pointer = VirtualDevice::builder()
            .map_err(unavailable)?
            .name("Ulanzi Deck virtual pointer")
            .with_keys(&left)
            .map_err(unavailable)?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, info))
            .map_err(unavailable)?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, info))
            .map_err(unavailable)?
            .with_properties(&pointer_prop)
            .map_err(unavailable)?
            .build()
            .map_err(unavailable)?;
        Ok(VirtualMouse { mouse, pointer })
    }
}

/// The real devices. The keyboard is shared with the Keyboard Shortcut action.
pub struct VirtualInput {
    keyboard: Arc<Mutex<Option<VirtualKeyboard>>>,
    mouse: Option<VirtualMouse>,
}

impl VirtualInput {
    pub fn new(keyboard: Arc<Mutex<Option<VirtualKeyboard>>>) -> VirtualInput {
        let mouse = match VirtualMouse::new() {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::warn!("{e}");
                None
            }
        };
        VirtualInput { keyboard, mouse }
    }

    fn mouse(&mut self) -> Result<&mut VirtualMouse, ActionError> {
        if self.mouse.is_none() {
            self.mouse = Some(VirtualMouse::new()?);
        }
        self.mouse
            .as_mut()
            .ok_or_else(|| ActionError::Failed("no virtual mouse".into()))
    }
}

impl InputSink for VirtualInput {
    fn key(&mut self, key: &str, down: bool) -> Result<(), ActionError> {
        super::set_key(&self.keyboard, key, down)
    }

    fn button(&mut self, button: MouseButton, down: bool) -> Result<(), ActionError> {
        let event = InputEvent::new(EventType::KEY.0, button_code(button).0, down as i32);
        self.mouse()?.mouse.emit(&[event]).map_err(failed)
    }

    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), ActionError> {
        let rel = EventType::RELATIVE.0;
        let events = [
            InputEvent::new(rel, RelativeAxisCode::REL_HWHEEL.0, dx),
            // Positive is away from the person, the other way round from the screen.
            InputEvent::new(rel, RelativeAxisCode::REL_WHEEL.0, -dy),
        ];
        self.mouse()?.mouse.emit(&events).map_err(failed)
    }

    fn move_by(&mut self, dx: i32, dy: i32) -> Result<(), ActionError> {
        let rel = EventType::RELATIVE.0;
        let events = [
            InputEvent::new(rel, RelativeAxisCode::REL_X.0, dx),
            InputEvent::new(rel, RelativeAxisCode::REL_Y.0, dy),
        ];
        self.mouse()?.mouse.emit(&events).map_err(failed)
    }

    fn move_to(&mut self, x: f32, y: f32) -> Result<(), ActionError> {
        let abs = EventType::ABSOLUTE.0;
        let events = [
            InputEvent::new(abs, AbsoluteAxisCode::ABS_X.0, abs_value(x)),
            InputEvent::new(abs, AbsoluteAxisCode::ABS_Y.0, abs_value(y)),
        ];
        self.mouse()?.pointer.emit(&events).map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_button_has_its_code() {
        assert_eq!(button_code(MouseButton::Left), KeyCode::BTN_LEFT);
        assert_eq!(button_code(MouseButton::Right), KeyCode::BTN_RIGHT);
        assert_eq!(button_code(MouseButton::Middle), KeyCode::BTN_MIDDLE);
        assert_eq!(button_code(MouseButton::Back), KeyCode::BTN_SIDE);
        assert_eq!(button_code(MouseButton::Forward), KeyCode::BTN_EXTRA);
    }

    #[test]
    fn shares_map_onto_the_absolute_range() {
        assert_eq!(abs_value(0.0), 0);
        assert_eq!(abs_value(1.0), 32767);
        assert_eq!(abs_value(0.5), 16384);
        assert_eq!(abs_value(2.0), 32767, "off the desktop stays on it");
        assert_eq!(abs_value(-1.0), 0);
    }
}
