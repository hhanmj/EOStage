use std::sync::Mutex;

use insar_core::{ContractError, ProviderResult};
use insar_provider::asf::download::{
    resolve_asf_download, resolve_frozen_asf_asset, DownloadMetadataTransport,
};
use serde_json::{json, Map, Value};

struct ScriptedTransport {
    response: Mutex<Option<ProviderResult<Value>>>,
    calls: Mutex<Vec<Vec<(String, String)>>>,
}

#[test]
fn frozen_generic_asf_assets_require_trusted_https_and_real_filename() {
    let resolved = resolve_frozen_asf_asset(
        "ALPSRP123",
        "https://datapool.asf.alaska.edu/ALOS/native-product.tar.gz",
        "native-product.tar.gz",
        Some(4096),
    )
    .unwrap();

    assert_eq!(resolved.file_name, "native-product.tar.gz");
    assert_eq!(resolved.expected_size, Some(4096));
    assert!(resolve_frozen_asf_asset(
        "ALPSRP123",
        "https://untrusted.example/native-product.tar.gz",
        "native-product.tar.gz",
        None,
    )
    .is_err());
    assert!(resolve_frozen_asf_asset(
        "ALPSRP123",
        "http://datapool.asf.alaska.edu/ALOS/native-product.tar.gz",
        "native-product.tar.gz",
        None,
    )
    .is_err());
}

impl ScriptedTransport {
    fn success(response: Value) -> Self {
        Self {
            response: Mutex::new(Some(Ok(response))),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl DownloadMetadataTransport for ScriptedTransport {
    fn cmr_json(
        &self,
        params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.calls.lock().unwrap().push(params.to_vec());
        self.response.lock().unwrap().take().unwrap()
    }
}

fn scene_id() -> &'static str {
    "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_1234"
}

fn cmr_entry(url: &str) -> Value {
    json!({
        "feed": {"entry": [{
            "producer_granule_id": format!("{}-SLC", scene_id()),
            "granule_size": "1024.5",
            "links": [{
                "href": url,
                "rel": "http://esipfed.org/ns/fedsearch/1.1/data#",
                "title": "Direct Download"
            }]
        }]}
    })
}

#[test]
fn resolves_exact_scene_from_live_cmr_without_exposing_url_in_debug() {
    let transport = ScriptedTransport::success(cmr_entry(
        "https://datapool.asf.alaska.edu/SLC/SA/live-product.zip?Signature=SECRET",
    ));

    let resolved =
        resolve_asf_download(&transport, scene_id(), "frozen-product.zip", "SLC", &|| {
            false
        })
        .unwrap();

    assert_eq!(resolved.item_id, scene_id());
    assert_eq!(resolved.file_name, "frozen-product.zip");
    assert_eq!(resolved.expected_size, Some(1024 * 1024 * 1024));
    assert!(resolved
        .url()
        .starts_with("https://datapool.asf.alaska.edu/"));
    let debug = format!("{resolved:?}");
    assert!(!debug.contains("Signature"));
    assert!(!debug.contains("SECRET"));

    let calls = transport.calls.lock().unwrap();
    let params = &calls[0];
    assert!(params
        .iter()
        .any(|(key, value)| key == "granule_ur[]" && value == scene_id()));
    assert!(params
        .iter()
        .any(|(key, value)| key == "granule_ur[]" && value == &format!("{}-SLC", scene_id())));
    assert!(params.iter().all(|(_, value)| !value.contains("Signature")));
}

#[test]
fn rejects_wrong_scene_missing_url_unsafe_filename_and_cancellation() {
    let wrong = ScriptedTransport::success(json!({
        "feed": {"entry": [{"producer_granule_id": "S1A_WRONG", "links": []}]}
    }));
    let error = resolve_asf_download(&wrong, scene_id(), "x.zip", "SLC", &|| false).unwrap_err();
    assert_eq!(error.code, "DL005");

    let unsafe_name = ScriptedTransport::success(cmr_entry(
        "https://datapool.asf.alaska.edu/SLC/SA/live-product.zip",
    ));
    let error = resolve_asf_download(
        &unsafe_name,
        scene_id(),
        "nested/product.zip",
        "SLC",
        &|| false,
    )
    .unwrap_err();
    assert_eq!(error.code, "DL003");

    let cancelled = ScriptedTransport {
        response: Mutex::new(Some(Err(ContractError::new(
            "DL001",
            "cancelled",
            Map::new(),
        )
        .unwrap()))),
        calls: Mutex::new(Vec::new()),
    };
    let error = resolve_asf_download(&cancelled, scene_id(), "x.zip", "SLC", &|| true).unwrap_err();
    assert_eq!(error.code, "DL001");
}
