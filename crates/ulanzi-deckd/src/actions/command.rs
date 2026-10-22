//! Spawning processes detached from the daemon.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::Command;

use super::ActionError;

/// First executable called `name` on `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

/// Start `argv` in its own process group with no stdio, and reap it in the background.
pub fn spawn_detached(argv: &[String]) -> Result<(), ActionError> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| ActionError::Failed("nothing to run".into()))?;
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| ActionError::Failed(format!("could not start {prog}: {e}")))?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

const TERMINALS: [&[&str]; 6] = [
    &["xdg-terminal-exec"],
    &["kgx", "--"],
    &["konsole", "-e"],
    &["gnome-terminal", "--"],
    &["cosmic-term", "-e"],
    &["xterm", "-e"],
];

/// Run `cmd` through `sh -c`, optionally inside the first terminal emulator found.
pub fn spawn_shell(cmd: &str, in_terminal: bool) -> Result<(), ActionError> {
    let mut argv: Vec<String> = Vec::new();
    if in_terminal {
        let term = TERMINALS
            .iter()
            .find(|t| which(t[0]).is_some())
            .ok_or_else(|| ActionError::Unavailable("no terminal emulator found".into()))?;
        argv.extend(term.iter().map(|s| s.to_string()));
    }
    argv.extend(["sh".to_string(), "-c".to_string(), cmd.to_string()]);
    spawn_detached(&argv)
}

/// Run `argv` to completion and return stdout; a non-zero exit becomes an error with stderr.
pub async fn run_capture(argv: &[String]) -> Result<String, ActionError> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| ActionError::Failed("nothing to run".into()))?;
    let out = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| ActionError::Unavailable(format!("could not run {prog}: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(ActionError::Failed(if err.is_empty() {
            format!("{prog} failed")
        } else {
            err
        }))
    }
}
