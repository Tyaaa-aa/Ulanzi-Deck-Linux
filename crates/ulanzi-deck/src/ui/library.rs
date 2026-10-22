//! The action library: every action as a card, by category. Drag a card onto the deck, or click it
//! to assign it to the selection.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use ulanzi_core::catalog::{self, ActionSpec, Category};

use crate::state::State;
use crate::ui::action_editor::action_chip;
use crate::ui::canvas::action_payload;

/// True when every word of `query` appears in the action's title or its category's name.
pub fn matches(spec: &ActionSpec, query: &str) -> bool {
    let hay = format!("{} {}", spec.title, spec.category.title()).to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

fn card(state: &State, spec: &'static ActionSpec) -> gtk::Widget {
    let action = (spec.make)();
    let b = gtk::Box::builder().spacing(8).build();
    b.append(&action_chip(state, Some(&action), 26));
    b.append(
        &gtk::Label::builder()
            .label(spec.title)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let button = gtk::Button::builder()
        .child(&b)
        .tooltip_text(format!("{}: drag onto the deck, or click to assign", spec.title))
        .css_classes(["action-card", "card"])
        .build();
    let drag = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
    {
        let payload = action_payload(&action);
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&payload.to_value())));
    }
    {
        let state = state.clone();
        drag.connect_drag_begin(move |src, _| {
            let look = catalog::default_look(&(spec.make)());
            if let Some(t) = crate::ui::canvas::key_texture(&look, &crate::ui::canvas::profile_dir(&state)) {
                src.set_icon(Some(&t), 24, 24);
            }
        });
    }
    button.add_controller(drag);
    {
        let state = state.clone();
        button.connect_clicked(move |_| assign(&state, (spec.make)()));
    }
    button.upcast()
}

/// Give the action to every selected control (one undo step).
fn assign(state: &State, action: ulanzi_core::model::Action) {
    let slots = state.selection.borrow().slots();
    if slots.is_empty() {
        crate::ui::toast(state, "Select a key first, or drag the action onto the deck");
        return;
    }
    state.edit(|e| {
        e.set_merge(Some("assign"));
        for s in &slots {
            e.drop_action(*s, action.clone());
        }
        e.set_merge(None);
    });
}

/// The library panel.
pub fn build(state: &State) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search actions")
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    root.append(&search);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_margin_start(12);
    column.set_margin_end(12);
    column.set_margin_bottom(18);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&column)
        .build();
    root.append(&scroller);

    let fill: Rc<dyn Fn(&str)> = {
        let (state, column) = (state.clone(), column.clone());
        Rc::new(move |query: &str| {
            while let Some(c) = column.first_child() {
                column.remove(&c);
            }
            column.append(
                &gtk::Label::builder()
                    .label("Drag an action onto a key, dial or the screen. Click one to give it to the selection.")
                    .wrap(true)
                    .xalign(0.0)
                    .css_classes(["dim-label", "caption"])
                    .margin_bottom(6)
                    .build(),
            );
            let mut any = false;
            for cat in Category::ALL {
                let specs: Vec<&'static ActionSpec> = catalog::catalog()
                    .iter()
                    .filter(|s| s.category == cat && matches(s, query))
                    .collect();
                if specs.is_empty() {
                    continue;
                }
                any = true;
                column.append(
                    &gtk::Label::builder()
                        .label(cat.title())
                        .xalign(0.0)
                        .css_classes(["heading"])
                        .margin_top(12)
                        .margin_bottom(4)
                        .build(),
                );
                let flow = gtk::FlowBox::builder()
                    .min_children_per_line(2)
                    .max_children_per_line(2)
                    .homogeneous(true)
                    .selection_mode(gtk::SelectionMode::None)
                    .row_spacing(6)
                    .column_spacing(6)
                    .build();
                for s in specs {
                    flow.append(&card(&state, s));
                }
                column.append(&flow);
            }
            if !any {
                column.append(
                    &adw::StatusPage::builder()
                        .icon_name("edit-find-symbolic")
                        .title("No Matching Actions")
                        .css_classes(["compact"])
                        .build(),
                );
            }
        })
    };
    fill("");
    search.connect_search_changed(move |s| fill(&s.text()));
    root.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str) -> &'static ActionSpec {
        catalog::catalog().iter().find(|s| s.id == id).expect(id)
    }

    #[test]
    fn library_filter_matches_title_and_category() {
        assert!(matches(spec("volume_up"), "vol"));
        assert!(matches(spec("volume_up"), "AUDIO up"));
        assert!(matches(spec("obs_record"), "obs"));
        assert!(matches(spec("play_pause"), "media"));
        assert!(matches(spec("play_pause"), ""));
        assert!(!matches(spec("play_pause"), "volume"));
    }
}
