//! The tray icon (StatusNotifierItem): device state, profile switching, brightness, open the app.
//! Runs inside the daemon so it is there without the window open. `Settings.tray` turns it off.

use std::sync::Arc;
use std::time::Duration;

use ksni::TrayMethods;
use tokio::sync::{Notify, broadcast, mpsc, oneshot};
use ulanzi_core::model::BrightnessOp;
use ulanzi_core::store::Store;
use uuid::Uuid;

use crate::deck::{DeckEvent, DeckMsg};

const APP_ICON: &str = "io.github.tyaaa_aa.UlanziDeck";
/// One colour, so the panel can tint it to match (KDE and GNOME both recolour it).
const TRAY_ICON: &str = "io.github.tyaaa_aa.UlanziDeck-symbolic";
/// The same icon with an accent-coloured dot: profiles follow the app in front.
const TRAY_ICON_AUTO: &str = "io.github.tyaaa_aa.UlanziDeck-auto-symbolic";
const LEVELS: [u8; 4] = [25, 50, 75, 100];

/// The tray menu, independent of the toolkit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Header(String),
    Profiles {
        names: Vec<String>,
        selected: usize,
    },
    Brightness(Vec<u8>),
    /// Auto profile switching, with whether it is on.
    AutoSwitch(bool),
    Open,
    Separator,
    Quit,
}

fn state_text(state: &str, model: &str) -> String {
    let deck = if model.is_empty() { "Deck" } else { model };
    match state {
        "ready" => format!("{deck} · Connected"),
        "not_found" => "Deck not connected".into(),
        "no_permission" => format!("{deck} · Needs permission"),
        "busy" => format!("{deck} · In use by another program"),
        _ => format!("{deck} · Problem"),
    }
}

/// Build the menu for the current profiles, active profile and device state.
pub fn menu_model(
    profiles: &[(Uuid, String)],
    active: Uuid,
    state: &str,
    model: &str,
    auto_switch: bool,
) -> Vec<Entry> {
    vec![
        Entry::Header(state_text(state, model)),
        Entry::Separator,
        Entry::Profiles {
            names: profiles.iter().map(|(_, n)| n.clone()).collect(),
            selected: profiles
                .iter()
                .position(|(id, _)| *id == active)
                .unwrap_or(usize::MAX),
        },
        Entry::AutoSwitch(auto_switch),
        Entry::Separator,
        Entry::Brightness(LEVELS.to_vec()),
        Entry::Open,
        Entry::Separator,
        Entry::Quit,
    ]
}

/// What the tray shows; updated from deck events.
#[derive(Debug, Clone, Default)]
struct View {
    profiles: Vec<(Uuid, String)>,
    active: Uuid,
    state: String,
    model: String,
    auto_switch: bool,
}

/// The tray icon: with an accent dot while profiles follow the app in front. (A separate icon,
/// not an overlay emblem: Plasma's tray doesn't draw emblems.)
fn tray_icon(auto_switch: bool) -> &'static str {
    if auto_switch { TRAY_ICON_AUTO } else { TRAY_ICON }
}

fn tooltip_text(state: &str, model: &str, auto_switch: bool) -> String {
    let mut t = state_text(state, model);
    if auto_switch {
        t.push_str("\nAuto-switch on: profiles follow the app in front");
    }
    t
}

struct DeckTray {
    tx: mpsc::Sender<DeckMsg>,
    quit: Arc<Notify>,
    view: View,
}

/// Start the app in its own scope (so it outlives a service restart) and reap it when it exits.
fn open_app() {
    if let Err(e) = crate::actions::command::spawn_detached(&["ulanzi-deck".to_string()]) {
        tracing::warn!("cannot start ulanzi-deck: {e}");
    }
}

