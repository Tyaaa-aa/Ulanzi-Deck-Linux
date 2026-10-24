//! The wide screen's widget editor: the widgets in order (each with its options), rotation,
//! background and accent colour. The canvas previews the widget opened here with sample data.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use ulanzi_core::Color;
use ulanzi_core::model::{
    Background, ClockFace, IconSet, ImageSource, KeyLook, Rotation, ScreenConfig, Stat, Widget,
};
use ulanzi_core::render::screen::{MediaData, ScreenData, StatsData, TimerData, WeatherData};

use crate::state::{Change, State};
use crate::ui::action_editor::How;
use crate::ui::canvas::{key_texture, profile_dir};
use crate::ui::color_popover::color_button;

/// Applies a change to the profile's screen settings.
pub type EditScreen = Rc<dyn Fn(How, Box<dyn FnOnce(&mut ScreenConfig)>)>;

thread_local! {
    /// The widget the canvas previews (the one last opened in the editor).
    static PREVIEW: Cell<usize> = const { Cell::new(0) };
}

/// Which widget the canvas shows.
pub fn preview_index() -> usize {
    PREVIEW.with(Cell::get)
}

fn set_preview(state: &State, i: usize) {
    if PREVIEW.with(|p| p.replace(i)) != i {
        state.notify(Change::Preview);
    }
}

/// Move widget `i` by `delta` places; returns the new index (unchanged at the ends).
pub fn move_widget(list: &mut [Widget], i: usize, delta: isize) -> usize {
    let j = i as isize + delta;
    if j < 0 || j as usize >= list.len() || i >= list.len() {
        return i;
    }
    list.swap(i, j as usize);
    j as usize
}

/// One line about a widget's settings, for its row.
pub fn summary(w: &Widget) -> String {
    match w {
        Widget::Blank => "Nothing, just the background".into(),
        Widget::Clock {
            face,
            hour24,
            seconds,
            ..
        } => format!(
            "{}, {}{}",
            face.title(),
            if *hour24 { "24-hour" } else { "12-hour" },
            if *seconds { ", seconds" } else { "" }
        ),
        Widget::NowPlaying { .. } => "The song that's playing".into(),
        Widget::System { stats, .. } => {
            let names: Vec<&str> = stats.iter().map(|s| s.title()).collect();
            if names.is_empty() {
                "Nothing chosen".into()
            } else {
                names.join(", ")
            }
        }
        Widget::Timer { minutes, pomodoro } => {
            if *pomodoro {
                format!("Pomodoro, {minutes} min focus")
            } else {
                format!("{minutes} minutes")
            }
        }
        Widget::Weather { place, .. } if place.trim().is_empty() => "Choose a place".into(),
        Widget::Weather { place, .. } => place.clone(),
        Widget::Command { cmd, .. } if cmd.trim().is_empty() => "Enter a command".into(),
        Widget::Command { title, cmd, .. } => {
            if title.is_empty() {
                cmd.clone()
            } else {
                title.clone()
            }
        }
        Widget::Pictures { paths, .. } => match paths.len() {
            0 => "Add some pictures".into(),
            1 => "1 picture".into(),
            n => format!("{n} pictures"),
        },
        Widget::Obs { .. } => "Scene on air, recording and streaming time".into(),
    }
}

/// Sample values for the preview: plausible, so every widget looks like it will on the deck.
pub fn demo_data(now: chrono::NaiveDateTime, place: &str) -> ScreenData {
    let wave = |base: f32, amp: f32, k: f32| -> Vec<u8> {
        (0..60)
            .map(|i| (base + amp * ((i as f32) * k).sin()).clamp(0.0, 100.0) as u8)
            .collect()
    };
    ScreenData {
        now,
        stats: Some(StatsData {
            cpu: wave(32.0, 14.0, 0.35),
            ram: wave(58.0, 3.0, 0.1),
            gpu: wave(21.0, 12.0, 0.5),
            temp: Some(54),
            net_down: 2_400_000,
            net_up: 180_000,
        }),
        media: Some(MediaData {
            title: "Midnight Drive".into(),
            artist: "The Sample Band".into(),
            art: None,
            position_s: Some(83),
            length_s: Some(214),
            playing: true,
        }),
        timer: Some(TimerData {
            remaining_s: 18 * 60 + 42,
            total_s: 25 * 60,
            running: true,
            phase: None,
        }),
        weather: Some(WeatherData::Ok {
            temp: 21.0,
            code: 2,
            place: if place.trim().is_empty() {
                "Your Town".into()
            } else {
                place.trim().to_string()
            },
            high: 24.0,
            low: 14.0,
        }),
        command: Some("Output of your command\nshows here".into()),
        picture: None,
        obs: Some(ulanzi_core::obs::ObsLive {
            connected: true,
            program: "Gameplay".into(),
            recording: true,
            rec_secs: 754,
            stats: Some(ulanzi_core::obs::ObsStats {
                fps: 60,
                cpu: 4,
                dropped_permille: 1,
            }),
            ..Default::default()
        }),
    }
}

