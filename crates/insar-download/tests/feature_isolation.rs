//! Production-configuration guard for the test-only free-space probe
//! (review of W2d): the `test-plenty-disk-space` feature is defined once in
//! insar-download's `[features]` and enabled only from `[dev-dependencies]`
//! tables, so no production build (no dev-dependencies) can turn the
//! disk-low guard into a no-op.

use std::fs;
use std::path::{Path, PathBuf};

const FEATURE: &str = "test-plenty-disk-space";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn workspace_members(root: &Path) -> Vec<String> {
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    let start = manifest.find("members = [").expect("workspace members");
    let end = start + manifest[start..].find(']').expect("members list end");
    manifest[start..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// Each line naming the feature, with the table it sits in.
fn mentions(manifest: &str) -> Vec<(String, String)> {
    let mut table = String::new();
    let mut found = Vec::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            table = trimmed.to_owned();
        } else if trimmed.contains(FEATURE) && !trimmed.starts_with('#') {
            found.push((table.clone(), trimmed.to_owned()));
        }
    }
    found
}

#[test]
fn the_test_probe_feature_is_enabled_only_from_dev_dependencies() {
    let root = workspace_root();
    let root_manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        mentions(&root_manifest).is_empty(),
        "the workspace manifest must not name {FEATURE}"
    );
    let members = workspace_members(&root);
    assert!(members
        .iter()
        .any(|member| member == "crates/insar-download"));
    let mut definitions = 0;
    let mut dev_enables = Vec::new();
    for member in &members {
        let manifest = fs::read_to_string(root.join(member).join("Cargo.toml"))
            .unwrap_or_else(|error| panic!("{member}/Cargo.toml: {error}"));
        for (table, line) in mentions(&manifest) {
            if member == "crates/insar-download"
                && table == "[features]"
                && line == format!("{FEATURE} = []")
            {
                definitions += 1;
            } else if table.ends_with("dev-dependencies]") {
                dev_enables.push(member.clone());
            } else {
                panic!("{member}/Cargo.toml names {FEATURE} outside [dev-dependencies]: {table} {line}");
            }
        }
    }
    assert_eq!(
        definitions, 1,
        "insar-download defines {FEATURE} exactly once, empty"
    );
    for expected in [
        "crates/insar-download",
        "crates/insar-application-service",
        "crates/insar-service",
        "src-tauri",
    ] {
        assert!(
            dev_enables.iter().any(|member| member == expected),
            "{expected} tests keep the plenty-of-space probe: {dev_enables:?}"
        );
    }
}
