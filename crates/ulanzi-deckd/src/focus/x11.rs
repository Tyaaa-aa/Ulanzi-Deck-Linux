//! X11: watch `_NET_ACTIVE_WINDOW` on the root window and read the window's class and title.

use tokio::sync::mpsc;
use ulanzi_core::autoswitch::Window;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt, EventMask};

use crate::deck::DeckMsg;

/// `WM_CLASS` is "instance\0class\0": the class is the second part.
pub fn class_from_wm_class(raw: &[u8]) -> String {
    let mut parts = raw.split(|b| *b == 0).filter(|p| !p.is_empty());
    let first = parts.next();
    let second = parts.next();
    String::from_utf8_lossy(second.or(first).unwrap_or_default()).into_owned()
}

fn watch(tx: mpsc::Sender<DeckMsg>) -> Result<(), String> {
    let (conn, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = conn.setup().roots[screen].root;
    let atom = |name: &str| -> Result<u32, String> {
        Ok(conn
            .intern_atom(false, name.as_bytes())
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .atom)
    };
    let active = atom("_NET_ACTIVE_WINDOW")?;
    let net_name = atom("_NET_WM_NAME")?;
    let utf8 = atom("UTF8_STRING")?;
    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    super::set_status("x11", "active", "Watching the app in front");
    let report = |conn: &x11rb::rust_connection::RustConnection| -> Option<Window> {
        let r = conn
            .get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let win = r.value32()?.next().filter(|w| *w != 0)?;
        let class = conn
            .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()?
            .reply()
            .ok()?;
        let title = conn
            .get_property(false, win, net_name, utf8, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        Some(Window {
            class: class_from_wm_class(&class.value),
            desktop_id: String::new(),
            title: String::from_utf8_lossy(&title.value).into_owned(),
        })
    };
    if let Some(w) = report(&conn) {
        let _ = tx.blocking_send(DeckMsg::Focus(w));
    }
    loop {
        let ev = conn.wait_for_event().map_err(|e| e.to_string())?;
        if let Event::PropertyNotify(p) = ev
            && p.atom == active
            && let Some(w) = report(&conn)
            && tx.blocking_send(DeckMsg::Focus(w)).is_err()
        {
            return Ok(());
        }
    }
}

pub async fn run(tx: mpsc::Sender<DeckMsg>) {
    match tokio::task::spawn_blocking(move || watch(tx)).await {
        Ok(Err(e)) => super::set_status("x11", "unavailable", &format!("Can't watch windows: {e}")),
        Err(e) => super::set_status("x11", "unavailable", &e.to_string()),
        Ok(Ok(())) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wm_class_takes_the_class_part() {
        assert_eq!(class_from_wm_class(b"navigator\0firefox\0"), "firefox");
        assert_eq!(class_from_wm_class(b"code\0"), "code");
        assert_eq!(class_from_wm_class(b""), "");
    }
}
