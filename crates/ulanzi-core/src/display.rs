//! PC monitors for Display Brightness keys. A key keeps a monitor's name, because the numbers the
//! desktop gives monitors change between sessions.

use crate::model::{BrightnessOp, DisplayTarget};

/// The same monitor under two names: KDE puts the maker first ("Acme Electronics Acme TV") and ddcutil
/// gives the model alone ("Acme TV"), so a key still finds it on another desktop.
fn same_monitor(a: &str, b: &str) -> bool {
    a.ends_with(&format!(" {b}")) || b.ends_with(&format!(" {a}"))
}

/// The list positions in `labels` that `target` means, or `None` when its monitor isn't connected.
/// Monitors with the same name are told apart by their order; if some are gone, the last one left
/// stands in. An exact name wins over the same monitor under another name.
pub fn pick(labels: &[String], target: &DisplayTarget) -> Option<Vec<usize>> {
    match target {
        DisplayTarget::All => Some((0..labels.len()).collect()),
        DisplayTarget::Named { name, nth } => {
            let all = 0..labels.len();
            let mut same: Vec<usize> = all.clone().filter(|i| labels[*i] == *name).collect();
            if same.is_empty() {
                same = all.filter(|i| same_monitor(&labels[*i], name)).collect();
            }
            let at = same.get(*nth as usize).or(same.last())?;
            Some(vec![*at])
        }
    }
}

/// The level `op` asks for on a monitor at `level` of `max`.
pub fn next_level(level: i32, max: i32, op: BrightnessOp) -> i32 {
    let share = |percent: u8| (max as i64 * percent as i64 / 100) as i32;
    match op {
        BrightnessOp::Set { percent } => share(percent.min(100)),
        BrightnessOp::Up { step } => level.saturating_add(share(step)),
        BrightnessOp::Down { step } => level.saturating_sub(share(step)),
    }
    .clamp(0, max)
}

/// One line of the monitor list in a key's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayChoice {
    pub title: String,
    pub target: DisplayTarget,
}

/// The choices for a key on `saved`, with `labels` connected: All Displays, each monitor (numbered
/// when two share a name) and the saved one if it isn't connected, so opening a key never changes it.
pub fn choices(labels: &[String], saved: &DisplayTarget) -> Vec<DisplayChoice> {
    let mut out = vec![DisplayChoice {
        title: "All Displays".into(),
        target: DisplayTarget::All,
    }];
    for (i, label) in labels.iter().enumerate() {
        let nth = labels[..i].iter().filter(|l| *l == label).count();
        let twins = labels.iter().filter(|l| *l == label).count() > 1;
        out.push(DisplayChoice {
            title: if twins {
                format!("{label} {}", nth + 1)
            } else {
                label.clone()
            },
            target: if matches!(saved, DisplayTarget::Named { .. }) && pick(labels, saved) == Some(vec![i]) {
                saved.clone()
            } else {
                DisplayTarget::Named {
                    name: label.clone(),
                    nth: nth as u8,
                }
            },
        });
    }
    if !out.iter().any(|c| c.target == *saved)
        && let DisplayTarget::Named { name, .. } = saved
    {
        out.push(DisplayChoice {
            title: format!("{name} (not connected)"),
            target: saved.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    fn named(name: &str, nth: u8) -> DisplayTarget {
        DisplayTarget::Named {
            name: name.into(),
            nth,
        }
    }

    #[test]
    fn a_monitor_is_found_by_name_wherever_it_sits() {
        assert_eq!(pick(&labels(&["A", "LG"]), &named("LG", 0)), Some(vec![1]));
        assert_eq!(pick(&labels(&["LG", "A"]), &named("LG", 0)), Some(vec![0]));
        assert_eq!(pick(&labels(&["A"]), &named("LG", 0)), None);
        assert_eq!(pick(&labels(&["A", "B"]), &DisplayTarget::All), Some(vec![0, 1]));
    }

    #[test]
    fn twins_go_by_order_and_the_one_left_stands_in() {
        let two = labels(&["Dell", "LG", "Dell"]);
        assert_eq!(pick(&two, &named("Dell", 0)), Some(vec![0]));
        assert_eq!(pick(&two, &named("Dell", 1)), Some(vec![2]));
        assert_eq!(pick(&labels(&["Dell"]), &named("Dell", 1)), Some(vec![0]));
    }

    #[test]
    fn a_monitor_is_found_under_another_desktops_name() {
        let kde = labels(&["Acme Electronics Acme TV", "Globex Electric Company GX27"]);
        assert_eq!(pick(&kde, &named("GX27", 0)), Some(vec![1]));
        let ddc = labels(&["Acme TV", "GX27"]);
        assert_eq!(
            pick(&ddc, &named("Globex Electric Company GX27", 0)),
            Some(vec![1])
        );
        assert_eq!(pick(&ddc, &named("TV 2", 0)), None);
    }

    #[test]
    fn the_exact_name_wins_over_a_longer_one() {
        let both = labels(&["Big Acme TV", "Acme TV"]);
        assert_eq!(pick(&both, &named("Acme TV", 0)), Some(vec![1]));
    }

    #[test]
    fn opening_a_key_saved_under_another_name_keeps_it() {
        let saved = named("Globex Electric Company GX27", 0);
        let list = choices(&labels(&["GX27"]), &saved);
        assert_eq!(list.len(), 2);
        assert_eq!((list[1].title.as_str(), &list[1].target), ("GX27", &saved));
    }

    #[test]
    fn levels_stay_in_range() {
        let up = BrightnessOp::Up { step: 2 };
        let down = BrightnessOp::Down { step: 2 };
        assert_eq!(next_level(1000, 10000, up), 1200);
        assert_eq!(next_level(9900, 10000, up), 10000);
        assert_eq!(next_level(100, 10000, down), 0);
        assert_eq!(next_level(0, 10000, BrightnessOp::Set { percent: 50 }), 5000);
        assert_eq!(next_level(0, 10000, BrightnessOp::Set { percent: 200 }), 10000);
    }

    #[test]
    fn the_list_numbers_twins_and_keeps_a_missing_monitor() {
        let titles = |c: Vec<DisplayChoice>| c.into_iter().map(|c| c.title).collect::<Vec<_>>();
        assert_eq!(
            titles(choices(&labels(&["Dell", "LG", "Dell"]), &DisplayTarget::All)),
            ["All Displays", "Dell 1", "LG", "Dell 2"]
        );
        let list = choices(&labels(&["Dell"]), &named("LG", 0));
        assert_eq!(
            titles(list.clone()),
            ["All Displays", "Dell", "LG (not connected)"]
        );
        assert_eq!(list[2].target, named("LG", 0));
        let one = choices(&labels(&["Dell"]), &DisplayTarget::All);
        assert_eq!(one[1].target, named("Dell", 0));
    }
}