/// The widget as an icon (the same Lucide icons the deck uses).
fn widget_icon(state: &State, w: &Widget) -> gtk::Widget {
    let look = KeyLook {
        source: ImageSource::Icon {
            set: IconSet::Lucide,
            name: w.icon().into(),
        },
        ..KeyLook::default()
    };
    match key_texture(&look, &profile_dir(state)) {
        Some(t) => {
            let i = gtk::Image::from_paintable(Some(&t));
            i.set_pixel_size(32);
            i.set_valign(gtk::Align::Center);
            i.set_overflow(gtk::Overflow::Hidden);
            i.add_css_class("action-chip-img");
            i.upcast()
        }
        None => gtk::Image::from_icon_name("image-missing-symbolic").upcast(),
    }
}

/// Edit widget `i` in place.
fn edit_widget(edit: &EditScreen, how: How, i: usize, f: impl FnOnce(&mut Widget) + 'static) {
    edit(
        how,
        Box::new(move |s| {
            if let ScreenConfig::Custom { widgets, .. } = s
                && let Some(w) = widgets.get_mut(i)
            {
                f(w);
            }
        }),
    );
}

fn edit_list(edit: &EditScreen, f: impl FnOnce(&mut Vec<Widget>) + 'static) {
    edit(
        How::Structure,
        Box::new(move |s| {
            if let ScreenConfig::Custom { widgets, .. } = s {
                f(widgets);
            }
        }),
    );
}

fn switch(title: &str, subtitle: &str, on: bool, set: impl Fn(bool) + 'static) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).active(on).build();
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row.connect_active_notify(move |r| set(r.is_active()));
    row
}

fn spin(
    title: &str,
    (min, max, step): (f64, f64, f64),
    value: f64,
    set: impl Fn(f64) + 'static,
) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(title);
    row.set_value(value);
    row.connect_value_notify(move |r| set(r.value()));
    row
}

fn entry(title: &str, value: &str, set: impl Fn(String) + 'static) -> adw::EntryRow {
    let row = adw::EntryRow::builder().title(title).text(value).build();
    row.connect_changed(move |r| set(r.text().to_string()));
    row
}

/// Ask for pictures and import them sized for the screen; `done` gets their profile paths.
fn choose_pictures(
    state: &State,
    parent: &impl IsA<gtk::Widget>,
    many: bool,
    done: impl Fn(Vec<String>) + 'static,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    filter.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let fd = gtk::FileDialog::builder()
        .title(if many {
            "Choose Pictures"
        } else {
            "Choose a Picture"
        })
        .filters(&filters)
        .modal(true)
        .build();
    let window = parent.root().and_downcast::<gtk::Window>();
    let (state, done) = (state.clone(), Rc::new(done));
    let import = move |paths: Vec<PathBuf>| {
        let dir = profile_dir(&state);
        let (state, done) = (state.clone(), done.clone());
        crate::runtime::run(
            async move {
                tokio::task::spawn_blocking(move || {
                    paths
                        .iter()
                        .map(|p| ulanzi_core::import::import_screen_image(p, &dir))
                        .collect::<Vec<_>>()
                })
                .await
            },
            move |r| {
                let Ok(results) = r else { return };
                let mut ok = Vec::new();
                for r in results {
                    match r {
                        Ok(rel) => ok.push(rel),
                        Err(e) => crate::ui::toast(&state, &e.to_string()),
                    }
                }
                if !ok.is_empty() {
                    done(ok);
                }
            },
        );
    };
    if many {
        fd.open_multiple(window.as_ref(), None::<&gio::Cancellable>, move |res| {
            let Ok(files) = res else { return };
            let paths: Vec<PathBuf> = (0..files.n_items())
                .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                .filter_map(|f| f.path())
                .collect();
            import(paths);
        });
    } else {
        fd.open(window.as_ref(), None::<&gio::Cancellable>, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                import(vec![p]);
            }
        });
    }
}

