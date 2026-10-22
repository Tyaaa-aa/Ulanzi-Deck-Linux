//! Label variables such as `{cpu}` and `{media_title}`.

use std::collections::BTreeMap;

/// Names the Python version used, mapped to the current ones.
const ALIASES: [(&str, &str); 2] = [("cpu_usage", "cpu"), ("ram_usage", "ram")];

/// Replace every `{name}` whose name (or legacy alias) is in `vars`; anything else is left as written.
pub fn resolve_vars(label: &str, vars: &BTreeMap<String, String>) -> String {
    if !label.contains('{') {
        return label.to_string();
    }
    let mut out = label.to_string();
    for (old, new) in ALIASES {
        if let Some(v) = vars.get(new) {
            out = out.replace(&format!("{{{old}}}"), v);
        }
    }
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}
