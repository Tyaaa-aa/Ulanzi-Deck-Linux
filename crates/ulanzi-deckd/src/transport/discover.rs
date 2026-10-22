use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceState {
    Ready { node: PathBuf },
    NotFound,
    NoPermission { node: PathBuf },
    Busy { node: PathBuf, pid: u32, exe: String },
    Error { message: String },
}

impl DeviceState {
    /// Stable machine-readable name used on D-Bus.
    pub fn code(&self) -> &'static str {
        match self {
            DeviceState::Ready { .. } => "ready",
            DeviceState::NotFound => "not_found",
            DeviceState::NoPermission { .. } => "no_permission",
            DeviceState::Busy { .. } => "busy",
            DeviceState::Error { .. } => "error",
        }
    }

    /// Human detail for the UI.
    pub fn detail(&self) -> String {
        match self {
            DeviceState::Ready { node } => node.display().to_string(),
            DeviceState::NotFound => "No Ulanzi deck is plugged in".into(),
            DeviceState::NoPermission { node } => format!("No permission to open {}", node.display()),
            DeviceState::Busy { node, pid, exe } => {
                format!("{} is in use by {exe} (pid {pid})", node.display())
            }
            DeviceState::Error { message } => message.clone(),
        }
    }
}

const HID_ID: &str = "HID_ID=0003:00002207:00000019";

/// Find the deck's hidraw node by scanning `<sysfs_hidraw>/*/device/uevent`.
pub fn find_node(sysfs_hidraw: &Path, dev: &Path) -> Option<PathBuf> {
    let mut names: Vec<String> = std::fs::read_dir(sysfs_hidraw)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names.into_iter().find_map(|name| {
        let uevent = std::fs::read_to_string(sysfs_hidraw.join(&name).join("device/uevent")).ok()?;
        uevent
            .lines()
            .any(|l| l.trim().eq_ignore_ascii_case(HID_ID))
            .then(|| dev.join(&name))
    })
}

/// Another process with `node` open, as (pid, process name).
pub fn find_holder(proc_root: &Path, node: &Path, self_pid: u32) -> Option<(u32, String)> {
    let entries = std::fs::read_dir(proc_root).ok()?;
    for e in entries.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if pid == self_pid {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else {
            continue;
        };
        let holds = fds
            .flatten()
            .any(|fd| std::fs::read_link(fd.path()).is_ok_and(|t| t == node));
        if holds {
            let name = std::fs::read_to_string(e.path().join("comm"))
                .map(|c| c.trim().to_string())
                .ok()
                .filter(|c| !c.is_empty())
                .or_else(|| {
                    std::fs::read_link(e.path().join("exe"))
                        .ok()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                })
                .unwrap_or_else(|| "unknown".into());
            return Some((pid, name));
        }
    }
    None
}

/// Work out the device state: present? held by someone else? can we open it?
pub fn probe(sysfs_hidraw: &Path, dev: &Path, proc_root: &Path) -> DeviceState {
    let Some(node) = find_node(sysfs_hidraw, dev) else {
        return DeviceState::NotFound;
    };
    if let Some((pid, exe)) = find_holder(proc_root, &node, std::process::id()) {
        return DeviceState::Busy { node, pid, exe };
    }
    match std::fs::OpenOptions::new().read(true).write(true).open(&node) {
        Ok(_) => DeviceState::Ready { node },
        Err(e) => open_error(node, &e),
    }
}

