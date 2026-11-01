//! Brightness of PC monitors through KDE's `org.kde.ScreenBrightness` (Plasma 6), or `ddcutil` on
//! any other desktop. The list is read on every press: the `displayN` and I2C bus numbers change
//! between sessions, so none is ever kept.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::Command;
use ulanzi_core::display::{next_level, pick};
use ulanzi_core::model::{BrightnessOp, DisplayTarget};
use zbus::Connection;
use zbus::proxy::{Builder, CacheProperties, Proxy};

use super::ActionError;

const SERVICE: &str = "org.kde.ScreenBrightness";
const ROOT: &str = "/org/kde/ScreenBrightness";
const DISPLAY: &str = "org.kde.ScreenBrightness.Display";

/// Where a monitor's brightness is set.
enum Route {
    Kde(String),
    Ddc(u32),
}

/// One connected monitor as the desktop reports it now.
struct Monitor {
    route: Route,
    label: String,
    level: i32,
    max: i32,
}

/// Talks to the desktop's brightness service, or to `ddcutil` where there is none.
#[derive(Debug, Clone)]
pub struct DisplayClient {
    conn: Connection,
    ddcutil: PathBuf,
    wait: Duration,
    /// Turns not applied yet. Whoever gets [`DisplayClient::turn`] takes them all, so quick dial
    /// turns add up to one write however slow the monitor is.
    waiting: Arc<Mutex<Vec<(DisplayTarget, BrightnessOp)>>>,
    turn: Arc<tokio::sync::Mutex<()>>,
}

impl DisplayClient {
    pub fn new(conn: Connection) -> DisplayClient {
        DisplayClient {
            conn,
            ddcutil: "ddcutil".into(),
            wait: Duration::from_secs(15),
            waiting: Arc::default(),
            turn: Arc::default(),
        }
    }

    /// Use this `ddcutil` and give it `wait` to answer.
    pub fn with_ddcutil(mut self, tool: PathBuf, wait: Duration) -> DisplayClient {
        self.ddcutil = tool;
        self.wait = wait;
        self
    }