/// The weather widget's place: search towns by name and pick one (town, region, country).
fn place_row(state: &State, edit: &EditScreen, i: usize, place: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title("Place")
        .subtitle(if place.is_empty() {
            "Choose a town or city"
        } else {
            place
        })
        .build();
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Town or city")
        .search_delay(350)
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let note = gtk::Label::builder()
        .label("Type at least two letters")
        .margin_top(8)
        .margin_bottom(8)
        .css_classes(["dim-label"])
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);
    column.append(&search);
    column.append(&note);
    column.append(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .propagate_natural_height(true)
            .max_content_height(320)
            .min_content_width(300)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    let popover = gtk::Popover::builder().child(&column).build();
    let button = gtk::MenuButton::builder()
        .label(if place.is_empty() {
            "Choose…"
        } else {
            "Change…"
        })
        .valign(gtk::Align::Center)
        .popover(&popover)
        .build();
    row.add_suffix(&button);
    let found: Rc<std::cell::RefCell<Vec<(String, f64, f64)>>> = Rc::default();
    // Only the newest search's answer counts.
    let generation = Rc::new(Cell::new(0u32));
    {
        let (state, list, note, found, generation) = (
            state.clone(),
            list.clone(),
            note.clone(),
            found.clone(),
            generation.clone(),
        );
        search.connect_search_changed(move |s| {
            let q = s.text().to_string();
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            found.borrow_mut().clear();
            if q.trim().chars().count() < 2 {
                note.set_label("Type at least two letters");
                note.set_visible(true);
                return;
            }
            let Some(c) = state.client.borrow().clone() else {
                note.set_label("The service isn’t running");
                return;
            };
            note.set_label("Searching…");
            note.set_visible(true);
            let g = generation.get() + 1;
            generation.set(g);
            let (list, note, found, generation) =
                (list.clone(), note.clone(), found.clone(), generation.clone());
            crate::runtime::run(async move { c.weather_places(&q).await }, move |r| {
                if generation.get() != g {
                    return;
                }
                match r {
                    Ok(places) if places.is_empty() => note.set_label("No town by that name"),
                    Ok(places) => {
                        note.set_visible(false);
                        for (label, _, _) in &places {
                            list.append(
                                &gtk::ListBoxRow::builder()
                                    .child(
                                        &gtk::Label::builder()
                                            .label(label)
                                            .xalign(0.0)
                                            .margin_top(4)
                                            .margin_bottom(4)
                                            .build(),
                                    )
                                    .build(),
                            );
                        }
                        *found.borrow_mut() = places;
                    }
                    Err(e) => note.set_label(&format!("Couldn’t search: {e}")),
                }
            });
        });
    }
    {
        let (edit, popover, found) = (edit.clone(), popover.downgrade(), found.clone());
        list.connect_row_activated(move |_, r| {
            let Some((label, la, lo)) = found.borrow().get(r.index().max(0) as usize).cloned() else {
                return;
            };
            if let Some(p) = popover.upgrade() {
                p.popdown();
            }
            edit_widget(&edit, How::Structure, i, move |w| {
                if let Widget::Weather { place, lat, lon, .. } = w {
                    *place = label;
                    *lat = Some(la);
                    *lon = Some(lo);
                }
            });
        });
    }
    {
        let search = search.clone();
        popover.connect_show(move |_| {
            search.grab_focus();
        });
    }
    row
}

