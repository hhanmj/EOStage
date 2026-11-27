use insar_agent::catalog::{catalog, credential_categories, validate};
use serde_json::json;
use std::collections::HashSet;

#[test]
fn catalog_read_tools_are_safe_registered_and_tier_zero() {
    let catalog = catalog();
    assert_eq!(catalog.schema_version, 1);
    validate(&catalog).unwrap();
    let expected: HashSet<&str> = [
        "application_service_readiness",
        "application_provider_capabilities",
        "application_working_aoi",
        "application_workbench_page",
        "application_download_task_page",
        "application_download_task_event_page",
        "credential_status",
        "application_list_sources",
        "application_account_session_statuses",
        "application_task_overview",
        "application_batch_page",
        "application_search_progress",
        "application_admin_boundary_search",
    ]
    .into_iter()
    .collect();
    let commands: HashSet<_> = catalog
        .tools
        .iter()
        .filter(|tool| tool.effect == "read")
        .map(|tool| tool.target_command.as_str())
        .collect();
    assert_eq!(
        commands, expected,
        "only registered read-only commands belong in the catalog read tier"
    );
    let tiers: Vec<_> = catalog
        .tools
        .iter()
        .filter(|tool| tool.effect != "read")
        .map(|tool| (tool.tool_id.as_str(), tool.confirmation.as_str()))
        .collect();
    assert_eq!(
        tiers,
        [
            ("update_download_task", "plan_confirm"),
            ("search_start", "disclose"),
            ("create_task", "plan_confirm"),
            ("task_action", "plan_confirm"),
            ("working_aoi_set", "disclose"),
        ]
    );
    for tool in catalog.tools.iter().filter(|tool| tool.effect == "read") {
        assert!(
            !tool.job.is_empty(),
            "each operator must serve a data preparation job"
        );
        assert_eq!(tool.effect, "read");
        assert_eq!(tool.confirmation, "none");
        assert!(!tool.evidence.is_empty());
    }
}

#[test]
fn rejects_unregistered_credential_and_write_commands() {
    for command in [
        "credential_reveal",
        "credential_save_cdse",
        "application_update_download_task",
        "ui_smoke_test",
    ] {
        let mut changed = catalog();
        changed.tools[0].target_command = command.into();
        assert!(validate(&changed).is_err(), "{command}");
    }
    let mut changed = catalog();
    changed.tools[0].effect = "commit".into();
    assert!(validate(&changed).is_err());
    let mut changed = catalog();
    changed.tools[0].confirmation = "plan_confirm".into();
    assert!(validate(&changed).is_err());
}

#[test]
fn credential_projection_drops_all_secret_and_identity_fields() {
    let projected = credential_categories(
        &json!({"cdse":"configured", "usgs":"none", "earthdata":"login:private-user", "token":"test-secret-not-real", "earthdata_input":{"password":"test-secret-not-real"}}),
    );
    assert_eq!(projected["cdse"], "ready");
    assert_eq!(projected["usgs"], "failed");
    assert_eq!(projected["earthdata"], "ready");
    let unavailable =
        credential_categories(&json!({"cdse":"unavailable","earthdata":"incomplete"}));
    assert_eq!(
        unavailable["cdse"], "failed",
        "unavailable keyring is not ready"
    );
    assert_eq!(
        unavailable["earthdata"], "failed",
        "half-configured login is not ready"
    );
    let serialized = projected.to_string();
    assert!(!serialized.contains("private-user"));
    assert!(!serialized.contains("test-secret"));
}

#[test]
fn catalog_targets_match_desktop_command_registration() {
    let registrations = include_str!("../../../src-tauri/src/lib.rs");
    let functions = include_str!("../../../src-tauri/src/commands.rs");
    for tool in catalog().tools {
        assert!(registrations.contains(&format!("commands::{},", tool.target_command)));
        assert!(functions.contains(&format!("fn {}(", tool.target_command)));
    }
}

/// A1 §3.1 floor that no permission mode changes: the catalog has no tool
/// that runs an arbitrary command, deletes downloaded results or the Store,
/// or changes account credentials. Checked by tool id, target command and
/// description, against the catalog snapshot below.
#[test]
fn no_tool_runs_commands_deletes_results_or_the_store_or_writes_credentials() {
    let catalog = catalog();
    let ids: Vec<&str> = catalog.tools.iter().map(|t| t.tool_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "service_readiness",
            "capabilities_get",
            "working_aoi_get",
            "workbench_page",
            "task_page",
            "task_event_page",
            "credential_readiness",
            "update_download_task",
            "source_list",
            "account_readiness",
            "task_overview",
            "batch_page",
            "search_progress",
            "search_start",
            "create_task",
            "task_action",
            "admin_boundary_search",
            "working_aoi_set",
        ],
        "a new tool must be reviewed against the permission-mode floor"
    );
    let forbidden_words = [
        "shell",
        "exec",
        "command",
        "script",
        "process",
        "spawn",
        "terminal",
        "delete",
        "remove",
        "erase",
        "wipe",
        "purge",
        "drop",
        "unlink",
        "store_reset",
        "login",
        "logout",
        "password",
        "credential_save",
        "credential_clear",
        "credential_reveal",
        "secret",
        "token",
        "netrc",
    ];
    for tool in &catalog.tools {
        let id = tool.tool_id.to_ascii_lowercase();
        let target = tool.target_command.to_ascii_lowercase();
        let description = tool.description.to_ascii_lowercase();
        for word in forbidden_words {
            assert!(
                !id.contains(word) && !target.contains(word),
                "{}: {word}",
                tool.tool_id
            );
            assert!(!description.contains(word), "{}: {word}", tool.tool_id);
        }
        // Credential tools may only report categories.
        if id.contains("credential") || target.contains("credential") {
            assert_eq!(tool.effect, "read", "{}", tool.tool_id);
            assert_eq!(tool.redaction, ["credential_category"], "{}", tool.tool_id);
        }
    }
    // The task action vocabulary has no delete: archive keeps the task.
    let action = catalog
        .tools
        .iter()
        .find(|t| t.tool_id == "task_action")
        .unwrap();
    assert!(action.description.contains("archive"));
    for word in ["delete", "remove"] {
        assert!(!action.description.to_ascii_lowercase().contains(word));
    }
}
