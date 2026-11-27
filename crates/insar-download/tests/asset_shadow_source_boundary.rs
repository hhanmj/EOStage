use std::fs;
use std::path::PathBuf;

#[test]
fn shadow_modules_cannot_import_executable_or_network_boundaries() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/asset_resolver");
    let sources = [
        "shadow_types.rs",
        "shadow_plan.rs",
        "shadow_compare.rs",
        "shadow_runtime.rs",
    ]
    .into_iter()
    .map(|name| fs::read_to_string(root.join(name)).expect("Shadow source exists"))
    .collect::<Vec<_>>()
    .join("\n");

    for forbidden in [
        "HttpTransfer",
        "TransferRequest",
        "ResolvedAsset",
        "reqwest::blocking",
        "resolve_earthdata_credential",
        "resolve_earthdata_burst_credential",
        "std::net::ToSocketAddrs",
        "std::fs",
    ] {
        assert!(
            !sources.contains(forbidden),
            "Shadow source imported {forbidden}"
        );
    }
    assert!(!sources.contains("Serialize"));
    assert!(!sources.contains("Deserialize"));
    assert!(!sources.contains("serde::"));
}
