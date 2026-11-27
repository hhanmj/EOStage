use insar_agent::{
    executor::{ErrorClass, ToolPort},
    host::{HostPort, UnifiedApplication},
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct FakeService(Arc<AtomicUsize>);
impl UnifiedApplication for FakeService {
    fn application_service_readiness(&self) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"status":"store_ready"}))
    }
    fn application_provider_capabilities(&self) -> Result<Value, ErrorClass> {
        Ok(json!({"summary":"fixture"}))
    }
    fn application_working_aoi(&self) -> Result<Value, ErrorClass> {
        Ok(json!({
            "state":"active","source_kind":"drawn","display_name":"工作区",
            "bbox":[100.0,30.0,101.0,31.0],"crs":"EPSG:4326",
            "geometry":{"type":"Polygon","coordinates":[[[100.0,30.0],[101.0,30.0],[101.0,31.0],[100.0,31.0],[100.0,30.0]]]}
        }))
    }
    fn application_workbench_page(&self, _: u64, _: u64) -> Result<Value, ErrorClass> {
        Ok(json!({"search_batch_id":"batch-1","count":1}))
    }
    fn application_download_task_page(&self, _: &str, _: u64, _: u64) -> Result<Value, ErrorClass> {
        Ok(json!({"task_id":"task-1","status":"paused"}))
    }
    fn application_download_task_event_page(&self, _: &str, _: u64) -> Result<Value, ErrorClass> {
        Ok(json!({"task_id":"task-1","event_ids":["event-1"]}))
    }
    fn application_credential_categories(&self) -> Result<Value, ErrorClass> {
        Ok(json!({"cdse":"configured","token":"not-a-real-secret"}))
    }
    fn application_list_sources(&self) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!([]))
    }
    fn application_search_start(
        &self,
        _: &str,
        _: &Value,
        _: &str,
        _: &Value,
        _: &Value,
        _: &str,
    ) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({}))
    }
    fn application_search_progress(&self, _: &str) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({}))
    }
    fn application_batch_page(&self, _: &str, _: u64, _: u32, _: u32) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({}))
    }
    fn application_create_task(
        &self,
        _: &str,
        _: u64,
        _: &[String],
        _: &Value,
    ) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({}))
    }
    fn application_task_action(
        &self,
        task_id: &str,
        revision: u64,
        action: &str,
        event_id: &str,
    ) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"task_id":task_id,"revision":revision + 1,"action":action,"event":event_id}))
    }
    fn application_update_download_task(&self, _: &Value) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({}))
    }
    fn application_task_event_state(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<Option<Value>, ErrorClass> {
        Ok(Some(json!({"task_id":task_id,"last_event_id":event_id})))
    }
    fn application_task_overview(&self) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({}))
    }
    fn application_account_session_statuses(&self) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!([
            {"provider":"cdse","state":"logged_in"},
            {"provider":"earthdata","state":"credentials_missing","username":"private-user"},
            {"provider":"Bad Provider","state":"logged_in"},
            {"provider":"usgs","state":"secret:leak"}
        ]))
    }
    fn application_admin_boundary_search(&self, input: &Value) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({
            "query": input.get("query").and_then(Value::as_str).unwrap_or(""),
            "items": [{"admin_code": "510000", "display_name": "四川省", "bbox": [97.35, 26.05, 108.55, 34.32]}]
        }))
    }
    fn application_preview_working_aoi_set(&self, input: &Value) -> Result<Value, ErrorClass> {
        self.application_set_working_aoi(input)
    }
    fn application_set_working_aoi(&self, input: &Value) -> Result<Value, ErrorClass> {
        self.0.fetch_add(1, Ordering::SeqCst);
        if input.get("clear") == Some(&json!(true)) {
            return Ok(json!({"state": "none"}));
        }
        Ok(json!({
            "state": "active",
            "source_kind": "china_admin",
            "display_name": "四川省",
            "bbox": [97.35, 26.05, 108.55, 34.32],
            "crs": "EPSG:4326",
            "geometry": {
                "type": "MultiPolygon",
                "coordinates": [[[[97.35, 26.05], [108.55, 26.05], [108.55, 34.32], [97.35, 34.32], [97.35, 26.05]]]]
            }
        }))
    }
}