/// The option rows of widget `i`.
fn option_rows(state: &State, edit: &EditScreen, i: usize, w: &Widget) -> Vec<gtk::Widget> {
    let mut rows: Vec<gtk::Widget> = Vec::new();
    match w.clone() {
        Widget::Blank | Widget::Obs { .. } => {}
        Widget::Clock {
            face,
            hour24,
            seconds,
            date,
        } => {
            let names: Vec<&str> = ClockFace::ALL.iter().map(|f| f.title()).collect();
            let combo = adw::ComboRow::builder()
                .title("Face")
                .model(&gtk::StringList::new(&names))
                .selected(ClockFace::ALL.iter().position(|f| *f == face).unwrap_or(0) as u32)
                .build();
            let e = edit.clone();
            combo.connect_selected_notify(move |r| {
                let f = ClockFace::ALL[(r.selected() as usize).min(3)];
                edit_widget(&e, How::Set, i, move |w| {
                    if let Widget::Clock { face, .. } = w {
                        *face = f;
                    }
                });
            });
            rows.push(combo.upcast());
            type Pick = fn(&mut Widget) -> Option<&mut bool>;
            let flags: [(&str, bool, Pick); 3] = [
                ("24-Hour Time", hour24, |w| match w {
                    Widget::Clock { hour24, .. } => Some(hour24),
                    _ => None,
                }),
                ("Show Seconds", seconds, |w| match w {
                    Widget::Clock { seconds, .. } => Some(seconds),
                    _ => None,
                }),
                ("Show Date", date, |w| match w {
                    Widget::Clock { date, .. } => Some(date),
                    _ => None,
                }),
            ];
            for (title, on, pick) in flags {
                let e = edit.clone();
                rows.push(
                    switch(title, "", on, move |v| {
                        edit_widget(&e, How::Set, i, move |w| {
                            if let Some(b) = pick(w) {
                                *b = v;
                            }
                        })
                    })
                    .upcast(),
                );
            }
        }
        Widget::NowPlaying {
            progress,
            blur_background,
        } => {
            let e = edit.clone();
            rows.push(
                switch("Progress Bar", "", progress, move |v| {
                    edit_widget(&e, How::Set, i, move |w| {
                        if let Widget::NowPlaying { progress, .. } = w {
                            *progress = v;
                        }
                    })
                })
                .upcast(),
            );
            let e = edit.clone();
            rows.push(
                switch(
                    "Album Art Background",
                    "A blurred copy of the cover behind the text",
                    blur_background,
                    move |v| {
                        edit_widget(&e, How::Set, i, move |w| {
                            if let Widget::NowPlaying { blur_background, .. } = w {
                                *blur_background = v;
                            }
                        })
                    },
                )
                .upcast(),
            );
        }
        Widget::System { stats, graphs } => {
            for st in Stat::ALL {
                let e = edit.clone();
                rows.push(
                    switch(st.title(), "", stats.contains(&st), move |v| {
                        edit_widget(&e, How::Set, i, move |w| {
                            if let Widget::System { stats, .. } = w {
                                stats.retain(|s| *s != st);
                                if v {
                                    stats.push(st);
                                    stats.sort_by_key(|s| Stat::ALL.iter().position(|a| a == s));
                                }
                            }
                        })
                    })
                    .upcast(),
                );
            }
            let e = edit.clone();
            rows.push(
                switch("Graphs", "The last minute under each value", graphs, move |v| {
                    edit_widget(&e, How::Set, i, move |w| {
                        if let Widget::System { graphs, .. } = w {
                            *graphs = v;
                        }
                    })
                })
                .upcast(),
            );
            let note = adw::ActionRow::builder()
                .subtitle("Up to four values fit on the screen.")
                .css_classes(["dim-label"])
                .build();
            rows.push(note.upcast());
        }
        Widget::Timer { minutes, pomodoro } => {
            let e = edit.clone();
            rows.push(
                spin("Minutes", (1.0, 180.0, 1.0), minutes as f64, move |v| {
                    edit_widget(&e, How::Typing("timer-min"), i, move |w| {
                        if let Widget::Timer { minutes, .. } = w {
                            *minutes = v as u32;
                        }
                    })
                })
                .upcast(),
            );
            let e = edit.clone();
            rows.push(
                switch(
                    "Pomodoro",
                    "Focus, then a 5-minute break, over and over",
                    pomodoro,
                    move |v| {
                        edit_widget(&e, How::Set, i, move |w| {
                            if let Widget::Timer { pomodoro, .. } = w {
                                *pomodoro = v;
                            }
                        })
                    },
                )
                .upcast(),
            );
            rows.push(
                adw::ActionRow::builder()
                    .subtitle("Tap the screen to start or pause. The Timer actions in the library control it from a key.")
                    .css_classes(["dim-label"])
                    .build()
                    .upcast(),
            );
        }
        Widget::Weather {
            place, fahrenheit, ..
        } => {
            rows.push(place_row(state, edit, i, &place).upcast());
            let e = edit.clone();
            rows.push(
                switch("Fahrenheit", "", fahrenheit, move |v| {
                    edit_widget(&e, How::Set, i, move |w| {
                        if let Widget::Weather { fahrenheit, .. } = w {
                            *fahrenheit = v;
                        }
                    })
                })
                .upcast(),
            );
            rows.push(
                adw::ActionRow::builder()
                    .subtitle("Forecast from Open-Meteo, updated every 15 minutes.")
                    .css_classes(["dim-label"])
                    .build()
                    .upcast(),
            );
        }
        Widget::Command { cmd, every_s, title } => {
            let e = edit.clone();
            rows.push(
                entry("Title", &title, move |t| {
                    edit_widget(&e, How::Typing("cmd-title"), i, move |w| {
                        if let Widget::Command { title, .. } = w {
                            *title = t;
                        }
                    })
                })
                .upcast(),
            );
            let e = edit.clone();
            rows.push(
                entry("Command", &cmd, move |t| {
                    edit_widget(&e, How::Typing("cmd-cmd"), i, move |w| {
                        if let Widget::Command { cmd, .. } = w {
                            *cmd = t;
                        }
                    })
                })
                .upcast(),
            );
            let e = edit.clone();
            rows.push(
                spin(
                    "Run Every (Seconds)",
                    (5.0, 3600.0, 5.0),
                    every_s as f64,
                    move |v| {
                        edit_widget(&e, How::Typing("cmd-every"), i, move |w| {
                            if let Widget::Command { every_s, .. } = w {
                                *every_s = v as u32;
                            }
                        })
                    },
                )
                .upcast(),
            );
            rows.push(
                adw::ActionRow::builder()
                    .subtitle("The first two lines of its output show. It is stopped after 3 seconds.")
                    .css_classes(["dim-label"])
                    .build()
                    .upcast(),
            );
        }
        Widget::Pictures { paths, every_s } => {
            for (n, p) in paths.iter().enumerate() {
                let row = adw::ActionRow::builder()
                    .title(format!("Picture {}", n + 1))
                    .build();
                let full = profile_dir(state).join(p);
                if let Ok(t) = gtk::gdk::Texture::from_filename(&full) {
                    let img = gtk::Picture::for_paintable(&t);
                    img.set_size_request(70, 30);
                    img.set_content_fit(gtk::ContentFit::Cover);
                    img.add_css_class("card");
                    row.add_prefix(&img);
                }
                let remove = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .tooltip_text("Remove")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                let e = edit.clone();
                remove.connect_clicked(move |_| {
                    restructure(&e, i, move |w| {
                        if let Widget::Pictures { paths, .. } = w
                            && n < paths.len()
                        {
                            paths.remove(n);
                        }
                    })
                });
                row.add_suffix(&remove);
                rows.push(row.upcast());
            }
            let add = adw::ButtonRow::builder()
                .title("Add Pictures…")
                .start_icon_name("list-add-symbolic")
                .build();
            {
                let (state, e) = (state.clone(), edit.clone());
                add.connect_activated(move |b| {
                    let e = e.clone();
                    choose_pictures(&state, b, true, move |new| {
                        restructure(&e, i, move |w| {
                            if let Widget::Pictures { paths, .. } = w {
                                paths.extend(new);
                            }
                        })
                    });
                });
            }
            rows.push(add.upcast());
            let e = edit.clone();
            rows.push(
                spin(
                    "Next Picture After (Seconds)",
                    (2.0, 3600.0, 1.0),
                    every_s as f64,
                    move |v| {
                        edit_widget(&e, How::Typing("pic-every"), i, move |w| {
                            if let Widget::Pictures { every_s, .. } = w {
                                *every_s = v as u32;
                            }
                        })
                    },
                )
                .upcast(),
            );
        }
    }

    if let Widget::Obs { stats } = w {
        let e = edit.clone();
        rows.push(
            switch(
                "Show Stats",
                "Frame rate, CPU and dropped frames",
                *stats,
                move |v| {
                    edit_widget(&e, How::Set, i, move |w| {
                        if let Widget::Obs { stats } = w {
                            *stats = v;
                        }
                    })
                },
            )
            .upcast(),
        );
    }

    /// A change that adds or removes rows.
    fn restructure(e: &EditScreen, i: usize, f: impl FnOnce(&mut Widget) + 'static) {
        edit_widget(e, How::Structure, i, f);
    }
    rows
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build()
}

