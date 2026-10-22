//! Label variables such as `{cpu}` and `{media_title}`.

use std::collections::BTreeMap;

/// Replace every `{name}` whose name is in `vars`; anything else is left as written.
pub fn resolve_vars(label: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = label.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}