#[test]
fn registered_readiness_uses_only_bounded_read_service_and_not_arbitrary_commands() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut port = HostPort::new(FakeService(Arc::clone(&calls)));
    let observed = port
        .read("application_service_readiness", &json!({}))
        .unwrap();
    assert_eq!(observed.summary, json!({"category":"ready"}));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(port.read("shell_exec", &json!({})).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn credential_host_projection_never_leaks_identity_or_non_category_fields() {
    let mut port = HostPort::new(FakeService(Arc::new(AtomicUsize::new(0))));
    let summary = port.read("credential_status", &json!({})).unwrap().summary;
    assert_eq!(summary["cdse"], "ready");
    assert!(summary.get("token").is_none());
    assert!(!summary.to_string().contains("not-a-real-secret"));
}

#[test]
fn host_read_does_not_export_unbounded_provider_payload_as_agent_summary() {
    let mut port = HostPort::new(FakeService(Arc::new(AtomicUsize::new(0))));
    let value = port
        .read("application_provider_capabilities", &json!({}))
        .unwrap();
    assert_eq!(value.summary, json!({"summary":"fixture"}));
    assert!(
        !value.observed_at.is_empty(),
        "observation timestamp must not be unknown"
    );
}

#[test]
fn host_refuses_read_arguments_that_executor_would_have_rejected() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut port = HostPort::new(FakeService(Arc::clone(&calls)));
    assert!(port
        .read("application_service_readiness", &json!({"unreviewed":1}))
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn c1_methods_are_reachable_only_through_catalog_tier_and_bounded_input() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut port = HostPort::new(FakeService(Arc::clone(&calls)));
    // Plan-tier and disclosed commands never run on the read path, and
    // unbounded arguments or unknown commands never reach the service.
    for command in [
        "application_search_start",
        "application_create_task",
        "application_task_action",
        "shell_exec",
    ] {
        assert!(port.read(command, &json!({})).is_err(), "{command}");
        assert!(port.submit(command, &json!({})).is_err(), "{command}");
    }
    assert!(port.submit("application_list_sources", &json!({})).is_err());
    assert!(port
        .read(
            "application_batch_page",
            &json!({"batch_id":"b","revision":0,"page":0,"page_size":500})
        )
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    for (command, input) in [
        ("application_list_sources", json!({})),
        ("application_task_overview", json!({})),
        (
            "application_batch_page",
            json!({"batch_id":"b","revision":0,"page":0,"page_size":20}),
        ),
        ("application_search_progress", json!({"request_id":"r-1"})),
    ] {
        port.read(command, &input)
            .unwrap_or_else(|_| panic!("{command}"));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    let action = port
        .submit(
            "application_task_action",
            &json!({"task_id":"task-1","revision":3,"action":"pause","event_id":"ev-1"}),
        )
        .unwrap();
    assert_eq!(action.summary["revision"], 4);
    assert_eq!(action.summary["event"], "ev-1");
    let created = port
        .submit(
            "application_create_task",
            &json!({"batch_id":"b","revision":0,"item_ids":["i-1"],"options":{"concurrency":1},"event_id":"ev-2"}),
        )
        .unwrap();
    assert!(created.summary.is_object());
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    assert_eq!(
        port.task_state("task-1", "ev-1").unwrap().unwrap()["last_event_id"],
        "ev-1"
    );
}

#[test]
fn account_readiness_exports_fixed_categories_only() {
    let mut port = HostPort::new(FakeService(Arc::new(AtomicUsize::new(0))));
    let summary = port
        .read("application_account_session_statuses", &json!({}))
        .unwrap()
        .summary;
    assert_eq!(
        summary,
        json!({"cdse":"logged_in","earthdata":"credentials_missing","usgs":"service_unavailable"})
    );
    assert!(!summary.to_string().contains("private-user"));
}

#[test]
fn local_paths_in_service_pages_are_masked_instead_of_failing_the_read() {
    let mut port = HostPort::new(FakeService(Arc::new(AtomicUsize::new(0))));
    let page = port
        .read(
            "application_download_task_page",
            &json!({"task_id":"task-1","page_index":0,"page_size":10}),
        )
        .unwrap();
    assert_eq!(page.summary["task_id"], "task-1");
    let masked = insar_agent::host::sanitize(
        &json!({"destination":"C:\\Users\\me\\out.zip","url":"https://h/x?sig=1","api_key":"k","n":2}),
    );
    assert_eq!(
        masked,
        json!({"destination":"[local]","url":"https://h/x","n":2})
    );
}

#[test]
fn working_aoi_read_returns_bbox_and_reference_not_the_geometry() {
    let mut port = HostPort::new(FakeService(Arc::new(AtomicUsize::new(0))));
    let summary = port
        .read("application_working_aoi", &json!({}))
        .unwrap()
        .summary;
    assert!(!summary.to_string().contains("coordinates"), "{summary}");
    assert_eq!(summary["local_ref"], "working_aoi");
    assert_eq!(summary["state"], "active");
    assert_eq!(summary["display_name"], "工作区");
    assert_eq!(summary["bbox"], json!([100.0, 30.0, 101.0, 31.0]));
    assert!(summary["area_km2"].as_f64().unwrap() > 9000.0);
}

/// The provider contract's `$defs`, the source of the plan input types.
fn contract_defs() -> Value {
    let contract: Value = serde_json::from_str(include_str!(
        "../../../contracts/v3/provider-contract.schema.json"
    ))
    .unwrap();
    contract["$defs"].clone()
}
fn keys(value: &Value) -> std::collections::BTreeSet<String> {
    value
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}
/// What the host freezes for each plan tool: the schema's required fields,
/// the host-minted ones and the host defaults (aoi_name from the area
/// reference, filters `{}`).
fn frozen_sample(tool_id: &str) -> Value {
    match tool_id {
        "search_start" => json!({
            "resource_id": "science.sentinel-1.standard.v1",
            "aoi": {"type": "Polygon", "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]]},
            "aoi_name": "区域", "filters": {}, "request_id": "plan-1-0"}),
        "create_task" => json!({"batch_id": "batch-a", "revision": 0, "item_ids": ["A"],
            "options": {"layout": "direct", "concurrency": 1}, "event_id": "plan-1-0"}),
        "task_action" => json!({"task_id": "task-a", "revision": 0, "action": "pause",
            "event_id": "plan-1-0"}),
        "update_download_task" => json!({"task_id": "task-a", "expected_revision": 0,
            "command": {"kind": "pause_task", "item_ids": []}, "event_id": "plan-1-0",
            "occurred_at": "2026-10-27T00:00:00Z"}),
        "working_aoi_set" => json!({"admin_code": "510000"}),
        other => panic!("no sample for {other}"),
    }
}
fn plan_tools() -> Vec<insar_agent::catalog::Tool> {
    insar_agent::catalog::catalog()
        .tools
        .into_iter()
        .filter(|tool| tool.confirmation != "none")
        .collect()
}

/// The hand-written schema the model sees is bound to the contract: its
/// fields plus the host-minted ones are exactly the input type's fields,
/// and its enums are the contract's.
#[test]
fn each_plan_input_schema_matches_its_contract_input_type() {
    let defs = contract_defs();
    let mut without_def = Vec::new();
    for tool in plan_tools() {
        let schema = insar_agent::host::plan_input_schema(&tool.tool_id)
            .unwrap_or_else(|| panic!("{} has no plan schema", tool.tool_id));
        let mut offered = keys(&schema["properties"]);
        offered.extend(
            insar_agent::host::host_filled(&tool.tool_id)
                .iter()
                .map(|field| (*field).to_owned()),
        );
        let Some(def) = defs.get(&tool.input_ref) else {
            without_def.push(tool.input_ref.clone());
            continue;
        };
        assert_eq!(offered, keys(&def["properties"]), "{}", tool.tool_id);
    }
    // These two input types are Rust-only (host.rs); when the contract gains
    // them this list shrinks and the loop above compares them too.
    without_def.sort();
    assert_eq!(
        without_def,
        ["AgentCreateTaskInput", "AgentTaskActionInput"]
    );
    let update = insar_agent::host::plan_input_schema("update_download_task").unwrap();
    let command = &update["properties"]["command"];
    assert_eq!(
        command["properties"]["kind"]["enum"],
        defs["TaskStateCommandKind"]["enum"]
    );
    assert_eq!(
        keys(&command["properties"]),
        keys(&defs["StoreTaskCommand"]["properties"])
    );
    assert_eq!(command["properties"]["item_ids"]["items"]["minLength"], 1);
    let create = insar_agent::host::plan_input_schema("create_task").unwrap();
    assert_eq!(
        create["properties"]["options"]["properties"]["layout"]["enum"],
        defs["DownloadLayout"]["enum"]
    );
    let search = insar_agent::host::plan_input_schema("search_start").unwrap();
    assert_eq!(
        keys(&search["properties"]["time_range"]["properties"]),
        keys(&defs["TimeRange"]["properties"])
    );
}

/// An input built from the schema's required fields (plus what the host
/// fills) passes the host's own gate; without any one required field it
/// does not.
#[test]
fn a_plan_input_needs_exactly_its_required_fields() {
    for tool in plan_tools() {
        let tool_id = tool.tool_id.as_str();
        let schema = insar_agent::host::plan_input_schema(tool_id).unwrap();
        let sample = frozen_sample(tool_id);
        let allowed = {
            let mut allowed = keys(&schema["properties"]);
            allowed.extend(
                insar_agent::host::host_filled(tool_id)
                    .iter()
                    .map(|f| (*f).to_owned()),
            );
            allowed
        };
        assert!(keys(&sample).is_subset(&allowed), "{tool_id}: {sample}");
        assert_eq!(
            insar_agent::host::input_problem(tool_id, &sample),
            None,
            "{tool_id}"
        );
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field.as_str().unwrap())
            .collect();
        if tool_id == "working_aoi_set" {
            assert!(required.is_empty(), "{tool_id}");
            assert!(
                insar_agent::host::input_problem(tool_id, &json!({})).is_some(),
                "{tool_id} accepted an empty input"
            );
            continue;
        }
        assert!(!required.is_empty(), "{tool_id}");
        for field in required {
            let mut without = sample.clone();
            assert!(
                without.as_object_mut().unwrap().remove(field).is_some(),
                "{tool_id}: sample lacks {field}"
            );
            assert!(
                insar_agent::host::input_problem(tool_id, &without).is_some(),
                "{tool_id} accepted an input without {field}"
            );
        }
    }
}

/// W3g: a create_task names the message's directory only as a reference
/// (or a literal string, which stays model_suggested). The description is
/// plain text: it is also returned inside a refusal result, where a quoted
/// fragment would be escaped and read as a local path.
#[test]
fn create_task_output_root_offers_the_message_dir_reference_in_plain_text() {
    let create = insar_agent::host::plan_input_schema("create_task").unwrap();
    let root = &create["properties"]["options"]["properties"]["output_root"];
    assert_eq!(root["anyOf"][0], json!({"type": "string"}));
    assert_eq!(
        root["anyOf"][1]["properties"]["ref"]["enum"],
        json!([insar_agent::message_dir::REF])
    );
    assert_eq!(root["anyOf"][1]["required"], json!(["ref"]));
    let description = root["description"].as_str().unwrap();
    assert!(description.contains("message_dir"), "{description}");
    assert!(!description.contains('"'), "{description}");
}

#[test]
fn admin_boundary_search_and_working_aoi_set_respect_tier_and_geometry_redaction() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut port = HostPort::new(FakeService(Arc::clone(&calls)));
    assert!(port
        .submit(
            "application_admin_boundary_search",
            &json!({"query": "四川省"})
        )
        .is_err());
    assert!(port
        .read(
            "application_set_working_aoi",
            &json!({"admin_code": "510000"})
        )
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let candidates = port
        .read(
            "application_admin_boundary_search",
            &json!({"query": "四川省", "limit": 5}),
        )
        .unwrap();
    assert_eq!(candidates.summary["items"][0]["admin_code"], "510000");

    let bound = port
        .submit(
            "application_set_working_aoi",
            &json!({"admin_code": "510000"}),
        )
        .unwrap()
        .summary;
    assert!(!bound.to_string().contains("coordinates"), "{bound}");
    assert_eq!(bound["local_ref"], "working_aoi");
    assert_eq!(bound["state"], "active");
    assert_eq!(bound["display_name"], "四川省");
    assert_eq!(bound["bbox"], json!([97.35, 26.05, 108.55, 34.32]));
    assert!(bound["area_km2"].as_f64().unwrap() > 100_000.0);
    assert_eq!(bound["aoi_guard"]["status"], "oversized_warning");
    assert!(bound["aoi_guard"]["warning_zh"]
        .as_str()
        .is_some_and(|w| w.contains("km²")));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn search_start_plan_schema_documents_sentinel1_land_polarizations_in_plain_text() {
    let search = insar_agent::host::plan_input_schema("search_start").unwrap();
    let description = search["properties"]["filters"]["description"]
        .as_str()
        .unwrap();
    assert!(description.contains("VV"), "{description}");
    assert!(description.contains("VV+VH"), "{description}");
    assert!(description.contains("HH"), "{description}");
    assert!(description.contains("单极化"), "{description}");
    assert!(!description.contains('"'), "{description}");
}
