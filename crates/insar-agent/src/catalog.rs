use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use thiserror::Error;

mod generated {
    include!("generated_catalog.rs");
}

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub catalog_id: String,
    pub tools: Vec<Tool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tool {
    pub tool_id: String,
    pub version: u32,
    pub description: String,
    pub target_command: String,
    pub input_ref: String,
    pub output_ref: String,
    pub job: String,
    pub effect: String,
    pub confirmation: String,
    pub evidence: Vec<String>,
    pub redaction: Vec<String>,
    pub deterministic: bool,
    pub page_limit: Option<u32>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("invalid agent tool catalog: {0}")]
    Invalid(String),
    #[error("agent tool not registered: {0}")]
    UnknownTool(String),
}

// These command symbols are the only Tauri entry points admitted in A1a; their
// command registration is checked against src-tauri/src/lib.rs in catalog tests.
const REGISTERED_READ_COMMANDS: &[&str] = &[
    "application_service_readiness",
    "application_provider_capabilities",
    "application_working_aoi",
    "application_admin_boundary_search",
    "application_workbench_page",
    "application_download_task_page",
    "application_download_task_event_page",
    "credential_status",
    "application_list_sources",
    "application_account_session_statuses",
    "application_task_overview",
    "application_batch_page",
    "application_search_progress",
];
// Provider requests or disclosed workspace mutations: disclosed per plan, never silent.
const REGISTERED_DISCLOSE_COMMANDS: &[&str] =
    &["application_search_start", "application_set_working_aoi"];
const REGISTERED_WRITE_COMMANDS: &[&str] = &[
    "application_update_download_task",
    "application_create_task",
    "application_task_action",
];
const REJECTED_COMMANDS: &[&str] = &[
    "credential_reveal",
    "credential_read_for_edit",
    "credential_earthdata_auth",
    "credential_bhoonidhi_auth",
    "application_cdse_login",
    "application_usgs_login",
    "application_cdse_logout",
    "application_usgs_logout",
    "acceptance_bhoonidhi_auth_generation",
    "acceptance_bhoonidhi_auth_revoke_once",
];

pub fn catalog() -> Catalog {
    serde_json::from_str(include_str!(
        "../../../contracts/v3/agent-tool-catalog.v1.json"
    ))
    .expect("embedded catalog is valid")
}

pub fn validate(catalog: &Catalog) -> Result<(), CatalogError> {
    if catalog.schema_version != 1 || catalog.catalog_id != "eostage.agent-tools.v1" {
        return Err(CatalogError::Invalid("version".into()));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for tool in &catalog.tools {
        if !seen.insert(tool.tool_id.as_str())
            || tool.version == 0
            || tool.description.is_empty()
            || tool.job.is_empty()
            || tool.evidence.is_empty()
            || tool.input_ref.is_empty()
            || tool.output_ref.is_empty()
            || tool.tool_id.trim() != tool.tool_id
        {
            return Err(CatalogError::Invalid(tool.tool_id.clone()));
        }
        if REJECTED_COMMANDS.contains(&tool.target_command.as_str())
            || tool.target_command.starts_with("credential_save_")
            || tool.target_command.starts_with("credential_clear_")
            || tool.target_command.starts_with("ui_smoke_")
            || !(REGISTERED_READ_COMMANDS.contains(&tool.target_command.as_str())
                || REGISTERED_DISCLOSE_COMMANDS.contains(&tool.target_command.as_str())
                || REGISTERED_WRITE_COMMANDS.contains(&tool.target_command.as_str()))
        {
            return Err(CatalogError::Invalid(tool.target_command.clone()));
        }
        let write = matches!(tool.effect.as_str(), "commit" | "external_write");
        let valid_tier = if write {
            tool.confirmation == "plan_confirm"
                && REGISTERED_WRITE_COMMANDS.contains(&tool.target_command.as_str())
        } else if tool.effect == "read_external" {
            tool.confirmation == "disclose"
                && REGISTERED_DISCLOSE_COMMANDS.contains(&tool.target_command.as_str())
        } else {
            tool.effect == "read"
                && tool.confirmation == "none"
                && REGISTERED_READ_COMMANDS.contains(&tool.target_command.as_str())
        };
        if !valid_tier
            || (tool.target_command == "application_update_download_task"
                && tool.tool_id != "update_download_task")
        {
            return Err(CatalogError::Invalid(tool.tool_id.clone()));
        }
        if matches!(
            tool.tool_id.as_str(),
            "credential_readiness" | "account_readiness"
        ) && (tool.redaction != ["credential_category"]
            || tool.output_ref != "AgentCredentialCategoryOutput")
        {
            return Err(CatalogError::Invalid("credential projection".into()));
        }
    }
    if seen.len() != generated::TOOL_IDS.len()
        || !generated::TOOL_IDS.iter().all(|id| seen.contains(*id))
    {
        return Err(CatalogError::Invalid("generated binding drift".into()));
    }
    Ok(())
}

pub fn resolve(id: &str) -> Result<Tool, CatalogError> {
    let catalog = catalog();
    validate(&catalog)?;
    catalog
        .tools
        .into_iter()
        .find(|tool| tool.tool_id == id)
        .ok_or_else(|| CatalogError::UnknownTool(id.into()))
}

/// Projects existing desktop credential status to categories only. Never pass
/// its original object, masked identities, or empty edit fields to a model/MCP.
pub fn credential_categories(raw: &Value) -> Value {
    let mut output = serde_json::Map::new();
    for provider in [
        "cdse",
        "usgs",
        "earthdata",
        "bhoonidhi",
        "opentopography",
        "gacos",
    ] {
        let state = raw.get(provider).and_then(Value::as_str).unwrap_or("");
        output.insert(
            provider.to_owned(),
            json!(if state.is_empty()
                || matches!(state, "none" | "failed" | "incomplete" | "unavailable")
            {
                "failed"
            } else {
                "ready"
            }),
        );
    }
    Value::Object(output)
}