impl ksni::Tray for DeckTray {
    fn id(&self) -> String {
        "ulanzi-deck".into()
    }
    fn title(&self) -> String {
        "Ulanzi Deck".into()
    }
    fn icon_name(&self) -> String {
        tray_icon(self.view.auto_switch).into()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Ulanzi Deck".into(),
            description: tooltip_text(&self.view.state, &self.view.model, self.view.auto_switch),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        open_app();
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let v = &self.view;
        menu_model(&v.profiles, v.active, &v.state, &v.model, v.auto_switch)
            .into_iter()
            .map(|e| match e {
                Entry::Header(text) => StandardItem {
                    label: text,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                Entry::Separator => MenuItem::Separator,
                Entry::Profiles { names, selected } => RadioGroup {
                    selected,
                    select: Box::new(|this: &mut Self, i| {
                        if let Some((id, _)) = this.view.profiles.get(i) {
                            this.view.active = *id;
                            let _ = this.tx.try_send(DeckMsg::SwitchProfile(*id));
                        }
                    }),
                    options: names
                        .into_iter()
                        .map(|n| RadioItem {
                            label: n,
                            ..Default::default()
                        })
                        .collect(),
                }
                .into(),
                Entry::Brightness(levels) => SubMenu {
                    label: "Brightness".into(),
                    icon_name: "display-brightness-symbolic".into(),
                    submenu: levels
                        .into_iter()
                        .map(|p| {
                            StandardItem {
                                label: format!("{p}%"),
                                activate: Box::new(move |this: &mut Self| {
                                    let _ = this
                                        .tx
                                        .try_send(DeckMsg::Brightness(BrightnessOp::Set { percent: p }));
                                }),
                                ..Default::default()
                            }
                            .into()
                        })
                        .collect(),
                    ..Default::default()
                }
                .into(),
                Entry::AutoSwitch(on) => CheckmarkItem {
                    label: "Switch Profiles by App".into(),
                    checked: on,
                    activate: Box::new(|this: &mut Self| {
                        this.view.auto_switch = !this.view.auto_switch;
                        let _ = this.tx.try_send(DeckMsg::ToggleAutoSwitch);
                    }),
                    ..Default::default()
                }
                .into(),
                Entry::Open => StandardItem {
                    label: "Open Ulanzi Deck".into(),
                    icon_name: APP_ICON.into(),
                    activate: Box::new(|_: &mut Self| open_app()),
                    ..Default::default()
                }
                .into(),
                Entry::Quit => StandardItem {
                    label: "Quit Service".into(),
                    icon_name: "application-exit-symbolic".into(),
                    activate: Box::new(|this: &mut Self| this.quit.notify_one()),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }
}

async fn snapshot(tx: &mpsc::Sender<DeckMsg>) -> Option<View> {
    let (reply, rx) = oneshot::channel();
    tx.send(DeckMsg::Query(reply)).await.ok()?;
    let s = rx.await.ok()?;
    Some(View {
        profiles: s.profiles,
        active: s.profile,
        state: s.device.code().into(),
        model: format!("{:?}", s.model),
        auto_switch: s.auto_switch,
    })
}

/// Keep the tray in step with the deck and with `Settings.tray`; `quit` fires on "Quit Service".
pub async fn run(
    tx: mpsc::Sender<DeckMsg>,
    mut events: broadcast::Receiver<DeckEvent>,
    store: Store,
    quit: Arc<Notify>,
) {
    let mut handle: Option<ksni::Handle<DeckTray>> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(3));
    loop {
        tokio::select! {
            e = events.recv() => {
                if matches!(e, Err(broadcast::error::RecvError::Closed)) {
                    break;
                }
            }
            _ = tick.tick() => {}
        }
        let wanted = store.load_settings().map(|s| s.tray).unwrap_or(true);
        match (&handle, wanted) {
            (None, true) => {
                let Some(view) = snapshot(&tx).await else { continue };
                let tray = DeckTray {
                    tx: tx.clone(),
                    quit: quit.clone(),
                    view,
                };
                match tray.spawn().await {
                    Ok(h) => handle = Some(h),
                    Err(e) => {
                        // No tray host (e.g. GNOME without the AppIndicator extension).
                        tracing::info!("no tray icon: {e}");
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                }
            }
            (Some(h), true) => {
                if let Some(view) = snapshot(&tx).await {
                    h.update(move |t| t.view = view).await;
                }
            }
            (Some(h), false) => {
                h.shutdown().await;
                handle = None;
            }
            (None, false) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn auto_switch_shows_on_the_icon() {
        assert_eq!(tray_icon(true), TRAY_ICON_AUTO);
        assert_eq!(tray_icon(false), TRAY_ICON);
        assert!(tooltip_text("ready", "D200X", true).contains("Auto-switch on"));
        assert!(!tooltip_text("ready", "D200X", false).contains("Auto-switch on"));
    }

    #[test]
    fn menu_model_lists_profiles_and_state() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let profiles = vec![(a, "Default".to_string()), (b, "Gaming".to_string())];
        let m = menu_model(&profiles, b, "ready", "D200X", true);
        assert_eq!(m[0], Entry::Header("D200X · Connected".into()));
        assert!(m.contains(&Entry::Profiles {
            names: vec!["Default".into(), "Gaming".into()],
            selected: 1,
        }));
        assert!(m.contains(&Entry::Brightness(vec![25, 50, 75, 100])));
        assert!(m.contains(&Entry::AutoSwitch(true)));
        assert!(m.contains(&Entry::Open));
        assert_eq!(m.last(), Some(&Entry::Quit));

        let m = menu_model(&profiles, Uuid::from_u128(9), "not_found", "", false);
        assert_eq!(m[0], Entry::Header("Deck not connected".into()));
        // An unknown active profile selects nothing rather than the wrong one.
        assert!(m.contains(&Entry::Profiles {
            names: vec!["Default".into(), "Gaming".into()],
            selected: usize::MAX,
        }));
    }
}
