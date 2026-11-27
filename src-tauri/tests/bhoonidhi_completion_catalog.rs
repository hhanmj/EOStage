//! Explicit, bounded production S-band authentication/catalogue acceptance.
//! No product download, credential echo, alternate auth source or fixture provider.
use eostage_desktop::dem_live_acceptance_support::{state, ApplicationServiceState};
use insar_application_service::ApplicationCatalogSearchAndFreezeRequest;
use serde_json::{json, Value};
use std::{fs, io::Write, path::PathBuf, time::Instant};

fn record(root: &std::path::Path, value: Value) {
    let mut events = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("catalogue-events.jsonl"))
        .unwrap();
    writeln!(events, "{value}").unwrap();
    fs::write(
        root.join("catalogue-result.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    println!("{value}");
}

struct SessionCleanup<'a>(&'a ApplicationServiceState);

fn shutdown_and_record(service: &ApplicationServiceState, root: &std::path::Path) {
    let started = Instant::now();
    let shutdown = service.shutdown();
    record(
        root,
        json!({"phase":"logout", "ok":shutdown.is_ok(),
        "elapsed_ms":started.elapsed().as_millis(),
        "code":shutdown.as_ref().err().map(|error| error.code()), "downloads_started":false}),
    );
    shutdown.unwrap();
}

impl Drop for SessionCleanup<'_> {
    fn drop(&mut self) {
        // Attempt cleanup even when an assertion fails after successful login.
        // The service takes its session first, so this never repeats a logout.
        let _ = self.0.shutdown();
    }
}

