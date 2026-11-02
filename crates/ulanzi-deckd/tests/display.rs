//! Display Brightness against a stand-in for KDE's brightness service on a private bus, and against
//! a stand-in `ddcutil` when that service is missing (skipped when `dbus-daemon` is not installed).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ulanzi_core::model::{BrightnessOp, DisplayTarget};
use ulanzi_deckd::actions::ActionError;
use ulanzi_deckd::actions::display::DisplayClient;
use zbus::Connection;

const ROOT: &str = "/org/kde/ScreenBrightness";

type Names = Arc<Mutex<Vec<String>>>;

struct Root {
    names: Names,
}

#[zbus::interface(name = "org.kde.ScreenBrightness")]
impl Root {
    #[zbus(property, name = "DisplaysDBusNames")]
    fn displays(&self) -> Vec<String> {
        self.names.lock().expect("names").clone()
    }
}

struct Display {
    label: String,
    level: Arc<Mutex<i32>>,
}

#[zbus::interface(name = "org.kde.ScreenBrightness.Display")]
impl Display {
    #[zbus(property)]
    fn label(&self) -> String {
        self.label.clone()
    }

    #[zbus(property)]
    fn brightness(&self) -> i32 {
        *self.level.lock().expect("level")
    }

    #[zbus(property)]
    fn max_brightness(&self) -> i32 {
        10000
    }

