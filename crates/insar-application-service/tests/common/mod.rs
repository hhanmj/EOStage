use insar_application_service::{
    ApplicationService, WorkspaceNamedNodeRequest, WorkspaceOpenRequest,
};

fn safe_label(label: &str) -> String {
    let value: String = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let value = value.trim_matches('-');
    if value.is_empty() {
        "fixture".to_owned()
    } else {
        value.to_owned()
    }
}

pub fn ensure_fixture_region(service: &ApplicationService, label: &str) -> String {
    let safe = safe_label(label);
    let workspace_id = format!("workspace-fixture-{safe}");
    let context = service
        .workspace_open(&WorkspaceOpenRequest {
            request_id: format!("{safe}-workspace-open"),
            workspace_id: workspace_id.clone(),
            workspace_root: std::env::temp_dir(),
            workspace_name: format!("{label} workspace"),
            expected_revision: None,
        })
        .expect("fixture workspace must open");
    let project = service
        .project_create(&WorkspaceNamedNodeRequest {
            request_id: format!("{safe}-project-create"),
            parent_id: workspace_id,
            expected_revision: context.revision,
            name: format!("{label} project"),
        })
        .expect("fixture project must create");
    let project_id = project.project_id.expect("fixture project must have an ID");
    service
        .region_create(&WorkspaceNamedNodeRequest {
            request_id: format!("{safe}-region-create"),
            parent_id: project_id,
            expected_revision: project.revision,
            name: format!("{label} region"),
        })
        .expect("fixture region must create")
        .region_id
        .expect("fixture region must have an ID")
}
