//! Update checks: which release is newest, which file fits this install, and the text to show.
//! The service does the network work; this is the part with no side effects.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::store::{Store, write_atomic};

const STATE_FILE: &str = "updates.toml";

/// `v2.1.2` or `2.1.2` as numbers; anything else (a release candidate, `latest`) is `None`.
pub fn parse_version(tag: &str) -> Option<(u32, u32, u32)> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let mut next = || parts.next()?.parse::<u32>().ok();
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Whether the release `tag` is newer than the running version.
pub fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(t), Some(c)) => t > c,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    pub title: String,
    pub published: String,
    pub notes: String,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn version(&self) -> &str {
        self.tag.strip_prefix('v').unwrap_or(&self.tag)
    }

    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// GitHub's description of a release; `None` for a draft, a prerelease or anything unreadable.
pub fn parse_latest(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v["draft"].as_bool() != Some(false) || v["prerelease"].as_bool() != Some(false) {
        return None;
    }
    let text = |key: &str| v[key].as_str().unwrap_or_default().to_string();
    let assets = v["assets"]
        .as_array()?
        .iter()
        .filter_map(|a| {
            Some(Asset {
                name: a["name"].as_str()?.to_string(),
                url: a["browser_download_url"].as_str()?.to_string(),
                size: a["size"].as_u64()?,
            })
        })
        .collect();
    Some(Release {
        tag: v["tag_name"].as_str()?.to_string(),
        title: text("name"),
        published: text("published_at"),
        notes: text("body"),
        assets,
    })
}

/// How this copy of the app was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallKind {
    AppImage,
    Arch,
    Deb,
    Rpm,
    Tarball,
    Other,
}

/// The release file for an install of `kind`.
pub fn asset_name(kind: InstallKind, version: &str, arch: &str) -> Option<String> {
    Some(match kind {
        InstallKind::AppImage => format!("Ulanzi_Deck-{version}-{arch}.AppImage"),
        InstallKind::Arch => format!("ulanzi-deck-{version}-1-{arch}.pkg.tar.zst"),
        InstallKind::Deb => {
            let arch = match arch {
                "x86_64" => "amd64",
                "aarch64" => "arm64",
                other => other,
            };
            format!("ulanzi-deck_{version}-1_{arch}.deb")
        }
        InstallKind::Rpm => format!("ulanzi-deck-{version}-1.{arch}.rpm"),
        InstallKind::Tarball => format!("ulanzi-deck-{version}-{arch}.tar.gz"),
        InstallKind::Other => return None,
    })
}

