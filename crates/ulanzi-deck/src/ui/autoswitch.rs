//! Auto-switch settings for one profile: the apps that bring it to the front, whether it is the
//! default, and its priority.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use ulanzi_core::model::{MatchRule, Profile};
use uuid::Uuid;

use crate::runtime;
use crate::state::{Change, State};
use crate::ui::action_editor::installed_apps;
use crate::ui::app_picker;

/// The app's name for a match entry (a desktop id).
pub fn app_name(entry: &str) -> String {
    installed_apps()
        .iter()
        .find(|a| a.desktop_id == entry)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| entry.trim_end_matches(".desktop").to_string())
}

/// The sidebar line under a profile's name.
pub fn summary(rule: Option<&MatchRule>, is_default: bool) -> String {
    let apps: Vec<String> = rule
        .map(|r| r.apps.iter().map(|a| app_name(a)).collect())
        .unwrap_or_default();
    let list = match apps.len() {
        0 => String::new(),
        1 | 2 => apps.join(", "),
        n => format!("{}, {} +{}", apps[0], apps[1], n - 2),
    };
    match (is_default, list.is_empty()) {
        (true, true) => "Default".into(),
        (true, false) => format!("Default · {list}"),
        (false, true) => "Manual".into(),
        (false, false) => format!("Auto · {list}"),
    }
}

