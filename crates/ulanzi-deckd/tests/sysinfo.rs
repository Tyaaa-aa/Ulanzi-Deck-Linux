use std::collections::BTreeMap;
use std::path::Path;

use ulanzi_deckd::screen::art_cache_path;
use ulanzi_deckd::sysinfo::SysSampler;
use ulanzi_deckd::vars::resolve_vars;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, text).expect("write");
}

#[test]
fn cpu_from_two_samples() {
    let d = tempfile::tempdir().expect("tmp");
    write(
        d.path(),
        "proc/stat",
        "cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 1 1 1\n",
    );
    write(
        d.path(),
        "proc/meminfo",
        "MemTotal: 1000 kB\nMemAvailable: 1000 kB\n",
    );
    let mut s = SysSampler::new(d.path());
    assert_eq!(s.sample().cpu, 0, "first sample has no delta");
    write(d.path(), "proc/stat", "cpu  200 0 200 1400 0 0 0 0 0 0\n");
    assert_eq!(s.sample().cpu, 25);
}

#[test]
fn ram_percent() {
    let d = tempfile::tempdir().expect("tmp");
    write(
        d.path(),
        "proc/meminfo",
        "MemTotal:       1000 kB\nMemFree: 1 kB\nMemAvailable:    250 kB\n",
    );
    assert_eq!(SysSampler::new(d.path()).sample().ram, 75);
}

#[test]
fn temp_prefers_k10temp() {
    let d = tempfile::tempdir().expect("tmp");
    write(d.path(), "sys/class/hwmon/hwmon0/name", "acpitz\n");
    write(d.path(), "sys/class/hwmon/hwmon0/temp1_input", "90000\n");
    write(d.path(), "sys/class/hwmon/hwmon1/name", "k10temp\n");
    write(d.path(), "sys/class/hwmon/hwmon1/temp1_input", "45000\n");
    write(d.path(), "sys/class/hwmon/hwmon1/temp2_input", "52000\n");
    assert_eq!(SysSampler::new(d.path()).sample().cpu_temp, 52);
}

#[test]
fn temp_falls_back_to_thermal_zone() {
    let d = tempfile::tempdir().expect("tmp");
    write(d.path(), "sys/class/thermal/thermal_zone0/type", "x86_pkg_temp\n");
    write(d.path(), "sys/class/thermal/thermal_zone0/temp", "61000\n");
    write(d.path(), "sys/class/thermal/thermal_zone1/type", "cpu-thermal\n");
    write(d.path(), "sys/class/thermal/thermal_zone1/temp", "48000\n");
    assert_eq!(SysSampler::new(d.path()).sample().cpu_temp, 48);
}

#[test]
fn vars_replace_and_alias() {
    let mut v = BTreeMap::new();
    v.insert("cpu".to_string(), "10".to_string());
    v.insert("ram".to_string(), "20".to_string());
    assert_eq!(resolve_vars("{cpu_usage}% {ram} {x}", &v), "10% 20 {x}");
    assert_eq!(resolve_vars("{ram_usage}", &v), "20");
}

#[test]
fn art_cache_key_stable() {
    let d = Path::new("/c");
    assert_eq!(
        art_cache_path(d, "https://a/b.jpg"),
        art_cache_path(d, "https://a/b.jpg")
    );
    assert_ne!(
        art_cache_path(d, "https://a/b.jpg"),
        art_cache_path(d, "https://a/c.jpg")
    );
    assert!(art_cache_path(d, "https://a/b.jpg").starts_with("/c/art"));
}

#[test]
fn gpu_and_network_counters() {
    let d = tempfile::tempdir().expect("tmp");
    write(d.path(), "sys/class/drm/card0/device/gpu_busy_percent", "12\n");
    write(d.path(), "sys/class/drm/card1/device/gpu_busy_percent", "57\n");
    write(
        d.path(),
        "proc/net/dev",
        "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 9999 1 0 0 0 0 0 0 9999 1 0 0 0 0 0 0\n  eth0: 1000 10 0 0 0 0 0 0 300 3 0 0 0 0 0 0\n wlan0: 500 5 0 0 0 0 0 0 200 2 0 0 0 0 0 0\n",
    );
    let s = ulanzi_deckd::sysinfo::SysSampler::new(d.path()).sample();
    assert_eq!(s.gpu, 57, "the busiest GPU");
    assert_eq!((s.net_rx, s.net_tx), (1500, 500), "loopback left out");
}
