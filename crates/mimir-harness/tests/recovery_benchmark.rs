//! Regression gates for recovery-benchmark scenario data.

#![allow(clippy::panic)]

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("Cargo.toml").exists() && dir.join("LICENSE").exists() {
            let cargo_toml = dir.join("Cargo.toml");
            let text = fs::read_to_string(&cargo_toml)
                .unwrap_or_else(|error| panic!("read {}: {error}", cargo_toml.display()));
            if text.contains("[workspace]") {
                return dir;
            }
        }
        assert!(
            dir.pop(),
            "could not find Mimir repo root from CARGO_MANIFEST_DIR"
        );
    }
}

#[test]
fn recovery_benchmark_scenarios_are_structured_data() {
    let scenarios_dir = repo_root()
        .join("benchmarks")
        .join("recovery")
        .join("scenarios");
    let entries = fs::read_dir(&scenarios_dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", scenarios_dir.display()));
    let mut scenario_files = Vec::new();
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("read {} entry: {error}", scenarios_dir.display()))
            .path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            scenario_files.push(path);
        }
    }
    scenario_files.sort();

    assert!(
        !scenario_files.is_empty(),
        "Category 9 recovery benchmark scenarios must be machine-readable JSON data"
    );
    for path in scenario_files {
        validate_scenario(&path);
    }
}

fn validate_scenario(path: &Path) {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let value: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse {} as JSON: {error}", path.display()));

    assert_eq!(
        required_u64(path, &value, "schema_version"),
        1,
        "{}: schema_version must be 1",
        path.display()
    );
    let id = required_str(path, &value, "id");
    let stem = path.file_stem().and_then(|stem| stem.to_str());
    assert_eq!(stem, Some(id), "{}: id must match filename", path.display());
    assert!(
        matches!(
            required_str(path, &value, "status"),
            "illustrative" | "production"
        ),
        "{}: status must be illustrative or production",
        path.display()
    );
    required_str(path, &value, "title");
    required_str(path, &value, "situation");
    required_str(path, &value, "cold_start_prompt");
    required_str(path, &value, "work_instruction_test");
    assert!(
        required_u64(path, &value, "cutoff_minutes") > 0,
        "{}: cutoff_minutes must be positive",
        path.display()
    );

    validate_baselines(path, required_array(path, &value, "baselines"));
    validate_ground_truth(path, required_array(path, &value, "ground_truth"));
    validate_staleness(path, required_array(path, &value, "staleness_tests"));
}

fn validate_baselines(path: &Path, baselines: &[Value]) {
    let mut ids = BTreeSet::new();
    for baseline in baselines {
        let id = required_str(path, baseline, "id");
        required_str(path, baseline, "name");
        required_str(path, baseline, "preserved_state");
        ids.insert(id.to_string());
    }
    assert_eq!(
        ids,
        BTreeSet::from([
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string()
        ]),
        "{}: baselines must define A, B, C, and D exactly once",
        path.display()
    );
}

fn validate_ground_truth(path: &Path, items: &[Value]) {
    assert!(
        !items.is_empty(),
        "{}: ground_truth must contain at least one item",
        path.display()
    );
    let mut ids = BTreeSet::new();
    for item in items {
        let id = required_str(path, item, "id");
        assert!(
            ids.insert(id.to_string()),
            "{}: duplicate ground_truth id {id}",
            path.display()
        );
        let category = required_str(path, item, "category");
        assert!(
            matches!(
                category,
                "operator_profile"
                    | "project_state"
                    | "load_bearing_decision"
                    | "recent_feedback_open_work"
            ),
            "{}: invalid ground_truth category {category}",
            path.display()
        );
        required_str(path, item, "text");
        assert!(
            !required_array(path, item, "source_refs").is_empty(),
            "{}: ground_truth item {id} needs at least one source_ref",
            path.display()
        );
        let auto_fail = required_bool(path, item, "auto_fail_if_wrong");
        if category == "load_bearing_decision" {
            assert!(
                auto_fail,
                "{}: load-bearing decision {id} must be auto-fail if wrong",
                path.display()
            );
        }
    }
}

fn validate_staleness(path: &Path, items: &[Value]) {
    let mut ids = BTreeSet::new();
    for item in items {
        let id = required_str(path, item, "id");
        assert!(
            ids.insert(id.to_string()),
            "{}: duplicate staleness test id {id}",
            path.display()
        );
        required_str(path, item, "superseded_fact");
        required_str(path, item, "current_fact");
        required_str(path, item, "expected_behavior");
    }
}

fn required_str<'a>(path: &Path, value: &'a Value, field: &str) -> &'a str {
    match value.get(field).and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => text,
        _ => panic!("{}: missing non-empty string field {field}", path.display()),
    }
}

fn required_u64(path: &Path, value: &Value, field: &str) -> u64 {
    match value.get(field).and_then(Value::as_u64) {
        Some(number) => number,
        None => panic!("{}: missing unsigned integer field {field}", path.display()),
    }
}

fn required_bool(path: &Path, value: &Value, field: &str) -> bool {
    match value.get(field).and_then(Value::as_bool) {
        Some(flag) => flag,
        None => panic!("{}: missing boolean field {field}", path.display()),
    }
}

fn required_array<'a>(path: &Path, value: &'a Value, field: &str) -> &'a [Value] {
    match value.get(field).and_then(Value::as_array) {
        Some(items) => items,
        None => panic!("{}: missing array field {field}", path.display()),
    }
}
