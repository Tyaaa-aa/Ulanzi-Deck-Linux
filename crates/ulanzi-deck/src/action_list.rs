//! The action list's model, shared by the side panel's library and the key editor's action picker:
//! the category switcher (All · built-in categories · one per plugin), search, and the groups with
//! their entries. Pure Rust (no GTK) so it can be unit-tested.
//!
//! Search matches every word of the query against an action's title, its group (category) name
//! and, for plugin actions, the plugin's name, case-insensitively. With All chosen it searches
//! everything; with a category chosen it searches that category and counts the matches elsewhere
//! ([`View::elsewhere`]), so the list can offer "Show N more in All" instead of looking empty.

use std::collections::HashSet;

use ulanzi_core::catalog::{self, Category};
use ulanzi_core::model::Action;

use crate::plugins::{Plugins, defaults};

/// The id of the All category.
pub const ALL: &str = "all";

/// One button of the category switcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: String,
    pub label: String,
}

/// One action in the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub title: String,
    /// The plugin's name, for plugin actions.
    pub plugin: Option<String>,
    /// The plugin's icon for this action (the action's own, else the plugin's), as the manifest
    /// names it. Built-in actions show their default key look instead.
    pub icon: Option<String>,
    /// What a click assigns and a drag carries: a fresh action with its defaults.
    pub action: Action,
}

/// A collapsible section: one built-in category or one plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// The same as its category's [`Tab::id`]; also the key its collapsed state is kept under.
    pub id: String,
    pub title: String,
    /// The plugin's icon, for plugin sections.
    pub icon: Option<String>,
    pub plugin: bool,
    /// A–Z by title.
    pub entries: Vec<Entry>,
}

/// What the list shows for a category and a query.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    pub groups: Vec<Group>,
    /// Section headers are shown (All); a single category shows its entries without one.
    pub headers: bool,
    /// Matches outside the chosen category (only counted while searching a category).
    pub elsewhere: usize,
}

impl View {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The first entry shown (Enter in the search field picks it).
    pub fn first(&self) -> Option<&Entry> {
        self.groups.first().and_then(|g| g.entries.first())
    }

    pub fn len(&self) -> usize {
        self.groups.iter().map(|g| g.entries.len()).sum()
    }
}

/// Every action the list can show, grouped and sorted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Catalog {
    groups: Vec<Group>,
}

fn category_id(c: Category) -> String {
    format!("cat:{}", format!("{c:?}").to_lowercase())
}

fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.title.cmp(&b.title))
    });
}

