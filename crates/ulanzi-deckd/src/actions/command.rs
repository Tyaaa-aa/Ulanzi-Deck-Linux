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

/// Under systemd, run `argv` in its own transient scope so that restarting the service does not kill
/// apps launched from the deck (they would otherwise live in the service's cgroup).
pub fn wrap_in_scope(argv: &[String], under_systemd: bool, has_systemd_run: bool) -> Vec<String> {
    if !(under_systemd && has_systemd_run) {
        return argv.to_vec();
    }
    let mut out: Vec<String> = ["systemd-run", "--user", "--scope", "--quiet", "--collect", "--"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    out.extend_from_slice(argv);
    out
}

/// Start `argv` in its own process group (and, under systemd, its own scope) with no stdio, and reap
/// it in the background.
pub fn spawn_detached(argv: &[String]) -> Result<(), ActionError> {
    let under_systemd = std::env::var_os("INVOCATION_ID").is_some();
    let argv = wrap_in_scope(
        argv,
        under_systemd,
        under_systemd && which("systemd-run").is_some(),
    );
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

/// Time limit for helper tools (wpctl, pactl) so a hung one cannot stall an action queue.
const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Run `argv` to completion and return stdout; a non-zero exit becomes an error with stderr.
pub async fn run_capture(argv: &[String]) -> Result<String, ActionError> {
    run_capture_timeout(argv, TOOL_TIMEOUT).await
}

/// [`run_capture`] with an explicit time limit; the process is killed when it runs out.
pub async fn run_capture_timeout(argv: &[String], limit: std::time::Duration) -> Result<String, ActionError> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| ActionError::Failed("nothing to run".into()))?;
    let child = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = match tokio::time::timeout(limit, child).await {
        Ok(r) => r.map_err(|e| ActionError::Unavailable(format!("could not run {prog}: {e}")))?,
        Err(_) => {
            return Err(ActionError::Failed(format!(
                "{prog} did not finish within {} s",
                limit.as_secs_f32()
            )));
        }
    };
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