/// The text of the `### Important` section, and the notes without it.
pub fn split_notes(notes: &str) -> (Option<String>, String) {
    let mut note = Vec::new();
    let mut rest = Vec::new();
    let mut inside = false;
    for line in notes.lines() {
        if line.starts_with('#') {
            inside = line
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case("important");
            if inside {
                continue;
            }
        }
        if inside {
            note.push(line);
        } else {
            rest.push(line);
        }
    }
    let note = note.join("\n").trim().to_string();
    ((!note.is_empty()).then_some(note), rest.join("\n"))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// One line of notes as Pango markup: bold, inline code and links. Anything unclosed stays text.
fn inline(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some((inner, tail)) = after.split_once("**")
        {
            out.push_str(&format!("<b>{}</b>", inline(inner)));
            rest = tail;
        } else if let Some(after) = rest.strip_prefix('`')
            && let Some((inner, tail)) = after.split_once('`')
        {
            out.push_str(&format!("<tt>{}</tt>", escape(inner)));
            rest = tail;
        } else if let Some(after) = rest.strip_prefix('[')
            && let Some((text, tail)) = after.split_once("](")
            && let Some((url, tail)) = tail.split_once(')')
        {
            if url.starts_with("https://") || url.starts_with("http://") {
                out.push_str(&format!("<a href=\"{}\">{}</a>", escape(url), escape(text)));
            } else {
                out.push_str(&escape(text));
            }
            rest = tail;
        } else {
            out.push_str(&escape(&c.to_string()));
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// Release notes as Pango markup: headings, bullets, bold, inline code and links.
pub fn notes_markup(notes: &str) -> String {
    notes
        .lines()
        .map(|line| {
            let t = line.trim();
            if t.starts_with('#') {
                format!("<b>{}</b>", inline(t.trim_start_matches('#').trim()))
            } else if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                format!("• {}", inline(item))
            } else {
                inline(t)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the update check remembers (`updates.toml` in the state folder, not `settings.toml`, so a
/// downgrade to 2.1.0 can still read its settings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateState {
    pub check: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_check: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offered: Option<Release>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whats_new_shown: Option<String>,
}

impl Default for UpdateState {
    fn default() -> Self {
        UpdateState {
            check: true,
            last_check: None,
            etag: None,
            offered: None,
            skipped: None,
            whats_new_shown: None,
        }
    }
}

impl UpdateState {
    /// The saved state, or the defaults when there is none or it cannot be read.
    pub fn load(store: &Store) -> UpdateState {
        std::fs::read_to_string(store.state_dir().join(STATE_FILE))
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, store: &Store) -> Result<(), CoreError> {
        let text = toml::to_string(self).map_err(|e| CoreError::Serialize(e.to_string()))?;
        let path = store.state_dir().join(STATE_FILE);
        write_atomic(&path, text.as_bytes()).map_err(|e| CoreError::io(&path, e))
    }
}

/// What the window needs to know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub current: String,
    pub offered: Option<Release>,
    pub kind: InstallKind,
    pub can_install: bool,
    pub check: bool,
    pub last_check: Option<String>,
    pub whats_new_shown: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v2.1.10", "2.1.9"));
        assert!(is_newer("2.2.0", "2.1.9"));
        assert!(!is_newer("v2.1.1", "2.1.1"));
        assert!(!is_newer("v2.1.0", "2.1.1"));
    }

    #[test]
    fn odd_tags_are_never_offered() {
        for t in ["v2.1.2-rc1", "latest", "", "v2.1", "v2.1.2.3", "vv2.1.2"] {
            assert!(!is_newer(t, "2.1.1"), "{t}");
        }
    }

    const JSON: &str = r#"{"tag_name":"v2.1.2","name":"Ulanzi Deck 2.1.2","draft":false,
        "prerelease":false,"published_at":"2026-11-02T10:00:00Z","body":"Notes",
        "assets":[{"name":"SHA256SUMS","browser_download_url":"https://x/SHA256SUMS","size":10}]}"#;

    #[test]
    fn a_release_is_read() {
        let r = parse_latest(JSON).expect("release");
        assert_eq!(r.tag, "v2.1.2");
        assert_eq!(r.title, "Ulanzi Deck 2.1.2");
        assert_eq!(r.notes, "Notes");
        assert_eq!(r.assets[0].url, "https://x/SHA256SUMS");
        assert_eq!(r.assets[0].size, 10);
    }

    #[test]
    fn drafts_and_prereleases_are_ignored() {
        assert!(parse_latest(&JSON.replace("\"draft\":false", "\"draft\":true")).is_none());
        assert!(parse_latest(&JSON.replace("\"prerelease\":false", "\"prerelease\":true")).is_none());
        assert!(parse_latest("not json").is_none());
    }

    #[test]
    fn each_kind_names_its_asset() {
        let name = |k| asset_name(k, "2.1.2", "x86_64");
        assert_eq!(
            name(InstallKind::Arch).as_deref(),
            Some("ulanzi-deck-2.1.2-1-x86_64.pkg.tar.zst")
        );
        assert_eq!(
            name(InstallKind::Deb).as_deref(),
            Some("ulanzi-deck_2.1.2-1_amd64.deb")
        );
        assert_eq!(
            name(InstallKind::Rpm).as_deref(),
            Some("ulanzi-deck-2.1.2-1.x86_64.rpm")
        );
        assert_eq!(
            name(InstallKind::Tarball).as_deref(),
            Some("ulanzi-deck-2.1.2-x86_64.tar.gz")
        );
        assert_eq!(
            name(InstallKind::AppImage).as_deref(),
            Some("Ulanzi_Deck-2.1.2-x86_64.AppImage")
        );
        assert_eq!(name(InstallKind::Other), None);
    }

    #[test]
    fn the_important_section_is_pulled_out() {
        let (note, rest) =
            split_notes("Intro\n\n### Important\n\nBack up first.\nReally.\n\n### New\n\n- A\n");
        assert_eq!(note.as_deref(), Some("Back up first.\nReally."));
        assert!(rest.contains("Intro"));
        assert!(rest.contains("### New"));
        assert!(!rest.contains("Back up"));
        assert_eq!(split_notes("Just notes").0, None);
    }

    #[test]
    fn notes_escape_markup_and_keep_links() {
        let m = notes_markup("### New\n\n- **Bold** <b> & `code`\n- [Site](https://x.org/a?b=1&c=2)\n");
        assert!(m.contains("<b>Bold</b>"), "{m}");
        assert!(m.contains("&lt;b&gt; &amp;"), "{m}");
        assert!(m.contains("<tt>code</tt>"), "{m}");
        assert!(
            m.contains("<a href=\"https://x.org/a?b=1&amp;c=2\">Site</a>"),
            "{m}"
        );
        assert!(m.contains("• "), "{m}");
    }

    #[test]
    fn notes_markup_drops_links_that_are_not_http() {
        let m = notes_markup("[a](javascript:bad) [b](file:///etc/passwd)");
        assert!(!m.contains("<a "), "{m}");
        assert!(m.contains('a') && m.contains('b'));
    }

    #[test]
    fn notes_never_produce_broken_markup() {
        let m = notes_markup("**open and `open and [x](https://a\"b)");
        assert!(gtkless_balanced(&m), "{m}");
    }

    /// Every opened tag is closed, in order.
    fn gtkless_balanced(m: &str) -> bool {
        let mut open: Vec<&str> = Vec::new();
        for part in m.split('<').skip(1) {
            let tag = part.split('>').next().unwrap_or("");
            match tag.strip_prefix('/') {
                Some(t) => {
                    if open.pop() != Some(t) {
                        return false;
                    }
                }
                None => open.push(tag.split(' ').next().unwrap_or("")),
            }
        }
        open.is_empty()
    }

    #[test]
    fn state_round_trips_and_defaults_to_checking() {
        let d = tempfile::tempdir().expect("tmp");
        let store = crate::store::Store::at(d.path().join("c"), d.path().join("s"), d.path().join("k"));
        let mut s = UpdateState::load(&store);
        assert!(s.check);
        s.skipped = Some("2.1.2".into());
        s.check = false;
        s.save(&store).expect("save");
        assert_eq!(UpdateState::load(&store), s);
    }
}
