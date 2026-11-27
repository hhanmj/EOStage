use insar_application_service::{
    ApplicationNetworkSettings, ApplicationService, ApplicationServiceConfig,
    GacosResultImportRequest, OrbitDirectoryMatchRequest, OrbitSceneInput, SceneImportRequest,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("insar-p04a-bc-{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).expect("create isolated native boundary root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn service(root: &TempRoot) -> ApplicationService {
    ApplicationService::new(ApplicationServiceConfig::business_store(
        root.path().join("tasks.sqlite3"),
    ))
}

#[test]
fn product_owned_settings_round_trip_without_store_schema_mutation() {
    let root = TempRoot::new("settings");
    let service = service(&root);
    let saved = service
        .save_network_settings(&ApplicationNetworkSettings {
            proxy_enabled: true,
            proxy_url: "http://127.0.0.1:8080".to_owned(),
            cache_enabled: true,
            cache_dir: root.path().join("cache").to_string_lossy().to_string(),
            cache_limit_mb: 512,
            tianditu_token: "user-configured-tianditu-token".to_owned(),
            asf_ssl_verify: true,
        })
        .expect("network settings save");
    assert_eq!(saved.tianditu_token, "user-configured-tianditu-token");
    let loaded = service
        .get_network_settings()
        .expect("network settings load");
    assert_eq!(loaded.proxy_url, "http://127.0.0.1:8080");
    assert_eq!(
        loaded.proxy_mode(),
        insar_application_service::NetworkProxyMode::Manual
    );
    let mut direct = loaded.clone();
    direct.proxy_enabled = false;
    assert_eq!(
        direct.proxy_mode(),
        insar_application_service::NetworkProxyMode::Direct
    );
    let mut automatic = loaded.clone();
    automatic.proxy_url.clear();
    assert_eq!(
        automatic.proxy_mode(),
        insar_application_service::NetworkProxyMode::Auto
    );
    assert_eq!(loaded.tianditu_token, "user-configured-tianditu-token");
    for unsafe_proxy in [
        "http://user:fixture-secret@127.0.0.1:8080",
        "http://127.0.0.1:8080/private",
        "http://127.0.0.1:8080/?token=fixture-secret",
        "http://127.0.0.1:8080/#fragment",
    ] {
        let mut unsafe_settings = loaded.clone();
        unsafe_settings.proxy_url = unsafe_proxy.to_owned();
        assert!(
            service.save_network_settings(&unsafe_settings).is_err(),
            "unsafe proxy URL accepted"
        );
        unsafe_settings.proxy_enabled = false;
        assert!(
            service.save_network_settings(&unsafe_settings).is_err(),
            "disabling proxy must not persist an unsafe URL"
        );
    }
    assert!(!root.path().join("tasks.sqlite3").exists());
}

#[test]
fn bounded_manual_scene_import_is_native_and_does_not_open_store() {
    let root = TempRoot::new("scenes");
    let service = service(&root);
    let result = service
        .import_scenes(&SceneImportRequest {
            source: "text".to_owned(),
            text: Some(
                "S1A_IW_SLC__1SDV_20240312T223805_20240312T223832_052914_0667A5_1234.SAFE\nS1A_IW_SLC__1SDV_20240312T223805_20240312T223832_052914_0667A5_1234.SAFE"
                    .to_owned(),
            ),
            path: None,
        })
        .expect("native scene import");
    assert_eq!(result["ok"], true);
    assert_eq!(result["scenes"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["duplicates"].as_array().map(Vec::len), Some(1));
    assert!(!root.path().join("tasks.sqlite3").exists());
}

#[test]
fn local_orbit_directory_matching_uses_core_parser_and_bounded_scene_input() {
    let root = TempRoot::new("orbit");
    let orbit_dir = root.path().join("orbits");
    let nested_orbit_dir = orbit_dir.join("nested");
    fs::create_dir_all(&nested_orbit_dir).expect("nested orbit directory");
    fs::write(
        nested_orbit_dir
            .join("S1A_OPER_AUX_POEORB_OPOD_20240313T120000_V20240312T000000_20240313T000000.EOF"),
        b"fixture",
    )
    .expect("orbit filename fixture");
    let result = service(&root)
        .match_orbits_directory(&OrbitDirectoryMatchRequest {
            orbit_dir,
            scenes: vec![OrbitSceneInput {
                scene_id: "S1A_IW_SLC__1SDV_20240312T223805_20240312T223832_052914_0667A5_1234"
                    .to_owned(),
                platform: insar_core::OrbitPlatform::S1A,
                acquisition_unix_seconds: Some(1_710_258_000),
            }],
        })
        .expect("native orbit directory match");
    assert_eq!(result["ok"], true);
    assert_eq!(result["orbit_files"], 1);
    assert_eq!(result["report"]["total"], 1);
}

#[test]
fn gacos_legacy_pair_import_cannot_bypass_frozen_request_and_store() {
    let root = TempRoot::new("gacos");
    let ztd = root.path().join("20240312.ztd");
    let rsc = root.path().join("20240312.ztd.rsc");
    fs::write(&ztd, [0_u8; 4]).expect("ztd fixture");
    fs::write(
        &rsc,
        b"WIDTH 1
FILE_LENGTH 1
",
    )
    .expect("rsc fixture");
    let error = service(&root)
        .gacos_import_result(&GacosResultImportRequest {
            ztd_path: ztd.clone(),
            rsc_path: rsc,
            output_dir: Some(root.path().join("gacos-output")),
        })
        .expect_err("unbound legacy import must not write files or task state");
    assert_eq!(error.code(), "GACOS_IMPORT_CONTEXT_REQUIRED");
    assert_eq!(fs::read(ztd).unwrap(), [0_u8; 4]);
    assert!(!root.path().join("gacos-output").exists());
    assert_eq!(
        service(&root).gacos_ledger().unwrap()["records"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
}
