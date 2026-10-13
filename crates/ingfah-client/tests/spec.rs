#![expect(clippy::expect_used, reason = "test helpers panic on broken fixtures")]

use std::collections::BTreeMap;

use ingfah_client::routes::{Method, ROUTES};
use serde_yaml::Value;

const SPEC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../spec/openapi.public.v1.yaml"
);

/// Every API-key operation in the vendored spec: (method, path) -> scope.
fn spec_operations() -> BTreeMap<(String, String), String> {
    let text = std::fs::read_to_string(SPEC).expect("read spec");
    let spec: Value = serde_yaml::from_str(&text).expect("parse spec");
    let paths = spec["paths"].as_mapping().expect("paths");
    let mut ops = BTreeMap::new();
    for (path, item) in paths {
        let path = path.as_str().expect("path key").to_owned();
        for (method, op) in item.as_mapping().expect("path item") {
            let method = method.as_str().expect("method").to_ascii_uppercase();
            if Method::parse(&method).is_none() {
                continue;
            }
            let uses_api_key = op["security"]
                .as_sequence()
                .is_some_and(|s| s.iter().any(|req| req.get("apiKey").is_some()));
            if !uses_api_key {
                continue;
            }
            let description = op["description"].as_str().unwrap_or_default();
            let scope = description
                .split("API key scope: `")
                .nth(1)
                .and_then(|rest| rest.split('`').next())
                .unwrap_or_else(|| panic!("{method} {path} names no scope"));
            ops.insert((method, path.clone()), scope.to_owned());
        }
    }
    ops
}

#[test]
fn spec_has_api_key_operations() {
    assert!(spec_operations().len() >= 60);
}

#[test]
fn registry_covers_every_spec_operation_with_the_same_scope() {
    let mut problems = Vec::new();
    for ((method, path), scope) in spec_operations() {
        let row = ROUTES
            .iter()
            .find(|r| r.method.as_str() == method && r.path == path);
        match row {
            None => problems.push(format!("missing {method} {path} ({scope})")),
            Some(r) if r.scope != scope => problems.push(format!(
                "{method} {path}: registry scope {} but spec says {scope}",
                r.scope
            )),
            Some(_) => {}
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