/// The query's words, lower case; empty for a blank query.
fn words(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn entry_matches(group: &Group, e: &Entry, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {}",
        e.title,
        group.title,
        e.plugin.as_deref().unwrap_or("")
    )
    .to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// `group` with only the entries matching `words`; `None` when none does.
fn filtered(group: &Group, words: &[String]) -> Option<Group> {
    let entries: Vec<Entry> = group
        .entries
        .iter()
        .filter(|e| entry_matches(group, e, words))
        .cloned()
        .collect();
    (!entries.is_empty()).then(|| Group {
        entries,
        ..group.clone()
    })
}

impl Catalog {
    /// The built-in actions (without Multi Action unless `include_multi`) and the actions of every
    /// plugin that can be loaded.
    pub fn new(plugins: &Plugins, include_multi: bool) -> Catalog {
        let mut groups: Vec<Group> = Vec::new();
        for cat in Category::ALL {
            let mut entries: Vec<Entry> = catalog::catalog()
                .iter()
                .filter(|s| s.category == cat && (include_multi || s.id != "multi"))
                .map(|s| Entry {
                    title: s.title.to_string(),
                    plugin: None,
                    icon: None,
                    action: (s.make)(),
                })
                .collect();
            if entries.is_empty() {
                continue;
            }
            sort_entries(&mut entries);
            groups.push(Group {
                id: category_id(cat),
                title: cat.title().to_string(),
                icon: None,
                plugin: false,
                entries,
            });
        }
        let by_plugin = plugins.actions_by_plugin();
        for (id, name, actions) in &by_plugin {
            let plugin_icon = plugins.manifests.get(id).and_then(|m| m.icon.clone());
            // A name shared with a built-in category or another plugin gets told apart.
            let builtin_clash = Category::ALL.iter().any(|c| c.title().eq_ignore_ascii_case(name));
            let plugin_clash = by_plugin
                .iter()
                .any(|(other, n, _)| other != id && n.eq_ignore_ascii_case(name));
            let title = if plugin_clash {
                format!("{name} ({id})")
            } else if builtin_clash {
                format!("{name} (plugin)")
            } else {
                name.clone()
            };
            let mut entries: Vec<Entry> = actions
                .iter()
                .map(|a| Entry {
                    title: a.name.clone(),
                    plugin: Some(name.clone()),
                    icon: a.icon.clone().or_else(|| plugin_icon.clone()),
                    action: Action::Plugin {
                        plugin: id.clone(),
                        action: a.id.clone(),
                        params: defaults(a),
                    },
                })
                .collect();
            sort_entries(&mut entries);
            groups.push(Group {
                id: format!("plugin:{id}"),
                title,
                icon: plugin_icon,
                plugin: true,
                entries,
            });
        }
        Catalog { groups }
    }

    /// All, the built-in categories in catalog order, then one per plugin by name.
    pub fn tabs(&self) -> Vec<Tab> {
        std::iter::once(Tab {
            id: ALL.to_string(),
            label: "All".to_string(),
        })
        .chain(self.groups.iter().map(|g| Tab {
            id: g.id.clone(),
            label: g.title.clone(),
        }))
        .collect()
    }

    /// `wanted` when it is still a category, else All (a remembered plugin was removed).
    pub fn resolve(&self, wanted: &str) -> String {
        if self.groups.iter().any(|g| g.id == wanted) {
            wanted.to_string()
        } else {
            ALL.to_string()
        }
    }

    /// The groups to show for `category` and `query`.
    pub fn view(&self, category: &str, query: &str) -> View {
        let words = words(query);
        let category = self.resolve(category);
        if category == ALL {
            return View {
                groups: self.groups.iter().filter_map(|g| filtered(g, &words)).collect(),
                headers: true,
                elsewhere: 0,
            };
        }
        let groups: Vec<Group> = self
            .groups
            .iter()
            .filter(|g| g.id == category)
            .filter_map(|g| filtered(g, &words))
            .collect();
        let elsewhere = if words.is_empty() {
            0
        } else {
            self.groups
                .iter()
                .filter(|g| g.id != category)
                .filter_map(|g| filtered(g, &words))
                .map(|g| g.entries.len())
                .sum()
        };
        View {
            groups,
            headers: false,
            elsewhere,
        }
    }
}

/// Whether a group is open: always while searching, else unless the user closed it.
pub fn expanded(collapsed: &HashSet<String>, group: &str, query: &str) -> bool {
    !query.trim().is_empty() || !collapsed.contains(group)
}

/// The words for an empty list: (title, description).
pub fn empty_text(query: &str, category: &str) -> (String, Option<String>) {
    let query = query.trim();
    if query.is_empty() {
        return ("No actions".to_string(), None);
    }
    let detail = if category == "All" {
        "Try another word".to_string()
    } else {
        format!("Nothing in {category} matches “{query}”")
    };
    ("No matching actions".to_string(), Some(detail))
}

/// The button offering the matches in other categories; `None` when there are none.
pub fn elsewhere_label(n: usize) -> Option<String> {
    (n > 0).then(|| format!("Show {n} more in All"))
}

/// What the list remembers while the app runs: the chosen category and the closed sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    pub category: String,
    pub collapsed: HashSet<String>,
}

