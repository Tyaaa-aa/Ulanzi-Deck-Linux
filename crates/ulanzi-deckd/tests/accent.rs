//! The desktop's accent colour, against a stand-in portal on a private bus (skipped when
//! `dbus-daemon` is not installed).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use ulanzi_core::Color;
use ulanzi_deckd::accent::read_accent;
use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

struct Portal {
    accent: (f64, f64, f64),
}

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Portal {
    fn read_one(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        assert_eq!((namespace, key), ("org.freedesktop.appearance", "accent-color"));
        OwnedValue::try_from(Value::from(self.accent)).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
}

struct Bus {
    child: Child,
    addr: String,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bus() -> Option<Bus> {
    // With no data folders it finds no services to start: it would start the real portal.
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .env("XDG_DATA_DIRS", "/nonexistent")
        .env("XDG_DATA_HOME", "/nonexistent")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take()?).read_line(&mut line).ok()?;
    Some(Bus {
        child,
        addr: line.trim().to_string(),
    })
}

async fn connect(bus: &Bus) -> Connection {
    zbus::connection::Builder::address(bus.addr.as_str())
        .expect("address")
        .build()
        .await
        .expect("connect")
}

async fn with_portal(bus: &Bus, accent: (f64, f64, f64)) -> Connection {
    let server = connect(bus).await;
    server
        .object_server()
        .at("/org/freedesktop/portal/desktop", Portal { accent })
        .await
        .expect("portal");
    server
        .request_name("org.freedesktop.portal.Desktop")
        .await
        .expect("name");
    server
}

#[tokio::test]
async fn the_portal_accent_is_read() {
    let Some(bus) = bus() else { return };
    let _portal = with_portal(&bus, (1.0, 0.5, 0.0)).await;
    assert_eq!(
        read_accent(&connect(&bus).await).await,
        Some(Color::rgb(255, 128, 0))
    );
}

#[tokio::test]
async fn no_portal_means_no_accent() {
    let Some(bus) = bus() else { return };
    assert_eq!(read_accent(&connect(&bus).await).await, None);
}

#[tokio::test]
async fn a_colour_out_of_range_means_no_accent() {
    let Some(bus) = bus() else { return };
    let _portal = with_portal(&bus, (-1.0, -1.0, -1.0)).await;
    assert_eq!(read_accent(&connect(&bus).await).await, None);
}