    fn set_brightness(&self, level: i32, _flags: u32) {
        *self.level.lock().expect("level") = level;
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
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
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

struct Desktop {
    _bus: Bus,
    server: Connection,
    names: Names,
    client: DisplayClient,
}

impl Desktop {
    async fn new() -> Option<Desktop> {
        let Some(bus) = bus() else {
            eprintln!("dbus-daemon not found; skipping");
            return None;
        };
        let server = connect(&bus).await;
        let names = Names::default();
        let root = Root { names: names.clone() };
        server.object_server().at(ROOT, root).await.expect("root");
        server
            .request_name("org.kde.ScreenBrightness")
            .await
            .expect("name");
        let client = DisplayClient::new(connect(&bus).await);
        Some(Desktop {
            _bus: bus,
            server,
            names,
            client,
        })
    }

    /// Connect a monitor as `displayN`; its level stays with the handle across renumbering.
    async fn plug(&self, n: u32, label: &str, level: &Arc<Mutex<i32>>) {
        let d = Display {
            label: label.into(),
            level: level.clone(),
        };
        let name = format!("display{n}");
        self.server
            .object_server()
            .at(format!("{ROOT}/{name}"), d)
            .await
            .expect("display");
        self.names.lock().expect("names").push(name);
    }

    async fn unplug(&self, n: u32) {
        let name = format!("display{n}");
        self.server
            .object_server()
            .remove::<Display, _>(format!("{ROOT}/{name}"))
            .await
            .expect("remove");
        self.names.lock().expect("names").retain(|x| *x != name);
    }
}

fn at(level: &Arc<Mutex<i32>>) -> i32 {
    *level.lock().expect("level")
}

fn level(v: i32) -> Arc<Mutex<i32>> {
    Arc::new(Mutex::new(v))
}

fn lg() -> DisplayTarget {
    DisplayTarget::Named {
        name: "LG TV".into(),
        nth: 0,
    }
}

#[tokio::test]
async fn a_key_follows_its_monitor_to_a_new_number() {
    let Some(desk) = Desktop::new().await else { return };
    let (tv, other) = (level(2000), level(1500));
    desk.plug(19, "LG TV", &tv).await;
    desk.plug(0, "Samsung", &other).await;
    let down = BrightnessOp::Down { step: 2 };
    desk.client.change(&lg(), down).await.expect("first");
    assert_eq!(at(&tv), 1800);

    // After a restart the same monitor has another number, and something else may hold the old one.
    desk.unplug(19).await;
    desk.plug(24, "LG TV", &tv).await;
    desk.client.change(&lg(), down).await.expect("second");
    assert_eq!(at(&tv), 1600);
    assert_eq!(at(&other), 1500, "the other monitor is left alone");
}

#[tokio::test]
async fn a_missing_monitor_says_so_and_works_again_when_it_is_back() {
    let Some(desk) = Desktop::new().await else { return };
    let tv = level(2000);
    desk.plug(3, "LG TV", &tv).await;
    desk.unplug(3).await;
    let up = BrightnessOp::Up { step: 10 };
    assert_eq!(
        desk.client.change(&lg(), up).await,
        Err(ActionError::Unavailable("LG TV isn’t connected".into()))
    );
    desk.plug(7, "LG TV", &tv).await;
    desk.client.change(&lg(), up).await.expect("back");
    assert_eq!(at(&tv), 3000);
}

#[tokio::test]
async fn all_displays_moves_each_from_its_own_level() {
    let Some(desk) = Desktop::new().await else { return };
    let (a, b) = (level(1000), level(9900));
    desk.plug(1, "A", &a).await;
    desk.plug(2, "B", &b).await;
    let up = BrightnessOp::Up { step: 2 };
    desk.client.change(&DisplayTarget::All, up).await.expect("all");
    assert_eq!((at(&a), at(&b)), (1200, 10000));
    assert_eq!(desk.client.labels().await.expect("labels"), ["A", "B"]);
}

#[tokio::test]
async fn quick_turns_all_count() {
    let Some(desk) = Desktop::new().await else { return };
    let tv = level(0);
    desk.plug(1, "LG TV", &tv).await;
    let up = BrightnessOp::Up { step: 1 };
    let (target, c) = (lg(), &desk.client);
    let turns = tokio::join!(
        c.change(&target, up),
        c.change(&target, up),
        c.change(&target, up),
        c.change(&target, up),
        c.change(&target, up),
    );
    assert_eq!(
        turns.0.and(turns.1).and(turns.2).and(turns.3).and(turns.4),
        Ok(())
    );
    assert_eq!(at(&tv), 500);
}

#[tokio::test]
async fn kde_goes_first_when_it_is_there() {
    let Some(desk) = Desktop::new().await else { return };
    let tv = level(2000);
    desk.plug(1, "LG TV", &tv).await;
    let tool = Ddcutil::new(&[(8, "ACM:MX34 Pro:", "VCP 10 C 50 100")]);
    let client = DisplayClient::new(connect(&desk._bus).await).with_ddcutil(tool.path(), WAIT);
    assert_eq!(client.labels().await.expect("labels"), ["LG TV"]);
    assert!(tool.calls().is_empty());
}

const WAIT: Duration = Duration::from_secs(5);

/// A stand-in `ddcutil` in a folder of its own, printing what the real one printed for these buses.
struct Ddcutil {
    dir: tempfile::TempDir,
}

impl Ddcutil {
    /// Monitors as (I2C bus, the `Monitor:` line, the `getvcp` line).
    fn new(monitors: &[(u32, &str, &str)]) -> Ddcutil {
        let dir = tempfile::tempdir().expect("dir");
        let mut detect = String::from(
            "Invalid display\n   I2C bus:          /dev/i2c-7\n   Monitor:          GSM:LG TV:\n\n",
        );
        for (n, (bus, name, vcp)) in monitors.iter().enumerate() {
            detect += &format!("Display {}\n   I2C bus:          /dev/i2c-{bus}\n", n + 1);
            detect += &format!("   DRM connector:    card1-DP-{n}\n   Monitor:          {name}\n\n");
            std::fs::write(dir.path().join(format!("vcp{bus}")), format!("{vcp}\n")).expect("vcp");
        }
        if monitors.is_empty() {
            detect = "No displays found.\n".into();
        }
        std::fs::write(dir.path().join("detect"), detect).expect("detect");
        let script = format!(
            r#"#!/bin/sh
cd '{}'
echo "$*" >> calls
case "$*" in
  "detect --brief") cat detect ;;
  *getvcp*) cat "vcp$2" ;;
  *setvcp*) sed -i "s/C [0-9]* /C $5 /" "vcp$2" ;;
esac
"#,
            dir.path().display()
        );
        let tool = dir.path().join("ddcutil");
        std::fs::write(&tool, script).expect("script");
        Command::new("chmod")
            .arg("+x")
            .arg(&tool)
            .status()
            .expect("chmod");
        Ddcutil { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("ddcutil")
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn writes(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|c| c.contains("setvcp"))
            .collect()
    }

    fn level(&self, bus: u32) -> String {
        let vcp = std::fs::read_to_string(self.dir.path().join(format!("vcp{bus}"))).expect("vcp");
        vcp.split_whitespace().nth(3).expect("level").to_string()
    }
}

/// A bus with no KDE service on it, so `tool` is what's left.
async fn no_kde(tool: PathBuf, wait: Duration) -> Option<(Bus, DisplayClient)> {
    let bus = bus()?;
    let client = DisplayClient::new(connect(&bus).await).with_ddcutil(tool, wait);
    Some((bus, client))
}

fn first() -> DisplayTarget {
    DisplayTarget::Named {
        name: "MX34 Pro".into(),
        nth: 0,
    }
}

const FIRST: (u32, &str, &str) = (8, "ACM:MX34 Pro:", "VCP 10 C 50 100");
const SECOND: (u32, &str, &str) = (9, "GBX:GX27:SN0000001", "VCP 10 C 30 100");

