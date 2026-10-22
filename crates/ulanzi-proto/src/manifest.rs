use std::collections::BTreeMap;

use crate::json::py_str;

/// One key's entry in `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ButtonEntry {
    pub state: u8,
    pub text: Option<String>,
    /// Path inside the ZIP, e.g. `icons/k0.png`.
    pub icon: Option<String>,
}

/// Manifest key for a control index: `"{col}_{row}"` on the 5-wide grid.
pub fn manifest_key(index: u8) -> String {
    format!("{}_{}", index % 5, index / 5)
}

/// `manifest.json` exactly as Python writes it with
/// `json.dumps(m, sort_keys=True, separators=(',', ':'), indent=2)`.
pub fn manifest_json(entries: &BTreeMap<u8, ButtonEntry>) -> String {
    let mut sorted: Vec<(String, &ButtonEntry)> =
        entries.iter().map(|(i, e)| (manifest_key(*i), e)).collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::from("{\n");
    for (n, (key, e)) in sorted.iter().enumerate() {
        out.push_str(&format!(
            "  {}:{{\n    \"State\":{},\n    \"ViewParam\":[\n",
            py_str(key),
            e.state
        ));
        let mut fields = Vec::new();
        if let Some(icon) = &e.icon {
            fields.push(format!("        \"Icon\":{}", py_str(icon)));
        }
        if let Some(text) = &e.text {
            fields.push(format!("        \"Text\":{}", py_str(text)));
        }
        if fields.is_empty() {
            out.push_str("      {}\n");
        } else {
            out.push_str("      {\n");
            out.push_str(&fields.join(",\n"));
            out.push_str("\n      }\n");
        }
        out.push_str("    ]\n  }");
        out.push_str(if n + 1 < sorted.len() { ",\n" } else { "\n" });
    }
    out.push('}');
    out
}