impl Default for Memory {
    fn default() -> Self {
        Memory {
            category: ALL.to_string(),
            collapsed: HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginRow;
    use ulanzi_core::plugin::Manifest;

    fn manifest(
        id: &str,
        name: &str,
        icon: Option<&str>,
        actions: &[(&str, &str, Option<&str>)],
    ) -> Manifest {
        let mut text = format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1.0.0\"\ncontract = \"1.0\"\n");
        if let Some(i) = icon {
            text.push_str(&format!("icon = \"{i}\"\n"));
        }
        for (aid, aname, aicon) in actions {
            text.push_str(&format!("[[actions]]\nid = \"{aid}\"\nname = \"{aname}\"\n"));
            if let Some(i) = aicon {
                text.push_str(&format!("icon = \"{i}\"\n"));
            }
        }
        Manifest::parse(&text).expect("manifest")
    }

    fn row(id: &str, name: &str, status: &str) -> PluginRow {
        PluginRow {
            id: id.into(),
            name: name.into(),
            version: "1.0.0".into(),
            status: status.into(),
            detail: String::new(),
        }
    }

    fn plugins(ms: Vec<Manifest>, rows: Vec<PluginRow>) -> Plugins {
        Plugins {
            loaded: true,
            list: rows,
            manifests: ms.into_iter().map(|m| (m.id.clone(), m)).collect(),
        }
    }

    fn sample() -> Plugins {
        plugins(
            vec![
                manifest(
                    "zzz",
                    "Home Assistant",
                    Some("mdi:home-assistant"),
                    &[
                        ("toggle", "Toggle an entity", Some("mdi:toggle-switch")),
                        ("call", "Call a service", None),
                        ("scene", "activate a scene", None),
                    ],
                ),
                manifest("obs", "OBS Studio", None, &[("rec", "Toggle recording", None)]),
                manifest("bad", "Broken", None, &[("x", "X", None)]),
                manifest("quiet", "No Actions", None, &[]),
            ],
            vec![
                row("zzz", "Home Assistant", "running"),
                row("obs", "OBS Studio", "problem"),
                row("bad", "Broken", "incompatible"),
                row("quiet", "No Actions", "running"),
            ],
        )
    }

    fn titles(v: &View) -> Vec<(String, Vec<String>)> {
        v.groups
            .iter()
            .map(|g| {
                (
                    g.title.clone(),
                    g.entries.iter().map(|e| e.title.clone()).collect(),
                )
            })
            .collect()
    }

    fn builtin_count(include_multi: bool) -> usize {
        catalog::catalog()
            .iter()
            .filter(|s| include_multi || s.id != "multi")
            .count()
    }

    #[test]
    fn tabs_are_all_then_categories_then_plugins() {
        let c = Catalog::new(&sample(), true);
        let tabs = c.tabs();
        let labels: Vec<&str> = tabs.iter().map(|t| t.label.as_str()).collect();
        let mut want = vec!["All"];
        want.extend(Category::ALL.iter().map(|c| c.title()));
        // The plugin shares its name with the built-in OBS category: it says it is a plugin.
        want.extend(["Home Assistant", "OBS Studio (plugin)"]);
        assert_eq!(labels, want);
        assert_eq!(tabs[0].id, ALL);
        let ids: HashSet<&str> = tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids.len(), tabs.len(), "tab ids are unique");
    }

    #[test]
    fn no_plugins_means_only_builtin_tabs() {
        let c = Catalog::new(&Plugins::default(), true);
        assert_eq!(c.tabs().len(), 1 + Category::ALL.len());
    }

