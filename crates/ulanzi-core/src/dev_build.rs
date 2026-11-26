//! What a dev build is. `scripts/dev-install.sh` sets the `ULANZI_DEV_*` variables at build time;
//! release builds have none of them, so [`DevBuild`] is `None` there.

/// A dev build's details, read at compile time with [`dev_build!`](crate::dev_build!).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevBuild {
    /// Plain words: what this build is for.
    pub note: String,
    pub branch: String,
    /// Short commit hash the build started from.
    pub commit: String,
    /// That commit's subject line.
    pub subject: String,
    /// Files changed on top of the commit.
    pub dirty: u32,
    /// Local build time, `YYYY-MM-DD HH:MM`.
    pub built: String,
}

/// This crate's dev build details, or `None` in a release build. A macro so the variables are read
/// in the calling binary, which is rebuilt when they change; the core library is not.
#[macro_export]
macro_rules! dev_build {
    () => {
        $crate::dev_build::DevBuild::from_env(
            option_env!("ULANZI_DEV_COMMIT"),
            option_env!("ULANZI_DEV_NOTE"),
            option_env!("ULANZI_DEV_BRANCH"),
            option_env!("ULANZI_DEV_SUBJECT"),
            option_env!("ULANZI_DEV_DIRTY"),
            option_env!("ULANZI_DEV_BUILT"),
        )
    };
}

impl DevBuild {
    /// Built from the raw variables; no commit means a release build.
    pub fn from_env(
        commit: Option<&str>,
        note: Option<&str>,
        branch: Option<&str>,
        subject: Option<&str>,
        dirty: Option<&str>,
        built: Option<&str>,
    ) -> Option<DevBuild> {
        let commit = commit.map(str::trim).filter(|c| !c.is_empty())?;
        let s = |v: Option<&str>| v.unwrap_or("").trim().to_string();
        Some(DevBuild {
            note: s(note),
            branch: s(branch),
            commit: commit.to_string(),
            subject: s(subject),
            dirty: dirty.and_then(|d| d.trim().parse().ok()).unwrap_or(0),
            built: s(built),
        })
    }

    /// Short text for the chip: `DEV · fb0ba5a`, with `*` when it had uncommitted changes.
    pub fn chip(&self) -> String {
        let star = if self.dirty > 0 { "*" } else { "" };
        format!("DEV · {}{star}", self.commit)
    }

    /// One line that names this exact build, for telling two builds apart.
    pub fn id(&self) -> String {
        format!("{}|{}|{}", self.commit, self.dirty, self.built)
    }

    /// Packed into one string for D-Bus (fields joined by a unit separator).
    pub fn encode(&self) -> String {
        [
            self.commit.as_str(),
            &self.note,
            &self.branch,
            &self.subject,
            &self.dirty.to_string(),
            &self.built,
        ]
        .join("\u{1f}")
    }

    /// The reverse of [`encode`](Self::encode); an empty string is a release build.
    pub fn decode(s: &str) -> Option<DevBuild> {
        let f: Vec<&str> = s.split('\u{1f}').collect();
        let get = |i: usize| f.get(i).copied();
        DevBuild::from_env(get(0), get(1), get(2), get(3), get(4), get(5))
    }

    /// The details, one per line, for a tooltip.
    pub fn describe(&self) -> String {
        let mut lines = Vec::new();
        if !self.note.is_empty() {
            lines.push(self.note.clone());
            lines.push(String::new());
        }
        lines.push(format!("Branch: {}", self.branch));
        lines.push(format!("Commit: {} {}", self.commit, self.subject));
        match self.dirty {
            0 => lines.push("No uncommitted changes".into()),
            1 => lines.push("Plus 1 uncommitted file".into()),
            n => lines.push(format!("Plus {n} uncommitted files")),
        }
        lines.push(format!("Built: {}", self.built));
        lines.join("\n")
    }
}

/// What the service is running, next to the app's own build, as one tooltip line or more.
/// `service` is `None` when the service was not asked or did not answer; `Some(None)` is a release.
pub fn service_line(
    app: &DevBuild,
    service: Option<Option<&DevBuild>>,
    service_version: Option<&str>,
) -> String {
    match service {
        None => match service_version {
            Some(v) => format!("Service: {v}, build unknown (older service)"),
            None => "Service: not connected".into(),
        },
        Some(None) => format!(
            "Service: release {} (not this dev build)",
            service_version.unwrap_or("?")
        ),
        Some(Some(s)) if s.id() == app.id() => "Service: same build as the app".into(),
        Some(Some(s)) => {
            let mut out = format!("Service: a different dev build ({}, built {})", s.chip(), s.built);
            if !s.note.is_empty() {
                out.push_str(&format!("\n{}", s.note));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(dirty: u32) -> DevBuild {
        DevBuild {
            note: "Fix for the wide-screen clock.".into(),
            branch: "roadmap-next".into(),
            commit: "fb0ba5a".into(),
            subject: "fix: pictures".into(),
            dirty,
            built: "2026-11-25 00:29".into(),
        }
    }

    #[test]
    fn release_has_no_build() {
        assert_eq!(DevBuild::from_env(None, Some("x"), None, None, None, None), None);
        assert_eq!(DevBuild::from_env(Some(" "), None, None, None, None, None), None);
        assert_eq!(DevBuild::decode(""), None);
    }

    #[test]
    fn chip_marks_uncommitted() {
        assert_eq!(build(0).chip(), "DEV · fb0ba5a");
        assert_eq!(build(2).chip(), "DEV · fb0ba5a*");
    }

    #[test]
    fn round_trips_over_dbus() {
        let b = build(1);
        assert_eq!(DevBuild::decode(&b.encode()), Some(b));
    }

    #[test]
    fn describes_in_plain_words() {
        let d = build(1).describe();
        assert!(d.starts_with("Fix for the wide-screen clock.\n\n"));
        assert!(d.contains("Plus 1 uncommitted file"));
    }

    #[test]
    fn service_line_tells_builds_apart() {
        let app = build(1);
        assert_eq!(
            service_line(&app, Some(Some(&build(1))), Some("2.5.0")),
            "Service: same build as the app"
        );
        assert!(
            service_line(&app, Some(Some(&build(0))), Some("2.5.0"))
                .starts_with("Service: a different dev build")
        );
        assert!(service_line(&app, Some(None), Some("2.5.0")).contains("release 2.5.0"));
        assert_eq!(service_line(&app, None, None), "Service: not connected");
    }
}
