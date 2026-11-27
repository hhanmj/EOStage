use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use tauri::ipc::{Origin, RuntimeAuthority};
use tauri::utils::acl::capability::{Capability, CapabilityFile};
use tauri::utils::acl::manifest::Manifest;
use tauri::utils::acl::resolved::Resolved;
use tauri::utils::platform::Target;

const NISAR_PREVIEW_ORIGIN: &str = "https://nisar.asf.earthdatacloud.nasa.gov";
// Reviewed Google Earth Engine `/v1/projects/.../maps` XYZ raster overlay tile
// origin; scoped strictly to `img-src` (all token exchange and compute DAG
// calls execute in the Rust host, never via WebView `connect-src`).
const EARTHENGINE_TILE_ORIGIN: &str = "https://earthengine.googleapis.com";
const PLANETARY_COMPUTER_PREVIEW_ORIGIN: &str = "https://planetarycomputer.microsoft.com";
const WAYBACK_TILE_ORIGIN: &str = "https://wayback-a.maptiles.arcgis.com";
// Result-row Browse/Thumbnail hosts emitted by the provider mappers (including
// PR #32 ECOSTRESS/EMIT LPDAAC public browse PNGs and Earth Search S2 COG
// thumbnails); the insar-provider browse_image_policy test keeps this aligned.
const RESULT_BROWSE_ORIGINS: [&str; 5] = [
    "https://datapool.asf.alaska.edu",
    "https://cumulus.asf.earthdatacloud.nasa.gov",
    "https://data.lpdaac.earthdatacloud.nasa.gov",
    "https://sentinel-cogs.s3.us-west-2.amazonaws.com",
    "https://bhoonidhi.nrsc.gov.in",
];

fn read_json(relative_path: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&source)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn read_capability(relative_path: &str) -> Capability {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    match CapabilityFile::load(&path)
        .unwrap_or_else(|error| panic!("failed to load {}: {error}", path.display()))
    {
        CapabilityFile::Capability(capability) => capability,
        CapabilityFile::List(_) | CapabilityFile::NamedList { .. } => {
            panic!("expected one capability in {}", path.display())
        }
    }
}

fn development_runtime_authority() -> RuntimeAuthority {
    let capability = read_capability("capabilities/development.json");
    let manifests: BTreeMap<String, Manifest> = serde_json::from_str(include_str!(concat!(
        env!("OUT_DIR"),
        "/acl-manifests.json"
    )))
    .expect("tauri build output must contain valid ACL manifests");
    let resolved = Resolved::resolve(
        &manifests,
        BTreeMap::from([(capability.identifier.clone(), capability)]),
        Target::Windows,
    )
    .expect("development capability must resolve against the locked Tauri manifests");
    RuntimeAuthority::new(manifests, resolved)
}

fn strings(value: &Value) -> BTreeSet<String> {
    match value {
        Value::String(source) => BTreeSet::from([source.clone()]),
        Value::Array(sources) => sources
            .iter()
            .map(|source| {
                source
                    .as_str()
                    .expect("security policy sources must be strings")
                    .to_owned()
            })
            .collect(),
        _ => panic!("security policy directive must be a string or string array"),
    }
}

fn expected_strings(sources: &[&str]) -> BTreeSet<String> {
    sources.iter().map(|source| (*source).to_owned()).collect()
}

fn csp(config: &Value, key: &str) -> BTreeMap<String, BTreeSet<String>> {
    config["app"]["security"][key]
        .as_object()
        .unwrap_or_else(|| panic!("missing app.security.{key}"))
        .iter()
        .map(|(directive, sources)| (directive.clone(), strings(sources)))
        .collect()
}

fn map_provider_contract() -> Value {
    read_json("../contracts/map-providers.v1.json")
}

fn enabled_map_external_origins() -> BTreeSet<String> {
    let contract = map_provider_contract();
    ["providers", "annotation_overlays"]
        .into_iter()
        .flat_map(|field| {
            contract[field]
                .as_array()
                .unwrap_or_else(|| panic!("map provider contract has {field}"))
                .iter()
        })
        .filter(|provider| {
            provider["enabled_in_preview"] == true && provider["production_available"] == true
        })
        .flat_map(|provider| strings(&provider["exact_image_origins"]))
        .filter(|origin| origin.starts_with("https://"))
        .collect()
}