/// The "Add Widget" menu.
fn add_menu(state: &State, edit: &EditScreen) -> gtk::MenuButton {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let popover = gtk::Popover::builder().child(&list).build();
    let kinds = Widget::all_kinds();
    for w in &kinds {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        let icon = widget_icon(state, w);
        icon.set_size_request(24, 24);
        row.append(&icon);
        row.append(&gtk::Label::builder().label(w.title()).xalign(0.0).build());
        list.append(&gtk::ListBoxRow::builder().child(&row).activatable(true).build());
    }
    {
        // Act on the row's position. (Calling the row's activate() here would ask the list to
        // activate it again, over and over, until the stack runs out.)
        let (edit, popover, state) = (edit.clone(), popover.clone(), state.clone());
        list.connect_row_activated(move |_, r| {
            let Some(w) = kinds.get(r.index().max(0) as usize).cloned() else {
                return;
            };
            popover.popdown();
            let n = Rc::new(Cell::new(0usize));
            let n2 = n.clone();
            edit_list(&edit, move |list| {
                // A lone blank is a placeholder: replace it.
                if list.len() == 1 && list[0] == Widget::Blank {
                    list.clear();
                }
                list.push(w);
                n2.set(list.len() - 1);
            });
            let i = n.get();
            state.expanded.borrow_mut().insert(format!("screen/widget{i}"));
            set_preview(&state, i);
        });
    }
    gtk::MenuButton::builder()
        .label("Add Widget")
        .popover(&popover)
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .build()
}

