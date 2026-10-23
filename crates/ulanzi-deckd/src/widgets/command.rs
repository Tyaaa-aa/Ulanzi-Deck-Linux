//! Output of a command for the Command widget: time-limited and size-limited.

use std::time::Duration;

use tokio::io::AsyncReadExt;

pub const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_READ: u64 = 64 * 1024;

/// Run `cmd` with `sh -c`; at most two non-empty lines of its output, or a short error.
pub async fn run_bounded(cmd: &str, timeout: Duration) -> String {
    let child = tokio::process::Command::new("sh")
        .args(["-c", cmd])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return format!("can't run it: {e}"),
    };
    let Some(stdout) = child.stdout.take() else {
        return String::new();
    };
    let read = async {
        let mut buf = Vec::new();
        let _ = stdout.take(MAX_READ).read_to_end(&mut buf).await;
        buf
    };
    match tokio::time::timeout(timeout, read).await {
        Ok(buf) => {
            let _ = child.kill().await;
            let text = String::from_utf8_lossy(&buf);
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(2)
                .collect::<Vec<_>>()
                .join("\n")
        }
        Err(_) => {
            let _ = child.kill().await;
            "took too long (over 3 s)".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn command_output_is_bounded() {
        assert_eq!(run_bounded("printf 'a\\n\\nb\\nc\\n'", TIMEOUT).await, "a\nb");
        let t0 = std::time::Instant::now();
        let slow = run_bounded("sleep 10", Duration::from_millis(300)).await;
        assert!(slow.contains("too long"), "{slow}");
        assert!(t0.elapsed() < Duration::from_secs(2));
        let big = run_bounded("yes x | head -c 1000000", TIMEOUT).await;
        assert!(big.len() < 100, "{}", big.len());
    }
}