fn assert_map_contract_and_csp_alignment(sources: &BTreeMap<String, BTreeSet<String>>) {
    let contract = map_provider_contract();
    let providers = contract["providers"]
        .as_array()
        .expect("map provider contract has providers");
    let enabled_keys: BTreeSet<String> = providers
        .iter()
        .filter(|provider| {
            provider["enabled_in_preview"] == true && provider["production_available"] == true
        })
        .map(|provider| {
            provider["key"]
                .as_str()
                .expect("map provider key is a string")
                .to_owned()
        })
        .collect();
    assert_eq!(
        enabled_keys,
        expected_strings(&[
            "osm",
            "cartoLight",
            "cartoDark",
            "arcgisSatellite",
            "arcgisTopo",
            "arcgisStreet",
            "openTopoMap",
            "googleSatellite",
            "googleMap",
            "googleHybrid",
            "gaodeMap",
            "gaodeSatellite",
            "tiandituSatellite",
            "tiandituVector",
        ])
    );
    assert_eq!(contract["default_provider_key"], "arcgisSatellite");
    assert!(providers
        .iter()
        .all(|provider| provider["key"] != "localFallback"));

    let mut enabled_external_origins = BTreeSet::new();
    for provider in providers {
        for field in [
            "key",
            "display_name",
            "tile_url_template",
            "subdomains",
            "exact_image_origins",
            "requires_token",
            "enabled_in_preview",
            "production_available",
            "tile_route",
            "readiness",
            "crs",
            "csp_origins",
            "contains_labels",
            "attribution",
        ] {
            assert!(
                provider.get(field).is_some(),
                "provider missing field: {field}"
            );
        }
        let template = provider["tile_url_template"]
            .as_str()
            .expect("provider tile URL template is a string");
        let subdomains = strings(&provider["subdomains"]);
        if template.contains("{s}") {
            assert!(
                !subdomains.is_empty(),
                "provider with {{s}} must own subdomains"
            );
        } else {
            assert!(
                subdomains.is_empty(),
                "provider without {{s}} must not invent subdomains"
            );
        }
        let origins = strings(&provider["exact_image_origins"]);
        assert!(origins.iter().all(|origin| !origin.contains('*')));
        enabled_external_origins.extend(
            origins
                .iter()
                .filter(|origin| origin.starts_with("https://"))
                .cloned(),
        );
    }
    let overlays = contract["annotation_overlays"]
        .as_array()
        .expect("map provider contract has annotation_overlays");
    assert_eq!(
        overlays
            .iter()
            .map(|overlay| overlay["key"].as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>(),
        expected_strings(&["tiandituImageLabels", "tiandituVectorLabels"])
    );
    for overlay in overlays {
        for field in [
            "key",
            "display_name",
            "tile_url_template",
            "subdomains",
            "exact_image_origins",
            "requires_token",
            "enabled_in_preview",
            "production_available",
            "layer",
            "source_key",
            "csp_origins",
            "z_index_role",
            "attribution",
        ] {
            assert!(
                overlay.get(field).is_some(),
                "annotation missing field: {field}"
            );
        }
        let origins = strings(&overlay["exact_image_origins"]);
        assert!(origins.iter().all(|origin| !origin.contains('*')));
        enabled_external_origins.extend(origins);
    }
    assert_eq!(enabled_external_origins, enabled_map_external_origins());
    assert!(sources["img-src"].is_superset(&enabled_external_origins));

    for directive_sources in sources.values() {
        assert!(directive_sources.iter().all(|source| source != "https://*"));
        assert!(directive_sources.iter().all(|source| !source.contains("*")));
    }
    let map_origins = enabled_map_external_origins();
    for (directive, directive_sources) in sources {
        if directive != "img-src" {
            assert!(!directive_sources.contains(NISAR_PREVIEW_ORIGIN));
            assert!(!directive_sources.contains(EARTHENGINE_TILE_ORIGIN));
            assert!(!directive_sources.contains(PLANETARY_COMPUTER_PREVIEW_ORIGIN));
            assert!(RESULT_BROWSE_ORIGINS
                .iter()
                .all(|origin| !directive_sources.contains(*origin)));
            assert!(directive_sources.is_disjoint(&map_origins));
        }
    }
}

fn assert_reviewed_image_policy(
    sources: &BTreeMap<String, BTreeSet<String>>,
    expected_connect_sources: &[&str],
) {
    // `EARTHENGINE_TILE_ORIGIN` is reviewed strictly for GEE `/maps` raster XYZ
    // overlay tiles rendered inside Leaflet (`img-src` only).
    let mut expected_image_sources = expected_strings(&[
        "'self'",
        "blob:",
        "data:",
        NISAR_PREVIEW_ORIGIN,
        EARTHENGINE_TILE_ORIGIN,
        PLANETARY_COMPUTER_PREVIEW_ORIGIN,
        WAYBACK_TILE_ORIGIN,
    ]);
    expected_image_sources.extend(expected_strings(&RESULT_BROWSE_ORIGINS));
    expected_image_sources.extend(enabled_map_external_origins());
    assert_eq!(sources.get("img-src"), Some(&expected_image_sources));
    assert_eq!(
        sources.get("connect-src"),
        Some(&expected_strings(expected_connect_sources))
    );
    assert_map_contract_and_csp_alignment(sources);
}

#[test]
fn profiles_allow_only_reviewed_preview_and_basemap_image_origins() {
    let production = csp(&read_json("tauri.conf.json"), "csp");
    let development = csp(&read_json("tauri.dev.conf.json"), "devCsp");

    assert_reviewed_image_policy(&production, &["'self'", "ipc:", "http://ipc.localhost"]);
    assert_reviewed_image_policy(
        &development,
        &[
            "'self'",
            "ipc:",
            "http://ipc.localhost",
            "http://127.0.0.1:5173",
            "ws://127.0.0.1:5173",
        ],
    );
}

#[test]
fn ordinary_development_and_capability_boundaries_remain_closed() {
    let base = read_json("tauri.conf.json");
    let development = read_json("tauri.dev.conf.json");

    assert_eq!(
        strings(&base["app"]["security"]["capabilities"]),
        expected_strings(&["production"])
    );
    assert_eq!(
        strings(&development["app"]["security"]["capabilities"]),
        expected_strings(&["development"])
    );
    let ordinary_sources = csp(&development, "devCsp");
    assert_eq!(
        ordinary_sources["connect-src"],
        expected_strings(&[
            "'self'",
            "ipc:",
            "http://ipc.localhost",
            "http://127.0.0.1:5173",
            "ws://127.0.0.1:5173",
        ])
    );

    let production_capability = read_json("capabilities/production.json");
    let development_capability = read_json("capabilities/development.json");
    let expected_permissions = expected_strings(&[
        "core:window:allow-close",
        "core:window:allow-inner-size",
        "core:window:allow-internal-toggle-maximize",
        "core:window:allow-is-maximized",
        "core:window:allow-maximize",
        "core:window:allow-minimize",
        "core:window:allow-start-dragging",
        "core:window:allow-start-resize-dragging",
        "core:window:allow-toggle-maximize",
        "core:window:allow-unmaximize",
        "dialog:allow-open",
    ]);

    assert_eq!(production_capability["identifier"], "production");
    assert_eq!(production_capability["local"], true);
    assert_eq!(
        strings(&production_capability["windows"]),
        expected_strings(&["main"])
    );
    assert!(production_capability.get("remote").is_none());
    assert_eq!(
        strings(&production_capability["permissions"]),
        expected_permissions
    );

    assert_eq!(development_capability["identifier"], "development");
    assert_eq!(development_capability["local"], true);
    assert_eq!(
        strings(&development_capability["windows"]),
        expected_strings(&["main"])
    );
    assert!(development_capability.get("remote").is_none());
    assert_eq!(
        strings(&development_capability["permissions"]),
        expected_permissions
    );
}

#[test]
fn development_acl_allows_local_dev_page_commands_and_rejects_remote_origins() {
    let authority = development_runtime_authority();

    // Tauri 2.11 classifies the configured devUrl and URLs relative to it as
    // Origin::Local before resolving IPC access. The development capability
    // must therefore authorize the local context, not grant the HTTP origin.
    for command in ["plugin:window|close", "plugin:dialog|open"] {
        assert!(
            authority
                .resolve_access(command, "main", "main", &Origin::Local)
                .is_some(),
            "{command} must resolve for the configured local development page"
        );

        for url in [
            "http://127.0.0.1:5173/tauri.html",
            "https://untrusted.invalid/",
        ] {
            assert!(
                authority
                    .resolve_access(
                        command,
                        "main",
                        "main",
                        &Origin::Remote {
                            url: url.parse().expect("test URL must parse"),
                        },
                    )
                    .is_none(),
                "{command} must reject remote origin {url}"
            );
        }
    }
}
