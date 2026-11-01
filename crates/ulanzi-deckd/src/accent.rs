//! The desktop's accent colour, from the settings portal.

use ulanzi_core::Color;
use zbus::Connection;
use zbus::zvariant::OwnedValue;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";

/// `None` when there is no portal, it has no such setting, or the desktop has no accent.
pub async fn read_accent(conn: &Connection) -> Option<Color> {
    let reply = conn
        .call_method(
            Some(PORTAL),
            PATH,
            Some(SETTINGS),
            "ReadOne",
            &("org.freedesktop.appearance", "accent-color"),
        )
        .await
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    let (r, g, b) = <(f64, f64, f64)>::try_from(value).ok()?;
    let channel = |v: f64| (0.0..=1.0).contains(&v).then(|| (v * 255.0).round() as u8);
    Some(Color::rgb(channel(r)?, channel(g)?, channel(b)?))
}