    #[test]
    fn plugins_with_the_same_name_are_told_apart() {
        let p = plugins(
            vec![
                manifest("ha", "Lights", None, &[("a", "A", None)]),
                manifest("hue", "Lights", None, &[("b", "B", None)]),
            ],
            vec![],
        );
        let tabs = Catalog::new(&p, true).tabs();
        let labels: Vec<&str> = tabs
            .iter()
            .skip(1 + Category::ALL.len())
            .map(|t| t.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Lights (ha)", "Lights (hue)"]);
    }

    #[test]
    fn all_shows_every_group_with_headers_entries_a_to_z() {
        let c = Catalog::new(&sample(), true);
        let v = c.view(ALL, "");
        assert!(v.headers);
        assert_eq!(v.elsewhere, 0);
        assert_eq!(v.len(), builtin_count(true) + 4);
        let names: Vec<&str> = v.groups.iter().map(|g| g.title.as_str()).collect();
        let mut want: Vec<&str> = Category::ALL.iter().map(|c| c.title()).collect();
        want.extend(["Home Assistant", "OBS Studio (plugin)"]);
        assert_eq!(names, want);
        for g in &v.groups {
            let t: Vec<String> = g.entries.iter().map(|e| e.title.to_lowercase()).collect();
            let mut sorted = t.clone();
            sorted.sort();
            assert_eq!(t, sorted, "{} is A–Z", g.title);
        }
        // Lower-case titles sort among the others, not after them.
        assert_eq!(
            titles(&c.view("plugin:zzz", ""))[0].1,
            vec!["activate a scene", "Call a service", "Toggle an entity"]
        );
    }

    #[test]
    fn a_category_shows_only_its_group_without_a_header() {
        let c = Catalog::new(&sample(), true);
        let v = c.view(&c.tabs()[1].id, "");
        assert!(!v.headers);
        assert_eq!(v.groups.len(), 1);
        assert_eq!(v.groups[0].title, Category::Navigation.title());
        let n = catalog::catalog()
            .iter()
            .filter(|s| s.category == Category::Navigation)
            .count();
        assert_eq!(v.len(), n);
        let v = c.view("plugin:obs", "");
        assert_eq!(
            titles(&v),
            vec![(
                "OBS Studio (plugin)".to_string(),
                vec!["Toggle recording".to_string()]
            )]
        );
        assert!(v.groups[0].plugin);
    }

    #[test]
    fn broken_plugins_and_plugins_without_actions_are_left_out() {
        let c = Catalog::new(&sample(), true);
        let labels: Vec<String> = c.tabs().into_iter().map(|t| t.label).collect();
        assert!(!labels.iter().any(|l| l == "Broken" || l == "No Actions"));
        assert!(c.view(ALL, "broken").is_empty());
    }

    #[test]
    fn multi_action_only_when_allowed() {
        let with = Catalog::new(&Plugins::default(), true);
        let without = Catalog::new(&Plugins::default(), false);
        assert_eq!(with.view(ALL, "multi").len(), 1);
        assert!(without.view(ALL, "multi").is_empty());
        assert_eq!(without.view(ALL, "").len(), builtin_count(false));
    }

    #[test]
    fn search_matches_title_group_and_plugin_name_case_insensitively() {
        let c = Catalog::new(&sample(), true);
        // Title.
        assert_eq!(titles(&c.view(ALL, "VOLUME up"))[0].1.len(), 1);
        // Category name.
        let audio = c.view(ALL, "audio");
        assert_eq!(audio.groups.len(), 1);
        assert_eq!(audio.groups[0].title, "Audio");
        // Plugin name finds all of its actions.
        let ha = c.view(ALL, "home assist");
        assert_eq!(ha.groups.len(), 1);
        assert_eq!(ha.groups[0].entries.len(), 3);
        // Words may come from different places.
        assert_eq!(
            titles(&c.view(ALL, "home toggle")),
            vec![("Home Assistant".to_string(), vec!["Toggle an entity".to_string()])]
        );
        // "obs" finds the built-in OBS actions and the plugin's.
        let obs = c.view(ALL, "obs");
        assert_eq!(
            obs.groups.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(),
            vec!["OBS Studio", "OBS Studio (plugin)"]
        );
        assert!(obs.headers, "search results keep their section headers");
        assert!(c.view(ALL, "nothing like this").is_empty());
        // Spaces alone are no query.
        assert_eq!(c.view(ALL, "   ").len(), c.view(ALL, "").len());
    }

    #[test]
    fn searching_a_category_counts_matches_elsewhere() {
        let c = Catalog::new(&sample(), true);
        let v = c.view("plugin:zzz", "toggle");
        assert_eq!(
            titles(&v),
            vec![("Home Assistant".to_string(), vec!["Toggle an entity".to_string()])]
        );
        let in_all = c.view(ALL, "toggle").len();
        assert_eq!(v.elsewhere, in_all - 1);
        assert!(v.elsewhere >= 1, "the OBS plugin's Toggle recording");
        let none_here = c.view("plugin:obs", "scene");
        assert!(none_here.is_empty());
        assert_eq!(none_here.elsewhere, c.view(ALL, "scene").len());
        // No query: nothing is counted.
        assert_eq!(c.view("plugin:obs", "").elsewhere, 0);
        assert_eq!(c.view(ALL, "toggle").elsewhere, 0);
    }

    #[test]
    fn unknown_categories_fall_back_to_all() {
        let c = Catalog::new(&sample(), true);
        assert_eq!(c.resolve("plugin:obs"), "plugin:obs");
        assert_eq!(c.resolve(&c.tabs()[2].id), c.tabs()[2].id);
        assert_eq!(c.resolve("plugin:gone"), ALL);
        assert_eq!(c.resolve("plugin:bad"), ALL);
        assert_eq!(c.resolve(""), ALL);
        assert_eq!(c.view("plugin:gone", "").len(), c.view(ALL, "").len());
    }

    #[test]
    fn entries_carry_the_action_a_click_or_drag_assigns() {
        let mut p = sample();
        let obs: Manifest =
            Manifest::parse(include_str!("../../../plugins/obs/plugin.toml")).expect("obs manifest");
        p.manifests.insert("obs".into(), obs);
        let c = Catalog::new(&p, true);
        // Built-in: a fresh action from the catalog.
        for spec in catalog::catalog() {
            let v = c.view(ALL, "");
            let e = v
                .groups
                .iter()
                .flat_map(|g| &g.entries)
                .find(|e| e.title == spec.title && e.plugin.is_none())
                .unwrap_or_else(|| panic!("{}", spec.title));
            assert_eq!(e.action, (spec.make)());
        }
        // Plugin: the plugin action with its parameters' defaults.
        let v = c.view("plugin:obs", "input's volume");
        let e = v.first().expect("input_volume");
        let want: toml::Table = toml::from_str("op = \"up\"\npercent = 0\nstep = 2").expect("toml");
        assert_eq!(
            e.action,
            Action::Plugin {
                plugin: "obs".into(),
                action: "input_volume".into(),
                params: want,
            }
        );
        assert_eq!(e.plugin.as_deref(), Some("OBS Studio"));
    }

    #[test]
    fn plugin_entries_show_the_plugin_name_and_icon() {
        let c = Catalog::new(&sample(), true);
        let v = c.view("plugin:zzz", "");
        let g = &v.groups[0];
        assert_eq!(g.icon.as_deref(), Some("mdi:home-assistant"));
        let toggle = g
            .entries
            .iter()
            .find(|e| e.title == "Toggle an entity")
            .expect("toggle");
        assert_eq!(toggle.icon.as_deref(), Some("mdi:toggle-switch"));
        assert_eq!(toggle.plugin.as_deref(), Some("Home Assistant"));
        let call = g
            .entries
            .iter()
            .find(|e| e.title == "Call a service")
            .expect("call");
        assert_eq!(
            call.icon.as_deref(),
            Some("mdi:home-assistant"),
            "falls back to the plugin's icon"
        );
        let builtin = c.view(ALL, "next page");
        assert_eq!(
            builtin.first().map(|e| (e.plugin.clone(), e.icon.clone())),
            Some((None, None))
        );
        assert_eq!(builtin.first().map(|e| e.title.as_str()), Some("Next Page"));
    }

    #[test]
    fn first_entry_is_the_top_of_the_list() {
        let c = Catalog::new(&sample(), true);
        let v = c.view(ALL, "toggle");
        assert_eq!(
            v.first().map(|e| e.title.as_str()),
            v.groups[0].entries.first().map(|e| e.title.as_str())
        );
        assert!(c.view(ALL, "zzzz").first().is_none());
    }

    #[test]
    fn many_plugins_stay_in_name_order() {
        let ms: Vec<Manifest> = (0..40)
            .map(|i| {
                let id = format!("p{:02}", 39 - i);
                manifest(&id, &format!("Plugin {}", 39 - i), None, &[("go", "Go", None)])
            })
            .collect();
        let c = Catalog::new(&plugins(ms, vec![]), true);
        let v = c.view(ALL, "plugin");
        assert_eq!(v.groups.len(), 40);
        let names: Vec<&str> = v.groups.iter().map(|g| g.title.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(names, sorted);
        assert_eq!(c.tabs().len(), 1 + Category::ALL.len() + 40);
    }

    #[test]
    fn groups_open_unless_closed_and_always_while_searching() {
        let mut closed = HashSet::new();
        assert!(expanded(&closed, "cat:audio", ""));
        closed.insert("cat:audio".to_string());
        assert!(!expanded(&closed, "cat:audio", ""));
        assert!(!expanded(&closed, "cat:audio", "  "));
        assert!(expanded(&closed, "cat:audio", "vol"));
        assert!(expanded(&closed, "cat:media", ""));
    }

    #[test]
    fn empty_and_elsewhere_words() {
        assert_eq!(
            empty_text("lamp", "Media"),
            (
                "No matching actions".to_string(),
                Some("Nothing in Media matches “lamp”".to_string())
            )
        );
        assert_eq!(
            empty_text("lamp", "All"),
            (
                "No matching actions".to_string(),
                Some("Try another word".to_string())
            )
        );
        assert_eq!(empty_text("", "All"), ("No actions".to_string(), None));
        assert_eq!(elsewhere_label(0), None);
        assert_eq!(elsewhere_label(1).as_deref(), Some("Show 1 more in All"));
        assert_eq!(elsewhere_label(12).as_deref(), Some("Show 12 more in All"));
    }

    #[test]
    fn memory_starts_on_all() {
        let m = Memory::default();
        assert_eq!(m.category, ALL);
        assert!(m.collapsed.is_empty());
    }
}