/// The editor's groups for a `Custom` screen.
pub fn groups(state: &State, screen: &ScreenConfig, edit: EditScreen) -> Vec<adw::PreferencesGroup> {
    let ScreenConfig::Custom {
        widgets,
        rotate,
        background,
        accent,
        media,
        ..
    } = screen
    else {
        return Vec::new();
    };
    let mut out = Vec::new();

    let list = adw::PreferencesGroup::builder()
        .title("Widgets")
        .description(if widgets.len() > 1 {
            "Shown one after another, as set in Rotation below."
        } else {
            "Add more to switch between them."
        })
        .header_suffix(&add_menu(state, &edit))
        .build();
    let n = widgets.len();
    for (i, w) in widgets.iter().enumerate() {
        let exp = adw::ExpanderRow::builder()
            .title(w.title())
            .subtitle(if i == 0 && n > 1 {
                format!("Default · {}", summary(w))
            } else {
                summary(w)
            })
            .build();
        exp.add_prefix(&widget_icon(state, w));
        state.remember_expanded(&exp, &format!("screen/widget{i}"));
        {
            let state = state.clone();
            exp.connect_expanded_notify(move |e| {
                if e.is_expanded() {
                    set_preview(&state, i);
                }
            });
        }
        let up = icon_button("go-up-symbolic", "Move Up");
        up.set_sensitive(i > 0);
        let down = icon_button("go-down-symbolic", "Move Down");
        down.set_sensitive(i + 1 < n);
        let remove = icon_button("user-trash-symbolic", "Remove Widget");
        for (b, delta) in [(&up, -1isize), (&down, 1)] {
            let (edit, state) = (edit.clone(), state.clone());
            b.connect_clicked(move |_| {
                let j = Rc::new(Cell::new(i));
                let j2 = j.clone();
                edit_list(&edit, move |list| j2.set(move_widget(list, i, delta)));
                set_preview(&state, j.get());
            });
        }
        {
            let (edit, state) = (edit.clone(), state.clone());
            remove.connect_clicked(move |_| {
                edit_list(&edit, move |list| {
                    if i < list.len() {
                        list.remove(i);
                    }
                    if list.is_empty() {
                        list.push(Widget::Blank);
                    }
                });
                set_preview(&state, i.saturating_sub(1));
            });
        }
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        if i > 0 {
            // The first widget is the default: the one the screen starts on and returns to.
            let star = icon_button("non-starred-symbolic", "Make Default");
            let (edit, state) = (edit.clone(), state.clone());
            star.connect_clicked(move |_| {
                edit_list(&edit, move |list| {
                    if i < list.len() {
                        let w = list.remove(i);
                        list.insert(0, w);
                    }
                });
                set_preview(&state, 0);
            });
            buttons.append(&star);
        }
        if n > 1 {
            buttons.append(&up);
            buttons.append(&down);
        }
        buttons.append(&remove);
        exp.add_suffix(&buttons);
        let rows = option_rows(state, &edit, i, w);
        exp.set_enable_expansion(!rows.is_empty());
        for r in rows {
            exp.add_row(&r);
        }
        list.add(&exp);
    }
    out.push(list);

    // Rotation, background, accent.
    let look = adw::PreferencesGroup::builder().title("Look").build();
    if widgets.len() > 1 {
        let rot = adw::ComboRow::builder()
            .title("Rotation")
            .model(&gtk::StringList::new(&[
                "Stay on the Default",
                "Every Few Seconds",
                "Next One When Tapped",
            ]))
            .selected(match rotate {
                Rotation::Off => 0,
                Rotation::Every { .. } => 1,
                Rotation::OnPress => 2,
            })
            .build();
        let e = edit.clone();
        rot.connect_selected_notify(move |r| {
            let new = match r.selected() {
                1 => Rotation::Every { secs: 10 },
                2 => Rotation::OnPress,
                _ => Rotation::Off,
            };
            e(
                How::Structure,
                Box::new(move |s| {
                    if let ScreenConfig::Custom { rotate, .. } = s {
                        *rotate = new;
                    }
                }),
            );
        });
        look.add(&rot);
        if let Rotation::Every { secs } = rotate {
            let e = edit.clone();
            look.add(&spin(
                "Seconds per Widget",
                (2.0, 3600.0, 1.0),
                *secs as f64,
                move |v| {
                    e(
                        How::Typing("rotate-secs"),
                        Box::new(move |s| {
                            if let ScreenConfig::Custom { rotate, .. } = s {
                                *rotate = Rotation::Every { secs: v as u32 };
                            }
                        }),
                    );
                },
            ));
        }
    }

    // Music takes the screen over, then the widgets come back.
    let takeover = adw::SwitchRow::builder()
        .title("Now Playing While Music Plays")
        .subtitle("Shows the song, then goes back to the widgets after it stops")
        .active(media.on)
        .build();
    {
        let (e, m) = (edit.clone(), *media);
        takeover.connect_active_notify(move |r| {
            let on = r.is_active();
            e(
                How::Structure,
                Box::new(move |s| {
                    if let ScreenConfig::Custom { media, .. } = s {
                        *media = ulanzi_core::model::MediaTakeover { on, ..m };
                    }
                }),
            );
        });
    }
    look.add(&takeover);
    if media.on {
        let e = edit.clone();
        look.add(&spin(
            "Back to the Widgets After (Seconds)",
            (1.0, 600.0, 1.0),
            media.back_after_s as f64,
            move |v| {
                e(
                    How::Typing("media-back"),
                    Box::new(move |s| {
                        if let ScreenConfig::Custom { media, .. } = s {
                            media.back_after_s = v as u32;
                        }
                    }),
                );
            },
        ));
    }

    let bg = adw::ComboRow::builder()
        .title("Background")
        .model(&gtk::StringList::new(&["Dark", "Colour", "Gradient", "Picture"]))
        .selected(match background {
            Background::Theme => 0,
            Background::Solid { .. } => 1,
            Background::Gradient { .. } => 2,
            Background::Picture { .. } => 3,
        })
        .build();
    {
        let (e, cur) = (edit.clone(), *accent);
        bg.connect_selected_notify(move |r| {
            let new = match r.selected() {
                1 => Background::Solid {
                    color: Color::rgb(0x24, 0x1f, 0x31),
                },
                2 => Background::Gradient {
                    from: Color::rgb(0x1c, 0x1c, 0x2e),
                    to: cur,
                },
                3 => Background::Picture { path: String::new() },
                _ => Background::Theme,
            };
            set_background(&e, How::Structure, new);
        });
    }
    look.add(&bg);
    match background.clone() {
        Background::Theme => {}
        Background::Solid { color } => {
            let row = adw::ActionRow::builder().title("Colour").build();
            let e = edit.clone();
            row.add_suffix(&color_button(
                Some(color),
                |_| {},
                move |c| set_background(&e, How::Set, Background::Solid { color: c }),
            ));
            look.add(&row);
        }
        Background::Gradient { from, to } => {
            let row_from = adw::ActionRow::builder().title("Top Left").build();
            let e = edit.clone();
            row_from.add_suffix(&color_button(
                Some(from),
                |_| {},
                move |c| set_background(&e, How::Structure, Background::Gradient { from: c, to }),
            ));
            look.add(&row_from);
            let row_to = adw::ActionRow::builder().title("Bottom Right").build();
            let e = edit.clone();
            row_to.add_suffix(&color_button(
                Some(to),
                |_| {},
                move |c| set_background(&e, How::Structure, Background::Gradient { from, to: c }),
            ));
            look.add(&row_to);
        }
        Background::Picture { path } => {
            let row = adw::ActionRow::builder()
                .title("Picture")
                .subtitle(if path.is_empty() {
                    "None chosen yet"
                } else {
                    "Dimmed a little so text stays readable"
                })
                .build();
            let choose = gtk::Button::builder()
                .label(if path.is_empty() { "Choose…" } else { "Change…" })
                .valign(gtk::Align::Center)
                .build();
            {
                let (state, e) = (state.clone(), edit.clone());
                choose.connect_clicked(move |b| {
                    let e = e.clone();
                    choose_pictures(&state, b, false, move |mut new| {
                        if let Some(p) = new.pop() {
                            set_background(&e, How::Structure, Background::Picture { path: p });
                        }
                    });
                });
            }
            row.add_suffix(&choose);
            look.add(&row);
        }
    }

    let accent_row = adw::ActionRow::builder()
        .title("Accent Colour")
        .subtitle("Clock hands, bars and graphs")
        .build();
    {
        let e = edit.clone();
        accent_row.add_suffix(&color_button(
            Some(*accent),
            |_| {},
            move |c| {
                e(
                    How::Set,
                    Box::new(move |s| {
                        if let ScreenConfig::Custom { accent, .. } = s {
                            *accent = c;
                        }
                    }),
                );
            },
        ));
    }
    look.add(&accent_row);
    out.push(look);
    // Keep the preview on an existing widget.
    if preview_index() >= widgets.len() {
        PREVIEW.with(|p| p.set(0));
    }
    out
}

