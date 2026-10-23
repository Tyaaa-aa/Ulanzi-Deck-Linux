//! Label variables such as `{cpu}` and `{media_title}`.

use std::collections::BTreeMap;

/// Names the Python version used, mapped to the current ones.
const ALIASES: [(&str, &str); 2] = [("cpu_usage", "cpu"), ("ram_usage", "ram")];

/// Replace every `{name}` whose name (or legacy alias) is in `vars`; anything else is left as
/// written. One pass: a value that itself looks like `{name}` is not expanded again.
pub fn resolve_vars(label: &str, vars: &BTreeMap<String, String>) -> String {
    if !label.contains('{') {
        return label.to_string();
    }
    let lookup = |name: &str| {
        ALIASES
            .iter()
            .find(|(old, _)| *old == name)
            .and_then(|(_, new)| vars.get(*new))
            .or_else(|| vars.get(name))
    };
    let mut out = String::with_capacity(label.len());
    let mut rest = label;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if !after[..close].contains('{') => match lookup(&after[..close]) {
                Some(v) => {
                    out.push_str(v);
                    rest = &after[close + 1..];
                }
                None => {
                    out.push_str(&rest[open..open + close + 2]);
                    rest = &after[close + 1..];
                }
            },
            // No closing brace, or another `{` first: this one is plain text.
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn values_are_not_expanded_again() {
        let v = vars(&[("cpu", "12"), ("hello.a", "{cpu}"), ("hello.b", "{hello.a}")]);
        assert_eq!(
            resolve_vars("{hello.a} {hello.b} {cpu}", &v),
            "{cpu} {hello.a} 12"
        );
        // A value naming a variable that comes later in the map stays as written too.
        let v = vars(&[("hello.a", "{hello.b}"), ("hello.b", "X")]);
        assert_eq!(resolve_vars("{hello.a}|{hello.b}", &v), "{hello.b}|X");
    }

    #[test]
    fn unknown_and_odd_braces_stay_literal() {
        let v = vars(&[("cpu", "12"), ("ram", "40")]);
        assert_eq!(resolve_vars("{nope} {cpu", &v), "{nope} {cpu");
        assert_eq!(resolve_vars("{{cpu}} }{", &v), "{12} }{");
        assert_eq!(resolve_vars("{cpu_usage}% {ram_usage}%", &v), "12% 40%");
        assert_eq!(resolve_vars("π {cpu} ✓", &v), "π 12 ✓");
    }
}