    async fn proxy(&self, path: String, interface: &'static str) -> zbus::Result<Proxy<'static>> {
        Builder::new(&self.conn)
            .destination(SERVICE)?
            .path(path)?
            .interface(interface)?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    async fn monitors(&self) -> Result<Vec<Monitor>, ActionError> {
        match self.kde_monitors().await {
            Err(e) if no_service(&e) => self.ddc_monitors().await,
            Err(e) => Err(failed(e)),
            Ok(list) => Ok(list),
        }
    }

    async fn kde_monitors(&self) -> zbus::Result<Vec<Monitor>> {
        let root = self.proxy(ROOT.to_string(), SERVICE).await?;
        let names: Vec<String> = root.get_property("DisplaysDBusNames").await?;
        let mut out = Vec::new();
        for name in names {
            let path = format!("{ROOT}/{name}");
            // One that goes away between the list and this read is simply not connected.
            if let Ok(m) = self.kde_monitor(path).await {
                out.push(m);
            }
        }
        Ok(out)
    }

    async fn kde_monitor(&self, path: String) -> zbus::Result<Monitor> {
        let p = self.proxy(path.clone(), DISPLAY).await?;
        Ok(Monitor {
            label: p.get_property("Label").await?,
            level: p.get_property("Brightness").await?,
            max: p.get_property("MaxBrightness").await?,
            route: Route::Kde(path),
        })
    }

    async fn ddc_monitors(&self) -> Result<Vec<Monitor>, ActionError> {
        let found = self.run_ddcutil(&["detect", "--brief"]).await?;
        let mut out = Vec::new();
        for (bus, label) in parse_detect(&String::from_utf8_lossy(&found.stdout)) {
            let read = self
                .run_ddcutil(&["--bus", &bus.to_string(), "getvcp", "10", "--brief"])
                .await?;
            // A monitor that can't report its brightness can't be stepped either.
            if let Some((level, max)) = parse_level(&String::from_utf8_lossy(&read.stdout)) {
                out.push(Monitor {
                    route: Route::Ddc(bus),
                    label,
                    level,
                    max,
                });
            }
        }
        if out.is_empty() {
            return Err(ActionError::Unavailable(
                "No monitor answered ddcutil. See Display Brightness in the User Guide".into(),
            ));
        }
        Ok(out)
    }

    async fn run_ddcutil(&self, args: &[&str]) -> Result<std::process::Output, ActionError> {
        let run = Command::new(&self.ddcutil).args(args).kill_on_drop(true).output();
        tokio::time::timeout(self.wait, run)
            .await
            .map_err(|_| ActionError::Failed("ddcutil didn’t answer".into()))?
            .map_err(|_| ActionError::Unavailable("Display brightness needs ddcutil or KDE Plasma 6".into()))
    }

    async fn set(&self, m: &Monitor) -> Result<(), ActionError> {
        match &m.route {
            Route::Kde(path) => {
                let p = self.proxy(path.clone(), DISPLAY).await.map_err(failed)?;
                p.call_method("SetBrightness", &(m.level, 0u32))
                    .await
                    .map_err(failed)?;
            }
            Route::Ddc(bus) => {
                let (bus, level) = (bus.to_string(), m.level.to_string());
                let args = ["--bus", &bus, "setvcp", "10", &level, "--noverify"];
                let out = self.run_ddcutil(&args).await?;
                if !out.status.success() {
                    let said = String::from_utf8_lossy(&out.stdout);
                    return Err(ActionError::Failed(said.trim().to_string()));
                }
            }
        }
        Ok(())
    }

    /// The names of the connected monitors, in the desktop's order.
    pub async fn labels(&self) -> Result<Vec<String>, ActionError> {
        Ok(self.monitors().await?.into_iter().map(|m| m.label).collect())
    }

    pub async fn change(&self, target: &DisplayTarget, op: BrightnessOp) -> Result<(), ActionError> {
        self.waiting.lock().expect("waiting").push((target.clone(), op));
        let _turn = self.turn.lock().await;
        let turns = std::mem::take(&mut *self.waiting.lock().expect("waiting"));
        if turns.is_empty() {
            return Ok(()); // the turn before this one took it along
        }
        let mut monitors = self.monitors().await?;
        let labels: Vec<String> = monitors.iter().map(|m| m.label.clone()).collect();
        let mut moved = vec![false; monitors.len()];
        let mut result = Ok(());
        for (target, op) in &turns {
            match chosen(&labels, target) {
                Ok(list) => {
                    for i in list {
                        let m = &mut monitors[i];
                        m.level = next_level(m.level, m.max, *op);
                        moved[i] = true;
                    }
                }
                Err(e) => result = Err(e),
            }
        }
        for (m, _) in monitors.iter().zip(moved).filter(|(_, moved)| *moved) {
            self.set(m).await?;
        }
        result
    }
}

fn chosen(labels: &[String], target: &DisplayTarget) -> Result<Vec<usize>, ActionError> {
    let Some(list) = pick(labels, target) else {
        let name = match target {
            DisplayTarget::Named { name, .. } => name.as_str(),
            DisplayTarget::All => "That display",
        };
        return Err(ActionError::Unavailable(format!("{name} isn’t connected")));
    };
    if list.is_empty() {
        return Err(ActionError::Unavailable(
            "No display here can change its brightness".into(),
        ));
    }
    Ok(list)
}

/// The usable monitors in `ddcutil detect --brief`, as I2C bus and model. "Invalid display" entries
/// are ones that don't answer DDC/CI.
fn parse_detect(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let (mut usable, mut bus) = (false, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with("Display ") {
            usable = true;
        } else if line.starts_with("Invalid display") {
            usable = false;
        } else if let Some(rest) = line.strip_prefix("I2C bus:") {
            bus = rest.trim().strip_prefix("/dev/i2c-").and_then(|n| n.parse().ok());
        } else if let Some(rest) = line.strip_prefix("Monitor:")
            && usable
            && let Some(bus) = bus
        {
            // maker:model:serial
            let rest = rest.trim();
            let model = rest.split_once(':').and_then(|(_, r)| r.rsplit_once(':'));
            let label = model.map_or(rest, |(model, _)| model);
            out.push((bus, label.to_string()));
        }
    }
    out
}

/// Level and maximum from `ddcutil getvcp 10 --brief`: `VCP 10 C 50 100`.
fn parse_level(text: &str) -> Option<(i32, i32)> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    match fields[..] {
        ["VCP", "10", "C", level, max] => Some((level.parse().ok()?, max.parse().ok()?)),
        _ => None,
    }
}

fn failed(e: zbus::Error) -> ActionError {
    ActionError::Failed(e.to_string())
}

/// Nothing on the session bus answers to the brightness service. It reaches here as a method
/// error or wrapped in a standard one, so the text is what they share.
fn no_service(e: &zbus::Error) -> bool {
    let text = e.to_string();
    text.contains("ServiceUnknown") || text.contains("NameHasNoOwner")
}