fn set_background(edit: &EditScreen, how: How, new: Background) {
    edit(
        how,
        Box::new(move |s| {
            if let ScreenConfig::Custom { background, .. } = s {
                *background = new;
            }
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_data_is_plausible() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 10, 23)
            .and_then(|d| d.and_hms_opt(14, 5, 0))
            .expect("date");
        let d = demo_data(now, "  Oslo ");
        let s = d.stats.expect("stats");
        assert_eq!(s.cpu.len(), 60);
        assert!(s.cpu.iter().all(|v| *v <= 100));
        let t = d.timer.expect("timer");
        assert!(t.remaining_s <= t.total_s);
        let m = d.media.expect("media");
        assert!(m.position_s < m.length_s);
        assert!(matches!(d.weather, Some(WeatherData::Ok { ref place, .. }) if place == "Oslo"));
        assert!(
            matches!(demo_data(now, "").weather, Some(WeatherData::Ok { ref place, .. }) if place == "Your Town")
        );
    }

    #[test]
    fn widgets_move_within_bounds() {
        let mut l = vec![Widget::Blank, Widget::clock(), Widget::now_playing()];
        assert_eq!(move_widget(&mut l, 0, -1), 0);
        assert_eq!(move_widget(&mut l, 0, 1), 1);
        assert_eq!(l[1], Widget::Blank);
        assert_eq!(move_widget(&mut l, 2, 1), 2);
        assert_eq!(move_widget(&mut l, 9, -1), 9);
    }

    #[test]
    fn summaries_read_well() {
        assert_eq!(
            summary(&Widget::Timer {
                minutes: 25,
                pomodoro: true
            }),
            "Pomodoro, 25 min focus"
        );
        assert_eq!(
            summary(&Widget::Weather {
                place: " ".into(),
                fahrenheit: false,
                lat: None,
                lon: None
            }),
            "Choose a place"
        );
        assert_eq!(
            summary(&Widget::System {
                stats: vec![Stat::Cpu, Stat::Net],
                graphs: true
            }),
            "CPU, Network"
        );
    }

    #[test]
    fn adding_a_widget_from_the_menu_adds_it() {
        crate::ui::with_gtk(|| {
            let mut p = ulanzi_core::model::Profile::new_default("T", ulanzi_core::model::Model::D200X);
            p.screen = ScreenConfig::custom(vec![Widget::now_playing()], None, None);
            let state = crate::state::AppState::new(p);
            let edit: EditScreen = {
                let st = state.clone();
                Rc::new(move |_how, f| {
                    st.edit(|e| {
                        let mut s = e.profile().screen.clone();
                        f(&mut s);
                        e.set_screen(s)
                    })
                })
            };
            let menu = add_menu(&state, &edit);
            let list = menu
                .popover()
                .and_then(|p| p.child())
                .and_downcast::<gtk::ListBox>()
                .expect("menu list");
            let row = list.row_at_index(0).expect("first kind");
            // What a click does.
            list.emit_by_name::<()>("row-activated", &[&row]);
            let ScreenConfig::Custom { widgets, .. } = state.editor.borrow().profile().screen.clone() else {
                panic!("not widgets");
            };
            assert_eq!(widgets.len(), 2);
        });
    }
}
