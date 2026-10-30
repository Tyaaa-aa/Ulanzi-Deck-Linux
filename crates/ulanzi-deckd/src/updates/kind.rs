//! How this copy of the app was installed, which decides how it can update itself.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use ulanzi_core::update::InstallKind;

/// This copy's install kind, worked out once.
pub fn detect() -> InstallKind {
    static KIND: OnceLock<InstallKind> = OnceLock::new();
    *KIND.get_or_init(|| {
        let exe = std::env::current_exe().unwrap_or_default();
        let manifest = ulanzi_core::store::data_dir()
            .map(|d| d.join("installed-files"))
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        classify(
            &exe,
            std::env::var_os("APPIMAGE").is_some(),
            &manifest,
            &|tool, args| {
                Command::new(tool)
                    .args(args)
                    .arg(&exe)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
            },
        )
    })
}

/// `owned` says whether a package manager command claims this program. `manifest` is the list of
/// files a tarball install wrote.
fn classify(
    exe: &Path,
    appimage: bool,
    manifest: &str,
    owned: &dyn Fn(&str, &[&str]) -> bool,
) -> InstallKind {
    if appimage {
        return InstallKind::AppImage;
    }
    let path = exe.to_string_lossy();
    if exe.starts_with("/usr/bin") {
        if owned("pacman", &["-Qo"]) {
            return InstallKind::Arch;
        }
        if owned("dpkg", &["-S"]) {
            return InstallKind::Deb;
        }
        if owned("rpm", &["-qf"]) {
            return InstallKind::Rpm;
        }
        return InstallKind::Other;
    }
    let for_one_user = !manifest.lines().any(|l| l == "mode=system");
    if for_one_user && manifest.lines().any(|l| l == path) {
        return InstallKind::Tarball;
    }
    InstallKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nobody(_: &str, _: &[&str]) -> bool {
        false
    }

    #[test]
    fn an_appimage_is_an_appimage_wherever_it_sits() {
        let k = classify(Path::new("/tmp/.mount_x/usr/bin/ulanzi-deckd"), true, "", &nobody);
        assert_eq!(k, InstallKind::AppImage);
    }

    #[test]
    fn the_package_manager_that_owns_it_decides() {
        let exe = Path::new("/usr/bin/ulanzi-deckd");
        assert_eq!(classify(exe, false, "", &|t, _| t == "pacman"), InstallKind::Arch);
        assert_eq!(classify(exe, false, "", &|t, _| t == "dpkg"), InstallKind::Deb);
        assert_eq!(classify(exe, false, "", &|t, _| t == "rpm"), InstallKind::Rpm);
        assert_eq!(classify(exe, false, "", &nobody), InstallKind::Other);
    }

    #[test]
    fn a_tarball_for_one_user_is_found_by_its_file_list() {
        let exe = Path::new("/home/a/.local/bin/ulanzi-deckd");
        let list = "/home/a/.local/bin/ulanzi-deckd\n";
        assert_eq!(classify(exe, false, list, &nobody), InstallKind::Tarball);
        assert_eq!(classify(exe, false, "", &nobody), InstallKind::Other);
    }

    #[test]
    fn a_system_tarball_needs_sudo_so_it_only_gets_a_notice() {
        let exe = Path::new("/usr/local/bin/ulanzi-deckd");
        let list = "mode=system\n/usr/local/bin/ulanzi-deckd\n";
        assert_eq!(classify(exe, false, list, &nobody), InstallKind::Other);
    }

    #[test]
    fn a_development_build_only_gets_a_notice() {
        let exe = Path::new("/home/a/src/target/debug/ulanzi-deckd");
        assert_eq!(classify(exe, false, "", &nobody), InstallKind::Other);
    }
}