#[tokio::test]
async fn without_kde_the_monitors_come_from_ddcutil() {
    let tool = Ddcutil::new(&[FIRST, SECOND]);
    let Some((_bus, client)) = no_kde(tool.path(), WAIT).await else {
        return;
    };
    assert_eq!(client.labels().await.expect("labels"), ["MX34 Pro", "GX27"]);
}

#[tokio::test]
async fn a_monitor_that_cannot_report_brightness_is_left_out() {
    let tool = Ddcutil::new(&[FIRST, (9, "GBX:GX27:", "VCP 10 ERR")]);
    let Some((_bus, client)) = no_kde(tool.path(), WAIT).await else {
        return;
    };
    assert_eq!(client.labels().await.expect("labels"), ["MX34 Pro"]);
}

#[tokio::test]
async fn a_key_changes_only_its_monitor_through_ddcutil() {
    let tool = Ddcutil::new(&[FIRST, SECOND]);
    let Some((_bus, client)) = no_kde(tool.path(), WAIT).await else {
        return;
    };
    let up = BrightnessOp::Up { step: 10 };
    client.change(&first(), up).await.expect("change");
    assert_eq!((tool.level(8), tool.level(9)), ("60".into(), "30".into()));
    assert_eq!(tool.writes(), ["--bus 8 setvcp 10 60 --noverify"]);
}

#[tokio::test]
async fn a_monitors_own_scale_is_respected() {
    let tool = Ddcutil::new(&[(8, "ACM:MX34 Pro:", "VCP 10 C 50 200")]);
    let Some((_bus, client)) = no_kde(tool.path(), WAIT).await else {
        return;
    };
    let set = BrightnessOp::Set { percent: 50 };
    client.change(&first(), set).await.expect("change");
    assert_eq!(tool.level(8), "100");
}

#[tokio::test]
async fn quick_turns_wait_and_add_up() {
    let tool = Ddcutil::new(&[FIRST]);
    let Some((_bus, client)) = no_kde(tool.path(), WAIT).await else {
        return;
    };
    let up = BrightnessOp::Up { step: 1 };
    let (target, c) = (first(), &client);
    let turns = tokio::join!(
        c.change(&target, up),
        c.change(&target, up),
        c.change(&target, up),
        c.change(&target, up),
        c.change(&target, up),
    );
    assert_eq!(
        turns.0.and(turns.1).and(turns.2).and(turns.3).and(turns.4),
        Ok(())
    );
    assert_eq!(tool.level(8), "55");
    assert_eq!(tool.writes().len(), 2, "the first turn, then the rest together");
}

#[tokio::test]
async fn without_either_it_says_what_it_needs() {
    let Some((_bus, client)) = no_kde("/nonexistent/ddcutil".into(), WAIT).await else {
        return;
    };
    let up = BrightnessOp::Up { step: 2 };
    assert_eq!(
        client.change(&DisplayTarget::All, up).await,
        Err(ActionError::Unavailable(
            "Display brightness needs ddcutil or KDE Plasma 6".into()
        ))
    );
    assert!(client.labels().await.is_err());
}

#[tokio::test]
async fn no_monitor_answering_points_to_the_guide() {
    let tool = Ddcutil::new(&[]);
    let Some((_bus, client)) = no_kde(tool.path(), WAIT).await else {
        return;
    };
    assert_eq!(
        client.labels().await,
        Err(ActionError::Unavailable(
            "No monitor answered ddcutil. See Display Brightness in the User Guide".into()
        ))
    );
}

#[tokio::test]
async fn a_failed_write_says_what_ddcutil_said() {
    let tool = Ddcutil::new(&[FIRST]);
    let script = std::fs::read_to_string(tool.path()).expect("script");
    let refused = script.replace("*setvcp*) ", "*setvcp*) echo 'DDC: refused'; exit 1 ;; *never*) ");
    std::fs::write(tool.path(), refused).expect("script");
    let Some((_bus, client)) = no_kde(tool.path(), WAIT).await else {
        return;
    };
    let set = BrightnessOp::Set { percent: 20 };
    assert_eq!(
        client.change(&first(), set).await,
        Err(ActionError::Failed("DDC: refused".into()))
    );
}

#[tokio::test]
async fn a_hung_ddcutil_gives_up() {
    let tool = Ddcutil::new(&[FIRST]);
    std::fs::write(tool.path(), "#!/bin/sh\nexec sleep 30\n").expect("script");
    let wait = Duration::from_millis(100);
    let Some((_bus, client)) = no_kde(tool.path(), wait).await else {
        return;
    };
    assert_eq!(
        client.labels().await,
        Err(ActionError::Failed("ddcutil didn’t answer".into()))
    );
}