#[test]
#[ignore = "requires explicit owner-authorized login/catalogue request and a fresh isolated Store"]
fn saved_login_and_one_sband_catalogue_item_use_native_authority() {
    assert_eq!(
        std::env::var("INSAR_SBAND_LIVE_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var_os("INSAR_SBAND_ACCEPTANCE_ROOT").expect("fresh root"));
    let runtime =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("product runtime"));
    assert!(root.is_absolute() && !root.exists());
    fs::create_dir_all(&root).unwrap();
    let readiness = insar_download::desktop_bhoonidhi_readiness();
    record(
        &root,
        json!({"phase":"credential_configuration", "readiness":readiness,
        "authentication_attempted":false, "downloads_started":false}),
    );
    assert_eq!(
        readiness,
        insar_download::BhoonidhiCredentialReadiness::Ready,
        "saved Bhoonidhi configuration is required; do not echo credentials"
    );
    let service = state(root.join("tasks.sqlite3"), runtime.clone()).unwrap();
    let _cleanup = SessionCleanup(&service);
    let started = Instant::now();
    if let Err(error) = service.bhoonidhi_authentication_check() {
        record(
            &root,
            json!({"phase":"authentication", "ok":false, "code":error.code(),
            "elapsed_ms":started.elapsed().as_millis(),
            "catalogue_verified":false, "downloads_started":false}),
        );
        shutdown_and_record(&service, &root);
        panic!("native authentication failed: {}", error.code());
    }
    record(
        &root,
        json!({"phase":"authentication", "ok":true, "catalogue_verified":false,
        "elapsed_ms":started.elapsed().as_millis(),
        "downloads_started":false}),
    );
    let geometry = json!({"type":"Polygon", "coordinates":[[
        [68.0,6.0],[98.0,6.0],[98.0,38.0],[68.0,38.0],[68.0,6.0]
    ]]})
    .as_object()
    .unwrap()
    .clone();
    let request: ApplicationCatalogSearchAndFreezeRequest = serde_json::from_value(json!({
        "request_id":"sband-completion-catalogue", "batch_id":"sband-completion-batch",
        "created_at":"2026-09-08T00:00:00Z", "aoi_name":"bounded S-band catalogue acceptance",
        "aoi_geojson":geometry, "workbench_page_size":1,
        "search":{
            "logical_dataset":"sar.nisar.s.ssar.rslc.v1", "provider_id":"isro.nrsc.bhoonidhi",
            "native_collection_id":"NISAR_SSAR_RSLC", "aoi_geojson":geometry,
            "cursor":null, "result_limit":1,
            "filters":{"online":"Y", "datetime":{"start":"2026-07-08T00:00:00Z","end":"2026-09-07T23:59:59Z"}}
        }
    })).unwrap();
    let started = Instant::now();
    let snapshot = match service.catalog_search_and_freeze(&request) {
        Ok(value) => value,
        Err(error) => {
            let (reason, status) = match &error {
                insar_application_service::ApplicationServiceError::Provider(provider_error) => (
                    provider_error.debug_context.get("reason"),
                    provider_error.debug_context.get("status"),
                ),
                _ => (None, None),
            };
            record(
                &root,
                json!({"phase":"catalogue", "ok":false,"code":error.code(),
                "reason":reason,"status":status,
                "elapsed_ms":started.elapsed().as_millis(),
                "session_available":true,"downloads_started":false}),
            );
            shutdown_and_record(&service, &root);
            panic!("native catalogue failed: {}", error.code());
        }
    };
    let workbench = snapshot.core.workbench.expect("frozen catalogue");
    assert_eq!(workbench.provider_id, "isro.nrsc.bhoonidhi");
    assert_eq!(workbench.batch_id, request.batch_id);
    assert!(workbench.candidate_count <= 1);
    assert!(
        snapshot.core.tasks.is_empty(),
        "catalogue query must not create a download"
    );
    record(
        &root,
        json!({"phase":"catalogue", "ok":true,"session_available":true,
        "elapsed_ms":started.elapsed().as_millis(),
        "candidate_count":workbench.candidate_count,"provider_id":workbench.provider_id,
        "batch_id":workbench.batch_id,"downloads_started":false,"restart_verified":false}),
    );
    shutdown_and_record(&service, &root);
    let reopened = state(root.join("tasks.sqlite3"), runtime).unwrap();
    let restored = reopened.core_restore(true).unwrap().core.workbench.unwrap();
    assert_eq!(restored.batch_id, workbench.batch_id);
    assert_eq!(restored.candidate_count, workbench.candidate_count);
    reopened.shutdown().unwrap();
    assert_eq!(
        workbench.candidate_count, 1,
        "request succeeded but a real S-band candidate is still needed"
    );
    record(
        &root,
        json!({"phase":"completed", "ok":true,"authentication_verified":true,"session_available":false,
        "candidate_count":workbench.candidate_count,"provider_id":workbench.provider_id,
        "batch_id":workbench.batch_id,"downloads_started":false,"restart_verified":true,
        "test_session_logout_verified":true}),
    );
}

/// Exercise the real UI query builder with the production capability descriptor.
/// Node receives public capability/query data only, never credential material.
fn ui_default_queries(service: &ApplicationServiceState) -> Vec<(String, Value)> {
    let source = service
        .provider_capabilities()
        .unwrap()
        .sources
        .into_iter()
        .find(|source| source.selector == "NISAR_SSAR")
        .expect("production S-band source capability");
    let module =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui/src/lib/sarSourceCapabilities.ts");
    let output = std::process::Command::new("node")
        .args(["--experimental-strip-types", "--input-type=module", "-e"])
        .arg(
            r#"
import { pathToFileURL } from 'node:url';
const { buildTypedSarCatalogQuery } = await import(pathToFileURL(process.argv[1]).href);
const source = JSON.parse(process.argv[2]);
const geometry = { type: 'Polygon', coordinates: [[[68,6],[98,6],[98,38],[68,38],[68,6]]] };
const cases = [
  ['dated', geometry, '2026-07-08', '2026-09-07', 1],
  ['undated', geometry, '', '', 1],
  ['overview-100', null, '', '', 100],
];
const queries = cases.map(([name, aoiGeojson, startUtc, endUtc, resultLimit]) => {
  const built = buildTypedSarCatalogQuery({ source, aoiGeojson, startUtc, endUtc,
    resultLimit, nisarProductType: 'RSLC', nisarBand: 'S' });
  if (!built.ok) throw new Error('S-band UI query construction failed');
  return [name, built.query];
});
process.stdout.write(JSON.stringify(queries));
"#,
        )
        .arg(module)
        .arg(serde_json::to_string(&source).unwrap())
        .output()
        .expect("Node is required for explicit cross-stack live acceptance");
    assert!(
        output.status.success(),
        "UI query construction must succeed"
    );
    assert!(output.stdout.len() < 64 * 1024);
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
#[ignore = "requires explicit owner-authorized UI-query live acceptance, Node and a fresh isolated Store; no downloads"]
fn saved_login_and_ui_default_queries_use_native_authority() {
    assert_eq!(
        std::env::var("INSAR_SBAND_UI_QUERY_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var_os("INSAR_SBAND_ACCEPTANCE_ROOT").expect("fresh root"));
    let runtime =
        PathBuf::from(std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("product runtime"));
    assert!(root.is_absolute() && !root.exists());
    fs::create_dir_all(&root).unwrap();
    assert_eq!(
        insar_download::desktop_bhoonidhi_readiness(),
        insar_download::BhoonidhiCredentialReadiness::Ready
    );
    let service = state(root.join("tasks.sqlite3"), runtime.clone()).unwrap();
    let _cleanup = SessionCleanup(&service);
    let queries = ui_default_queries(&service);
    assert_eq!(queries.len(), 3);
    let started = Instant::now();
    let authentication = service.bhoonidhi_authentication_check();
    record(
        &root,
        json!({"phase":"authentication", "ok":authentication.is_ok(),
        "code":authentication.as_ref().err().map(|error| error.code()),
        "elapsed_ms":started.elapsed().as_millis(), "downloads_started":false}),
    );
    if authentication.is_err() {
        shutdown_and_record(&service, &root);
        panic!("explicit S-band authentication failed; see safe event record");
    }
    let mut passed = 0;
    let mut last_frozen = None;
    for (name, query) in queries {
        let request: ApplicationCatalogSearchAndFreezeRequest = serde_json::from_value(json!({
            "request_id":format!("sband-ui-{name}"), "batch_id":format!("sband-ui-batch-{name}"),
            "created_at":"2026-09-30T00:00:00Z", "aoi_name":"S-band UI query acceptance",
            "aoi_geojson":query["aoi_geojson"], "workbench_page_size":100, "search":query,
        }))
        .unwrap();
        assert_eq!(request.search.provider_id, "isro.nrsc.bhoonidhi");
        assert_eq!(request.search.native_collection_id, "NISAR_SSAR_RSLC");
        assert_eq!(
            request.search.result_limit,
            Some(if name == "overview-100" { 100 } else { 1 })
        );
        let started = Instant::now();
        match service.catalog_search_and_freeze(&request) {
            Ok(snapshot) => {
                let workbench = snapshot.core.workbench.expect("frozen real catalogue");
                assert_eq!(workbench.provider_id, request.search.provider_id);
                assert_eq!(workbench.batch_id, request.batch_id);
                assert!(
                    snapshot.core.tasks.is_empty(),
                    "catalogue checks must not create downloads"
                );
                let complete = workbench.candidate_count
                    == usize::try_from(request.search.result_limit.unwrap()).unwrap();
                record(
                    &root,
                    json!({"phase":"ui_catalogue", "case":name, "ok":complete,
                    "elapsed_ms":started.elapsed().as_millis(), "candidate_count":workbench.candidate_count,
                    "batch_id":workbench.batch_id, "query":request.search, "downloads_started":false}),
                );
                passed += usize::from(complete);
                last_frozen = Some((workbench.batch_id, workbench.candidate_count));
            }
            Err(error) => {
                let status = match &error {
                    insar_application_service::ApplicationServiceError::Provider(source) => {
                        source.debug_context.get("status").and_then(Value::as_u64)
                    }
                    _ => None,
                };
                record(
                    &root,
                    json!({"phase":"ui_catalogue", "case":name, "ok":false,
                    "elapsed_ms":started.elapsed().as_millis(), "code":error.code(),
                    "http_status":status, "query":request.search, "downloads_started":false}),
                );
                // No automatic authentication retry or continued probing after a
                // transport, authorization or rate-limit failure. HTTP 5xx may be
                // compared with the remaining, distinct bounded query cases.
                if !matches!(status, Some(500..=599)) {
                    break;
                }
            }
        }
    }
    shutdown_and_record(&service, &root);
    let reopened = state(root.join("tasks.sqlite3"), runtime).unwrap();
    let restored = reopened.core_restore(true).unwrap().core;
    assert!(restored.tasks.is_empty());
    assert_eq!(
        restored
            .workbench
            .map(|batch| (batch.batch_id, batch.candidate_count)),
        last_frozen
    );
    reopened.shutdown().unwrap();
    record(
        &root,
        json!({"phase":"completed", "ok":passed == 3, "passed_cases":passed,
        "expected_cases":3, "restart_verified":true, "test_session_logout_verified":true,
        "downloads_started":false}),
    );
    assert_eq!(
        passed, 3,
        "UI-generated queries must all pass; see per-case safe evidence"
    );
}
