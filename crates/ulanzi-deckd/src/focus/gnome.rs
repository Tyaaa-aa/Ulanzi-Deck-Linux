//! GNOME: a small Shell extension reports the focused window to our `WindowActivated` method.
//! GNOME only lets the user turn extensions on, so we install it and ask GNOME to enable it; a new
//! extension is loaded at the next login.

const UUID: &str = "ulanzi-deck-focus@tyaaa-aa.github.io";
const METADATA: &str =
    include_str!("../../../../packaging/gnome-extension/ulanzi-deck-focus@tyaaa-aa.github.io/metadata.json");
const SCRIPT: &str =
    include_str!("../../../../packaging/gnome-extension/ulanzi-deck-focus@tyaaa-aa.github.io/extension.js");

fn extension_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))?;
    Some(base.join("gnome-shell/extensions").join(UUID))
}

/// Write the extension (when missing or changed). True when files were written.
pub fn install() -> std::io::Result<bool> {
    let Some(dir) = extension_dir() else {
        return Ok(false);
    };
    let same = std::fs::read_to_string(dir.join("extension.js")).is_ok_and(|s| s == SCRIPT)
        && std::fs::read_to_string(dir.join("metadata.json")).is_ok_and(|s| s == METADATA);
    if same {
        return Ok(false);
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("metadata.json"), METADATA)?;
    std::fs::write(dir.join("extension.js"), SCRIPT)?;
    Ok(true)
}

pub async fn run() {
    match install() {
        Ok(fresh) => {
            let _ = tokio::process::Command::new("gnome-extensions")
                .args(["enable", UUID])
                .status()
                .await;
            super::set_status(
                "gnome",
                "waiting",
                if fresh {
                    "Log out and back in once so GNOME loads the Ulanzi Deck extension"
                } else {
                    "Waiting for the Ulanzi Deck extension. Check it is on in the Extensions app"
                },
            );
        }
        Err(e) => super::set_status(
            "gnome",
            "unavailable",
            &format!("Can't install the extension: {e}"),
        ),
    }
}