/// Fetch the auto-switch rules of profiles the window doesn't have yet (for the sidebar).
pub fn load_rules(state: &State) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    let missing: Vec<Uuid> = state
        .profiles
        .borrow()
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| !state.rules.borrow().contains_key(id))
        .collect();
    if missing.is_empty() {
        return;
    }
    let st = state.clone();
    runtime::run(
        async move {
            let mut out = Vec::new();
            for id in missing {
                if let Ok(p) = c.profile(id).await {
                    out.push((id, p.r#match));
                }
            }
            out
        },
        move |rules| {
            if rules.is_empty() {
                return;
            }
            st.rules.borrow_mut().extend(rules);
            st.notify(Change::Profiles);
        },
    );
}

/// Save a profile's rule: through the editor when it is open (undoable), else directly.
fn save_rule(state: &State, id: Uuid, rule: MatchRule) {
    state.rules.borrow_mut().insert(id, rule.clone());
    if id == state.profile_id() {
        state.edit(|e| e.set_match(rule));
    } else if let Some(c) = state.client.borrow().clone() {
        let st = state.clone();
        runtime::run(
            async move {
                let mut p: Profile = c.profile(id).await?;
                p.r#match = rule;
                c.apply_now(&p).await
            },
            move |r| {
                if let Err(e) = r {
                    crate::ui::toast(&st, &format!("Not saved: {e}"));
                }
            },
        );
    }
    state.notify(Change::Profiles);
}

fn save_settings(state: &State) {
    let s = state.settings.borrow().clone();
    if let Some(c) = state.client.borrow().clone() {
        runtime::run(async move { c.apply_settings(&s).await }, |_| {});
    }
    state.notify(Change::Settings);
    state.notify(Change::Profiles);
}

type Refresh = Rc<dyn Fn()>;

/// Open the auto-switch settings for profile `id`.
pub fn open(state: &State, near: &impl IsA<gtk::Widget>, id: Uuid, name: &str) {
    let rule = if id == state.profile_id() {
        state.editor.borrow().profile().r#match.clone()
    } else {
        state.rules.borrow().get(&id).cloned().unwrap_or_default()
    };
    let rule = Rc::new(RefCell::new(rule));

    let page = adw::PreferencesPage::new();
    let apps_group = adw::PreferencesGroup::builder()
        .title(format!("Switch to “{name}” When…"))
        .description("One of these apps is in front. Works on KDE Plasma.")
        .build();
    page.add(&apps_group);

    let rows: Rc<RefCell<Vec<gtk::Widget>>> = Rc::default();
    // Rebuild the list of apps after each change.
    let refresh: Rc<RefCell<Option<Refresh>>> = Rc::default();
    let do_refresh: Rc<dyn Fn()> = {
        let (state, rule, rows, group, refresh) = (
            state.clone(),
            rule.clone(),
            rows.clone(),
            apps_group.clone(),
            refresh.clone(),
        );
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                group.remove(&r);
            }
            let current = rule.borrow().apps.clone();
            for entry in &current {
                let row = adw::ActionRow::new();
                row.add_prefix(&app_picker::app_label(Some(entry), &app_name(entry), 28));
                let remove = gtk::Button::builder()
                    .icon_name("list-remove-symbolic")
                    .tooltip_text("Remove")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat", "circular"])
                    .build();
                {
                    let (state, rule, entry, refresh) =
                        (state.clone(), rule.clone(), entry.clone(), refresh.clone());
                    remove.connect_clicked(move |_| {
                        rule.borrow_mut().apps.retain(|a| *a != entry);
                        save_rule(&state, id, rule.borrow().clone());
                        if let Some(f) = refresh.borrow().clone() {
                            f();
                        }
                    });
                }
                row.add_suffix(&remove);
                group.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
            // "Add App…" (apps already listed are left out of the picker).
            let add_row = adw::ActionRow::builder()
                .title(if current.is_empty() { "No apps yet" } else { "" })
                .build();
            let face = gtk::Box::builder().spacing(6).build();
            face.append(&gtk::Image::from_icon_name("list-add-symbolic"));
            face.append(&gtk::Label::new(Some("Add App…")));
            let picker = {
                let (state, rule, refresh) = (state.clone(), rule.clone(), refresh.clone());
                app_picker::button(&face, None, current.clone(), move |a| {
                    rule.borrow_mut().apps.push(a.desktop_id.clone());
                    save_rule(&state, id, rule.borrow().clone());
                    if let Some(f) = refresh.borrow().clone() {
                        f();
                    }
                })
            };
            add_row.add_suffix(&picker);
            group.add(&add_row);
            rows.borrow_mut().push(add_row.upcast());
        })
    };
    *refresh.borrow_mut() = Some(do_refresh.clone());
    do_refresh();

    let more = adw::PreferencesGroup::new();
    let default = adw::SwitchRow::builder()
        .title("Default Profile")
        .subtitle("Used when none of the listed apps is in front")
        .active(state.settings.borrow().default_profile == Some(id))
        .build();
    {
        let state = state.clone();
        default.connect_active_notify(move |r| {
            let mut s = state.settings.borrow_mut();
            if r.is_active() {
                s.default_profile = Some(id);
            } else if s.default_profile == Some(id) {
                s.default_profile = None;
            }
            drop(s);
            save_settings(&state);
        });
    }
    more.add(&default);
    let priority = adw::SpinRow::with_range(-10.0, 10.0, 1.0);
    priority.set_title("Priority");
    priority.set_subtitle("When two profiles list the same app, the higher one wins");
    priority.set_value(rule.borrow().priority as f64);
    {
        let (state, rule) = (state.clone(), rule.clone());
        priority.connect_value_notify(move |r| {
            rule.borrow_mut().priority = r.value() as i32;
            save_rule(&state, id, rule.borrow().clone());
        });
    }
    more.add(&priority);
    let on = adw::SwitchRow::builder()
        .title("Switch Profiles by App")
        .subtitle("For every profile. Also in the tray menu and as a key action.")
        .active(state.settings.borrow().auto_switch)
        .build();
    {
        let state = state.clone();
        on.connect_active_notify(move |r| {
            state.settings.borrow_mut().auto_switch = r.is_active();
            save_settings(&state);
        });
    }
    more.add(&on);
    page.add(&more);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    adw::Dialog::builder()
        .title("Auto-Switch")
        .content_width(460)
        .content_height(560)
        .child(&view)
        .build()
        .present(Some(near));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries() {
        let r = MatchRule {
            apps: vec!["a.desktop".into(), "b.desktop".into(), "c.desktop".into()],
            priority: 0,
        };
        assert_eq!(summary(None, true), "Default");
        assert_eq!(summary(None, false), "Manual");
        assert_eq!(summary(Some(&r), false), "Auto · a, b +1");
        assert_eq!(summary(Some(&r), true), "Default · a, b +1");
    }
}
