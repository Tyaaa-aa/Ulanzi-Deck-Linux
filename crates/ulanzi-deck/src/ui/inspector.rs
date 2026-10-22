//! Right-hand panel: the inspector for the selection, or the action library.

use adw::prelude::*;

use crate::state::{Change, State};

pub struct Panel {
    pub root: gtk::Widget,
}

impl Panel {
    pub fn build(state: &State, library_toggle: &gtk::ToggleButton) -> Panel {
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("input-mouse-symbolic")
            .title("Nothing Selected")
            .description("Click a key, dial or button on the deck to edit it")
            .build();
        empty.add_css_class("compact");
        stack.add_named(&empty, Some("empty"));
        let library = adw::StatusPage::builder().title("Actions").build();
        stack.add_named(&library, Some("library"));
        {
            let stack = stack.clone();
            library_toggle.connect_toggled(move |t| {
                stack.set_visible_child_name(if t.is_active() { "library" } else { "empty" });
            });
        }
        let view = adw::ToolbarView::new();
        let header = adw::HeaderBar::builder()
            .show_title(false)
            .show_start_title_buttons(false)
            .build();
        view.add_top_bar(&header);
        view.set_content(Some(&stack));
        let _ = state;
        let _ = Change::Selection;
        Panel { root: view.upcast() }
    }
}