pub(crate) fn open_error(node: PathBuf, e: &std::io::Error) -> DeviceState {
    match e.raw_os_error() {
        Some(c)
            if c == rustix::io::Errno::ACCESS.raw_os_error()
                || c == rustix::io::Errno::PERM.raw_os_error() =>
        {
            DeviceState::NoPermission { node }
        }
        Some(c)
            if c == rustix::io::Errno::NOENT.raw_os_error()
                || c == rustix::io::Errno::NODEV.raw_os_error() =>
        {
            DeviceState::NotFound
        }
        _ => DeviceState::Error {
            message: format!("{}: {e}", node.display()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_sysfs(root: &Path, name: &str, hid_id: &str) {
        let d = root.join(name).join("device");
        std::fs::create_dir_all(&d).expect("mkdir");
        std::fs::write(
            d.join("uevent"),
            format!("DRIVER=hid-generic\nHID_ID={hid_id}\nHID_NAME=x\n"),
        )
        .expect("write");
    }

    #[test]
    fn finds_ulanzi_node() {
        let t = tempfile::tempdir().expect("tmp");
        fake_sysfs(t.path(), "hidraw3", "0003:0000046D:0000C52B");
        fake_sysfs(t.path(), "hidraw16", "0003:00002207:00000019");
        assert_eq!(
            find_node(t.path(), Path::new("/dev")),
            Some(PathBuf::from("/dev/hidraw16"))
        );
    }

    #[test]
    fn no_device_is_not_found() {
        let t = tempfile::tempdir().expect("tmp");
        fake_sysfs(t.path(), "hidraw3", "0003:0000046D:0000C52B");
        assert_eq!(find_node(t.path(), Path::new("/dev")), None);
        let dev = tempfile::tempdir().expect("tmp");
        assert_eq!(probe(t.path(), dev.path(), t.path()), DeviceState::NotFound);
    }

    fn fake_proc(root: &Path, pid: u32, comm: &str, target: &Path) {
        let fd = root.join(pid.to_string()).join("fd");
        std::fs::create_dir_all(&fd).expect("mkdir");
        std::os::unix::fs::symlink(target, fd.join("5")).expect("symlink");
        std::fs::write(root.join(pid.to_string()).join("comm"), format!("{comm}\n")).expect("comm");
    }

    #[test]
    fn holder_detected() {
        let t = tempfile::tempdir().expect("tmp");
        fake_proc(t.path(), 4242, "python3", Path::new("/dev/hidraw16"));
        fake_proc(t.path(), 7, "bash", Path::new("/dev/null"));
        assert_eq!(
            find_holder(t.path(), Path::new("/dev/hidraw16"), 1),
            Some((4242, "python3".into()))
        );
    }

    #[test]
    fn self_is_not_holder() {
        let t = tempfile::tempdir().expect("tmp");
        fake_proc(t.path(), 4242, "ulanzi-deckd", Path::new("/dev/hidraw16"));
        assert_eq!(find_holder(t.path(), Path::new("/dev/hidraw16"), 4242), None);
    }

    #[test]
    fn permission_denied_maps() {
        use std::os::unix::fs::PermissionsExt;
        if rustix::process::geteuid().is_root() {
            return;
        }
        let sys = tempfile::tempdir().expect("tmp");
        let dev = tempfile::tempdir().expect("tmp");
        let procfs = tempfile::tempdir().expect("tmp");
        fake_sysfs(sys.path(), "hidraw16", "0003:00002207:00000019");
        let node = dev.path().join("hidraw16");
        std::fs::write(&node, b"").expect("node");
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        assert_eq!(
            probe(sys.path(), dev.path(), procfs.path()),
            DeviceState::NoPermission { node }
        );
    }

    #[test]
    fn openable_node_is_ready() {
        let sys = tempfile::tempdir().expect("tmp");
        let dev = tempfile::tempdir().expect("tmp");
        let procfs = tempfile::tempdir().expect("tmp");
        fake_sysfs(sys.path(), "hidraw16", "0003:00002207:00000019");
        let node = dev.path().join("hidraw16");
        std::fs::write(&node, b"").expect("node");
        assert_eq!(
            probe(sys.path(), dev.path(), procfs.path()),
            DeviceState::Ready { node }
        );
    }

    #[test]
    fn state_codes() {
        assert_eq!(DeviceState::NotFound.code(), "not_found");
        assert_eq!(DeviceState::Ready { node: "/x".into() }.code(), "ready");
    }
}
