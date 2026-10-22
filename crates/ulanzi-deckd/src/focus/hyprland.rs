//! Hyprland: its event socket reports `activewindow>>CLASS,TITLE`.

use std::path::PathBuf;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use ulanzi_core::autoswitch::Window;

use crate::deck::DeckMsg;

/// One event line → the window, when it is an `activewindow` event.
pub fn parse_line(line: &str) -> Option<Window> {
    let rest = line.strip_prefix("activewindow>>")?;
    let (class, title) = rest.split_once(',').unwrap_or((rest, ""));
    Some(Window {
        class: class.to_string(),
        desktop_id: String::new(),
        title: title.to_string(),
    })
}

pub async fn run(socket: PathBuf, tx: mpsc::Sender<DeckMsg>) {
    super::set_status("hyprland", "waiting", "Connecting to Hyprland");
    loop {
        match tokio::net::UnixStream::connect(&socket).await {
            Ok(s) => {
                super::set_status("hyprland", "active", "Watching the app in front");
                let mut lines = BufReader::new(s).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(w) = parse_line(&line)
                        && tx.send(DeckMsg::Focus(w)).await.is_err()
                    {
                        return;
                    }
                }
            }
            Err(e) => super::set_status("hyprland", "waiting", &format!("Can't reach Hyprland: {e}")),
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_line_parses() {
        let w = parse_line("activewindow>>code,main.rs - Code, OSS").expect("event");
        assert_eq!(w.class, "code");
        assert_eq!(w.title, "main.rs - Code, OSS");
        assert!(parse_line("workspace>>2").is_none());
    }
}
