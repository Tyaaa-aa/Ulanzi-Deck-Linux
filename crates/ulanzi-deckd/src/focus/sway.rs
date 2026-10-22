//! Sway (and i3): subscribe to window events on the IPC socket.

use std::path::PathBuf;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use ulanzi_core::autoswitch::Window;

use crate::deck::DeckMsg;

const MAGIC: &[u8; 6] = b"i3-ipc";
const SUBSCRIBE: u32 = 2;

/// An IPC message: magic, payload length and type (little-endian u32s), payload.
pub fn frame(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// (payload length, type) from a 14-byte header.
pub fn header(h: &[u8; 14]) -> Option<(u32, u32)> {
    if &h[..6] != MAGIC {
        return None;
    }
    let len = u32::from_le_bytes([h[6], h[7], h[8], h[9]]);
    let kind = u32::from_le_bytes([h[10], h[11], h[12], h[13]]);
    Some((len, kind))
}

/// A window event → the focused window (only `change: "focus"`).
pub fn parse_window_event(json: &str) -> Option<Window> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v.get("change")?.as_str()? != "focus" {
        return None;
    }
    let c = v.get("container")?;
    let class = c
        .get("app_id")
        .and_then(|a| a.as_str())
        .or_else(|| c.pointer("/window_properties/class").and_then(|a| a.as_str()))
        .unwrap_or_default();
    Some(Window {
        class: class.to_string(),
        desktop_id: String::new(),
        title: c
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

async fn session(socket: &PathBuf, tx: &mpsc::Sender<DeckMsg>) -> std::io::Result<()> {
    let mut s = tokio::net::UnixStream::connect(socket).await?;
    s.write_all(&frame(SUBSCRIBE, br#"["window"]"#)).await?;
    super::set_status("sway", "active", "Watching the app in front");
    loop {
        let mut h = [0u8; 14];
        s.read_exact(&mut h).await?;
        let (len, kind) = header(&h).ok_or_else(|| std::io::Error::other("not an i3 IPC reply"))?;
        if len > 16 * 1024 * 1024 {
            return Err(std::io::Error::other("IPC message too large"));
        }
        let mut payload = vec![0u8; len as usize];
        s.read_exact(&mut payload).await?;
        // Events have the high bit set; window events are type 3.
        if kind == 0x8000_0003
            && let Some(w) = parse_window_event(&String::from_utf8_lossy(&payload))
            && tx.send(DeckMsg::Focus(w)).await.is_err()
        {
            return Ok(());
        }
    }
}

pub async fn run(socket: PathBuf, tx: mpsc::Sender<DeckMsg>) {
    loop {
        if let Err(e) = session(&socket, &tx).await {
            super::set_status("sway", "waiting", &format!("Can't reach Sway: {e}"));
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i3_ipc_frame_round_trip() {
        let f = frame(SUBSCRIBE, br#"["window"]"#);
        let h: [u8; 14] = f[..14].try_into().expect("header");
        assert_eq!(header(&h), Some((10, SUBSCRIBE)));
        assert_eq!(&f[14..], br#"["window"]"#);
        assert_eq!(header(&[0u8; 14]), None);
    }

    #[test]
    fn sway_event_parses() {
        let wayland = r#"{"change":"focus","container":{"app_id":"org.gnome.Nautilus","name":"Home"}}"#;
        let w = parse_window_event(wayland).expect("focus");
        assert_eq!(
            (w.class.as_str(), w.title.as_str()),
            ("org.gnome.Nautilus", "Home")
        );
        let xwayland = r#"{"change":"focus","container":{"app_id":null,"window_properties":{"class":"Steam"},"name":"Steam"}}"#;
        assert_eq!(parse_window_event(xwayland).expect("focus").class, "Steam");
        assert!(parse_window_event(r#"{"change":"title","container":{}}"#).is_none());
    }
}
