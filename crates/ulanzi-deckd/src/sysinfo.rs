//! CPU, RAM and CPU temperature from /proc and /sys (same sources and order as the Python version).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SysStats {
    pub cpu: u8,
    pub ram: u8,
    pub cpu_temp: u8,
}

/// Samples system stats; CPU usage is the delta between consecutive samples.
pub struct SysSampler {
    root: PathBuf,
    prev: Option<(u64, u64)>,
}

impl SysSampler {
    /// `root` is `/` in production, a temp dir in tests.
    pub fn new(root: impl Into<PathBuf>) -> SysSampler {
        SysSampler {
            root: root.into(),
            prev: None,
        }
    }

    pub fn sample(&mut self) -> SysStats {
        SysStats {
            cpu: self.cpu(),
            ram: self.ram(),
            cpu_temp: self.temp(),
        }
    }

    fn read(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel)).ok()
    }

    fn cpu(&mut self) -> u8 {
        let Some(stat) = self.read("proc/stat") else {
            return 0;
        };
        let Some(line) = stat.lines().find(|l| l.starts_with("cpu ")) else {
            return 0;
        };
        let f: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|v| v.parse().ok())
            .collect();
        if f.len() < 8 {
            return 0;
        }
        let idle = f[3] + f[4];
        let busy = f[0] + f[1] + f[2] + f[5] + f[6] + f[7];
        let total = idle + busy;
        let pct = match self.prev {
            Some((pt, pi)) if total > pt => {
                let dt = total - pt;
                let di = idle.saturating_sub(pi);
                ((dt.saturating_sub(di)) * 100 / dt) as u8
            }
            _ => 0,
        };
        self.prev = Some((total, idle));
        pct
    }

    fn ram(&self) -> u8 {
        let Some(info) = self.read("proc/meminfo") else {
            return 0;
        };
        let field = |name: &str| {
            info.lines()
                .find(|l| l.starts_with(name))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        };
        match (field("MemTotal:"), field("MemAvailable:")) {
            (Some(t), Some(a)) if t > 0 => ((t.saturating_sub(a)) * 100 / t) as u8,
            _ => 0,
        }
    }

    fn temp(&self) -> u8 {
        let milli = |p: &Path| {
            std::fs::read_to_string(p)
                .ok()
                .and_then(|s| s.trim().parse::<i64>().ok())
        };
        let to_c = |m: i64| (m / 1000).clamp(0, 255) as u8;
        let mut hwmons: Vec<PathBuf> = dir_entries(&self.root.join("sys/class/hwmon"));
        hwmons.sort();
        for h in &hwmons {
            let name = std::fs::read_to_string(h.join("name")).unwrap_or_default();
            if matches!(name.trim(), "k10temp" | "coretemp" | "cpu_thermal") {
                let max = dir_entries(h)
                    .iter()
                    .filter(|p| {
                        p.file_name().is_some_and(|n| {
                            let n = n.to_string_lossy();
                            n.starts_with("temp") && n.ends_with("_input")
                        })
                    })
                    .filter_map(|p| milli(p))
                    .max();
                if let Some(m) = max {
                    return to_c(m);
                }
            }
        }
        let mut zones = dir_entries(&self.root.join("sys/class/thermal"));
        zones.sort();
        for z in &zones {
            let kind = std::fs::read_to_string(z.join("type"))
                .unwrap_or_default()
                .to_lowercase();
            if kind.contains("cpu")
                && let Some(m) = milli(&z.join("temp"))
            {
                return to_c(m);
            }
        }
        milli(&self.root.join("sys/class/thermal/thermal_zone0/temp"))
            .map(to_c)
            .unwrap_or(0)
    }
}

fn dir_entries(p: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(p)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}
