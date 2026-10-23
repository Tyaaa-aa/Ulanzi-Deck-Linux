#![allow(clippy::unwrap_used)]

use ulanzi_plugin::{json, testing::Harness};
use ulanzi_plugin_hello::Hello;

#[test]
fn count_starts_from_settings_and_counts() {
    let mut h = Harness::<Hello>::new();
    h.init(json!({"start": 41})).unwrap();
    assert_eq!(h.state("count"), Some("41"));
    h.action("count", json!({})).unwrap();
    assert_eq!(h.state("count"), Some("42"));
}

#[test]
fn greet_needs_a_name() {
    let mut h = Harness::<Hello>::new();
    h.init(json!({})).unwrap();
    h.action("greet", json!({"name": "Ana"})).unwrap();
    assert_eq!(h.state("greeting"), Some("Hello, Ana!"));
    assert_eq!(h.action("greet", json!({})), Err("greet needs a name".into()));
    assert_eq!(h.action("nope", json!({})), Err("unknown action: nope".into()));
}
