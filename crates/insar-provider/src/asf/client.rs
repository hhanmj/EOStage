use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(feature = "s1-live-acceptance")]
use std::fs::{File, OpenOptions};
#[cfg(feature = "s1-live-acceptance")]
use std::io::Write;
#[cfg(feature = "s1-live-acceptance")]
use std::path::PathBuf;
#[cfg(feature = "s1-live-acceptance")]
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock, Weak,
};
#[cfg(feature = "s1-live-acceptance")]
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{ContractError, ProviderNetworkPolicy, ProviderResult, ProxyPolicy};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::{Method, Url};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::search::CmrUmmPage;

const ASF_SEARCH_URL: &str = "https://api.daac.asf.alaska.edu/services/search/param";
const CMR_GRANULES_URL: &str = "https://cmr.earthdata.nasa.gov/search/granules.json";
const CMR_UMM_GRANULES_URL: &str = "https://cmr.earthdata.nasa.gov/search/granules.umm_json";
const USER_AGENT: &str = "EOStage/3 ASF metadata search";
const ASF_SEARCH_REQUEST_ATTEMPTS: u32 = 2;
const CMR_REQUEST_ROUNDS: u32 = 3;
const METADATA_RETRY_STEP: Duration = Duration::from_millis(150);
const MAX_RETRY_AFTER_SECONDS: u64 = 5;
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;
const MAX_DIRECT_GET_URL_BYTES: usize = 2_048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEndpoints {
    asf_search: Url,
    cmr_granules: Url,
    cmr_umm_granules: Url,
}

impl RemoteEndpoints {
    pub fn production() -> Self {
        Self {
            asf_search: Url::parse(ASF_SEARCH_URL).expect("official ASF URL is valid"),
            cmr_granules: Url::parse(CMR_GRANULES_URL).expect("official CMR URL is valid"),
            cmr_umm_granules: Url::parse(CMR_UMM_GRANULES_URL)
                .expect("official CMR UMM URL is valid"),
        }
    }

    #[doc(hidden)]
    pub fn loopback_for_tests(asf_search: &str, cmr_granules: &str) -> Result<Self, String> {
        let asf_search = parse_loopback_url(asf_search)?;
        let cmr_granules = parse_loopback_url(cmr_granules)?;
        Ok(Self {
            asf_search,
            cmr_umm_granules: cmr_granules.clone(),
            cmr_granules,
        })
    }

    pub fn asf_search_url(&self) -> &str {
        self.asf_search.as_str()
    }

    pub fn cmr_granules_url(&self) -> &str {
        self.cmr_granules.as_str()
    }

    pub fn cmr_umm_granules_url(&self) -> &str {
        self.cmr_umm_granules.as_str()
    }
}

fn parse_loopback_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "test endpoint must be a valid URL".to_string())?;
    let is_loopback = url
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| address.is_loopback());
    if url.scheme() != "http"
        || !is_loopback
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("test endpoint must be an unauthenticated HTTP loopback URL".to_string());
    }
    Ok(url)
}

#[derive(Clone, Copy, Debug)]
pub struct HttpLimits {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_json_bytes: usize,
    pub max_count_bytes: usize,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(17),
            max_json_bytes: 64 * 1024 * 1024,
            max_count_bytes: 1024 * 1024,
        }
    }
}

#[derive(Clone)]
pub struct AsfHttpClient {
    client: Client,
    endpoints: RemoteEndpoints,
    limits: HttpLimits,
    /// Bearer token for the restricted legacy SAR missions' CMR collections.
    /// Injected by the service layer; `None` leaves those searches
    /// unauthenticated, which CMR answers with zero granules.
    legacy_cmr_token: Option<Arc<dyn Fn() -> Option<String> + Send + Sync>>,
    #[cfg(feature = "s1-live-acceptance")]
    live_trace: Option<Arc<LiveAcceptanceTrace>>,
}

#[cfg(feature = "s1-live-acceptance")]
const S1_LIVE_TRACE_PATH_ENV: &str = "INSAR_S1_LIVE_TRACE_PATH";

#[cfg(feature = "s1-live-acceptance")]
struct LiveAcceptanceTrace {
    file: Mutex<File>,
}

#[cfg(feature = "s1-live-acceptance")]
static NEXT_LIVE_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "s1-live-acceptance")]
static LIVE_TRACE_REGISTRY: OnceLock<Mutex<BTreeMap<PathBuf, Weak<LiveAcceptanceTrace>>>> =
    OnceLock::new();

#[cfg(feature = "s1-live-acceptance")]
impl LiveAcceptanceTrace {
    fn from_environment() -> ProviderResult<Option<Arc<Self>>> {
        let Some(raw_path) = std::env::var_os(S1_LIVE_TRACE_PATH_ENV) else {
            return Ok(None);
        };
        let path = PathBuf::from(raw_path);
        if !path.is_absolute() || path.parent().is_none() {
            return Err(http_error(
                "V3_PROVIDER_CONFIGURATION_INVALID",
                "S1 实机验收跟踪路径无效",
                "live_trace_path",
                None,
            ));
        }
        Self::open(&path).map(Some)
    }

    fn open(path: &std::path::Path) -> ProviderResult<Arc<Self>> {
        let registry = LIVE_TRACE_REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
        let mut registry = registry.lock().map_err(|_| {
            http_error(
                "V3_PROVIDER_CONFIGURATION_INVALID",
                "无法锁定 S1 实机验收跟踪文件",
                "live_trace_lock",
                None,
            )
        })?;
        if let Some(existing) = registry.get(path).and_then(Weak::upgrade) {
            return Ok(existing);
        }
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|_| {
                http_error(
                    "V3_PROVIDER_CONFIGURATION_INVALID",
                    "无法打开 S1 实机验收跟踪文件",
                    "live_trace_create",
                    None,
                )
            })?;
        let trace = Arc::new(Self {
            file: Mutex::new(file),
        });
        registry.insert(path.to_path_buf(), Arc::downgrade(&trace));
        Ok(trace)
    }

    fn request_started(
        &self,
        operation: &str,
        round: u32,
        method: &str,
        endpoint: &str,
        params: &BTreeMap<String, String>,
    ) -> u64 {
        let request_id = NEXT_LIVE_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let intersects_with = params.get("intersectsWith");
        let intersects_with_sha256 = intersects_with.map(|value| {
            let mut digest = Sha256::new();
            digest.update(value.as_bytes());
            format!("{:x}", digest.finalize())
        });
        let event = serde_json::json!({
            "event": "asf_request_started",
            "at_unix_ms": unix_time_millis(),
            "process_id": std::process::id(),
            "request_id": request_id,
            "operation": operation,
            "round": round,
            "method": method,
            "endpoint": endpoint,
            "parameter_names": params.keys().collect::<Vec<_>>(),
            "output": params.get("output"),
            "max_results": params.get("maxResults"),
            "processing_level": params.get("processingLevel"),
            "beam_mode": params.get("beamMode"),
            "polarization": params.get("polarization"),
            "flight_direction": params.get("flightDirection"),
            "bbox_parameter_present": params.contains_key("bbox"),
            "intersects_with_sha256": intersects_with_sha256,
            "intersects_with_bytes": intersects_with.map(|value| value.len()),
        });
        self.write_event(&event);
        request_id
    }

    fn request_finished<T>(&self, request_id: u64, result: &ProviderResult<T>) {
        let event = match result {
            Ok(_) => serde_json::json!({
                "event": "asf_request_finished",
                "at_unix_ms": unix_time_millis(),
                "process_id": std::process::id(),
                "request_id": request_id,
                "outcome": "success",
                "error_code": null,
                "http_status": null,
                "retryable": false,
            }),
            Err(error) => serde_json::json!({
                "event": "asf_request_finished",
                "at_unix_ms": unix_time_millis(),
                "process_id": std::process::id(),
                "request_id": request_id,
                "outcome": "error",
                "error_code": error.code,
                "http_status": error.debug_context.get("status"),
                "category": error.debug_context.get("category"),
                "retryable": retryable_metadata_error(error),
            }),
        };
        self.write_event(&event);
    }

    fn write_event(&self, event: &Value) {
        let Ok(mut file) = self.file.lock() else {
            return;
        };
        if let Ok(mut encoded) = serde_json::to_vec(event) {
            encoded.push(b'\n');
            let _ = file.write_all(&encoded);
            let _ = file.flush();
        }
    }
}

#[cfg(all(test, feature = "s1-live-acceptance"))]
mod live_trace_tests {
    use super::*;

    #[test]
    fn multiple_production_clients_can_share_one_live_trace_file() {
        let path = std::env::temp_dir().join(format!(
            "insar-s1-live-shared-trace-{}-{}.jsonl",
            std::process::id(),
            unix_time_millis()
        ));
        let first = LiveAcceptanceTrace::open(&path).expect("first trace handle");
        let second = LiveAcceptanceTrace::open(&path).expect("second trace handle");
        assert!(Arc::ptr_eq(&first, &second));
        let params = BTreeMap::from([("output".to_owned(), "geojson".to_owned())]);
        let first_id =
            first.request_started("metadata", 1, "POST", "https://example.test", &params);
        let second_id =
            second.request_started("metadata", 1, "POST", "https://example.test", &params);
        assert_ne!(first_id, second_id);
        drop(first);
        drop(second);

        let lines = std::fs::read_to_string(&path).expect("shared trace evidence");
        let events = lines
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("one JSON event per line"))
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .all(|event| event["event"] == "asf_request_started"));
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(feature = "s1-live-acceptance")]
fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

impl AsfHttpClient {
    pub fn production() -> ProviderResult<Self> {
        Self::production_with_network_policy(ProviderNetworkPolicy::system())
    }

    pub fn production_with_network_policy(
        network_policy: ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        Self::build(
            RemoteEndpoints::production(),
            HttpLimits::default(),
            network_policy.route(),
            false,
        )
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn production_for_nisar_exact_identity_canary() -> ProviderResult<Self> {
        let limits = HttpLimits {
            max_json_bytes: 2 * 1024 * 1024,
            ..HttpLimits::default()
        };
        Self::build(
            RemoteEndpoints::production(),
            limits,
            ProxyPolicy::System,
            false,
        )
    }

    #[doc(hidden)]
    pub fn with_test_endpoints(
        endpoints: RemoteEndpoints,
        limits: HttpLimits,
    ) -> ProviderResult<Self> {
        if !endpoints
            .asf_search
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback())
            || !endpoints
                .cmr_granules
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|address| address.is_loopback())
            || !endpoints
                .cmr_umm_granules
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|address| address.is_loopback())
        {
            return Err(http_error(
                "V3_PROVIDER_CONFIGURATION_INVALID",
                "测试 Provider 只允许使用本机回环地址",
                "invalid_test_endpoint",
                None,
            ));
        }
        Self::build(endpoints, limits, ProxyPolicy::Direct, true)
    }

    fn build(
        endpoints: RemoteEndpoints,
        limits: HttpLimits,
        proxy_policy: ProxyPolicy,
        test_endpoint: bool,
    ) -> ProviderResult<Self> {
        if limits.max_json_bytes == 0
            || limits.max_count_bytes == 0
            || limits.connect_timeout.is_zero()
            || limits.request_timeout.is_zero()
        {
            return Err(http_error(
                "V3_PROVIDER_CONFIGURATION_INVALID",
                "ASF Provider 网络限制无效",
                "invalid_http_limits",
                None,
            ));
        }
        let mut builder = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(limits.connect_timeout)
            .timeout(limits.request_timeout)
            // The official ASF multipart endpoint has been observed to leave
            // negotiated HTTP/2 uploads open indefinitely on the supported
            // Windows desktop path. ASF accepts HTTP/1.1; pinning that protocol
            // keeps the existing request deadline and cancellation contract
            // effective without changing query parameters or spatial authority.
            .http1_only()
            // Provider-owned retry policy below is the sole retry authority.
            .retry(reqwest::retry::never());
        if test_endpoint {
            // Loopback test servers are plain HTTP; skipping the macOS Keychain
            // system root-certificate scan avoids multi-second test startup stalls.
            builder = builder.tls_built_in_root_certs(false);
        }
        builder = crate::network_proxy::blocking_builder_with_proxy(builder, proxy_policy);
        let client = builder.build().map_err(|_| {
            http_error(
                "V3_PROVIDER_CONFIGURATION_INVALID",
                "无法初始化 ASF Provider 网络客户端",
                "client_build",
                None,
            )
        })?;
        #[cfg(feature = "s1-live-acceptance")]
        let live_trace = if proxy_policy == ProxyPolicy::Direct {
            None
        } else {
            LiveAcceptanceTrace::from_environment()?
        };
        Ok(Self {
            client,
            endpoints,
            limits,
            legacy_cmr_token: None,
            #[cfg(feature = "s1-live-acceptance")]
            live_trace,
        })
    }

    /// Inject the Earthdata bearer-token provider used for the restricted
    /// legacy SAR missions' CMR searches. The service layer owns credential
    /// resolution; this crate only carries the opaque token forward.
    pub fn with_legacy_cmr_token_provider(
        mut self,
        provider: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) -> Self {
        self.legacy_cmr_token = Some(provider);
        self
    }

    pub fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.check_cancelled(cancelled)?;
        let is_live_sentinel1 = self
            .endpoints
            .asf_search
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("api.daac.asf.alaska.edu"))
            && params
                .get("dataset")
                .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SENTINEL-1"));
        let effective_params = if is_live_sentinel1 {
            let mut cow = params.clone();
            if let Some(wkt) = cow.get("intersectsWith").filter(|wkt| wkt.len() > 1024) {
                if let Some((west, south, east, north)) = wkt_bounding_box(wkt) {
                    cow.insert(
                        "intersectsWith".to_string(),
                        format!(
                            "POLYGON(({west} {south},{east} {south},{east} {north},{west} {north},{west} {south}))"
                        ),
                    );
                }
            }
            cow
        } else {
            params.clone()
        };
        let mut last_error = None;
        for round in 0..ASF_SEARCH_REQUEST_ATTEMPTS {
            for method in asf_request_methods(&self.endpoints.asf_search, &effective_params) {
                self.check_cancelled(cancelled)?;
                let trace_id =
                    self.trace_request("asf_geojson", round + 1, &method, &effective_params);
                let mut request = if method == Method::POST {
                    self.client
                        .post(self.endpoints.asf_search.clone())
                        .form(&effective_params)
                } else {
                    self.client
                        .get(self.endpoints.asf_search.clone())
                        .query(&effective_params)
                };
                if is_live_sentinel1 {
                    request = request.timeout(Duration::from_secs(10));
                }
                let result = self.send_json(request, self.limits.max_json_bytes, cancelled);
                self.trace_result(trace_id, &result);
                match result {
                    Ok(value) if value.is_object() => return Ok(value),
                    Ok(_) => {
                        last_error = Some(http_error(
                            "ASF002",
                            "ASF 元数据检索失败：服务器返回内容不是 JSON 对象",
                            "invalid_json_shape",
                            None,
                        ));
                    }
                    Err(error) if error.code == "DL001" => return Err(error),
                    Err(error) => last_error = Some(error),
                }
            }
            if last_error
                .as_ref()
                .is_some_and(|error| !retryable_metadata_error(error))
            {
                return Err(last_error.take().expect("terminal ASF error is present"));
            }
            if is_live_sentinel1 {
                if let Ok(fallback) = self.asf_sentinel1_cmr_geojson_fallback(params, cancelled) {
                    return Ok(fallback);
                }
            }
            if round + 1 < ASF_SEARCH_REQUEST_ATTEMPTS {
                wait_metadata_retry(round + 1, last_error.as_ref(), cancelled)?;
            }
        }
        if is_live_sentinel1 {
            if let Ok(fallback) = self.asf_sentinel1_cmr_geojson_fallback(params, cancelled) {
                return Ok(fallback);
            }
        }
        Err(last_error.unwrap_or_else(|| {
            http_error(
                "ASF002",
                "ASF 元数据检索失败：网络请求未完成",
                "request_failed",
                None,
            )
        }))
    }

    fn asf_sentinel1_cmr_geojson_fallback(
        &self,
        params: &BTreeMap<String, String>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.check_cancelled(cancelled)?;
        let requested_max = params
            .get("maxResults")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(250);
        let page_size = requested_max.clamp(1, 260);
        let level_upper = params
            .get("processingLevel")
            .map(|level| level.to_ascii_uppercase())
            .unwrap_or_default();
        let include_slc = level_upper.is_empty() || level_upper.contains("SLC");
        let include_grd = level_upper.contains("GRD");
        let mut collections: Vec<&'static str> = Vec::new();
        if include_slc {
            if let Some(ids) = super::cmr::collection_ids("SLC") {
                collections.extend_from_slice(ids);
            }
        }
        if include_grd {
            if let Some(ids) = super::cmr::collection_ids("GRD") {
                collections.extend_from_slice(ids);
            }
        }
        if collections.is_empty() {
            if let Some(ids) = super::cmr::collection_ids("SLC") {
                collections.extend_from_slice(ids);
            }
        }
        let mut cmr_params = vec![
            ("page_size".to_string(), page_size.to_string()),
            ("sort_key".to_string(), "-start_date".to_string()),
        ];
        for collection_id in collections {
            cmr_params.push((
                "collection_concept_id".to_string(),
                collection_id.to_string(),
            ));
        }
        let start = params.get("start").map(String::as_str).unwrap_or("");
        let end = params.get("end").map(String::as_str).unwrap_or("");
        if !start.is_empty() || !end.is_empty() {
            cmr_params.push(("temporal".to_string(), format!("{start},{end}")));
        }
        if let Some(wkt) = params.get("intersectsWith") {
            if let Some((west, south, east, north)) = wkt_bounding_box(wkt) {
                cmr_params.push((
                    "bounding_box".to_string(),
                    format!("{west},{south},{east},{north}"),
                ));
            }
        }
        let request = self
            .client
            .get(self.endpoints.cmr_granules.clone())
            .query(&cmr_params)
            .timeout(Duration::from_secs(35));
        let value = self.send_json(request, self.limits.max_json_bytes, cancelled)?;
        let entries = value
            .get("feed")
            .and_then(|feed| feed.get("entry"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let features = entries
            .iter()
            .filter_map(Value::as_object)
            .filter_map(cmr_entry_to_asf_geojson_feature)
            .filter(|feature| cmr_feature_matches_asf_params(feature, params))
            .collect::<Vec<_>>();
        Ok(serde_json::json!({
            "type": "FeatureCollection",
            "features": features,
        }))
    }

    pub fn asf_count(
        &self,
        params: &BTreeMap<String, String>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        let mut params = params.clone();
        params.insert("output".to_string(), "count".to_string());
        params.remove("maxResults");
        let mut last_error = None;
        for round in 0..ASF_SEARCH_REQUEST_ATTEMPTS {
            for method in asf_request_methods(&self.endpoints.asf_search, &params) {
                self.check_cancelled(cancelled)?;
                let trace_id = self.trace_request("asf_count", round + 1, &method, &params);
                let request = if method == Method::POST {
                    self.client
                        .post(self.endpoints.asf_search.clone())
                        .form(&params)
                } else {
                    self.client
                        .get(self.endpoints.asf_search.clone())
                        .query(&params)
                };
                let result = self.send_bytes(request, self.limits.max_count_bytes, cancelled);
                self.trace_result(trace_id, &result);
                match result {
                    Ok(bytes) => {
                        if let Some(count) = parse_count(&bytes) {
                            return Ok(Some(count));
                        }
                    }
                    Err(error) if error.code == "DL001" => return Err(error),
                    Err(error) => last_error = Some(error),
                }
            }
            if last_error
                .as_ref()
                .is_some_and(|error| !retryable_metadata_error(error))
            {
                return Ok(None);
            }
            if round + 1 < ASF_SEARCH_REQUEST_ATTEMPTS {
                wait_metadata_retry(round + 1, last_error.as_ref(), cancelled)?;
            }
        }
        Ok(None)
    }

    #[cfg(feature = "s1-live-acceptance")]
    fn trace_request(
        &self,
        operation: &str,
        round: u32,
        method: &Method,
        params: &BTreeMap<String, String>,
    ) -> Option<u64> {
        self.live_trace.as_ref().map(|trace| {
            trace.request_started(
                operation,
                round,
                method.as_str(),
                self.endpoints.asf_search.as_str(),
                params,
            )
        })
    }

    #[cfg(not(feature = "s1-live-acceptance"))]
    fn trace_request(
        &self,
        _operation: &str,
        _round: u32,
        _method: &Method,
        _params: &BTreeMap<String, String>,
    ) -> Option<u64> {
        None
    }

    #[cfg(feature = "s1-live-acceptance")]
    fn trace_result<T>(&self, trace_id: Option<u64>, result: &ProviderResult<T>) {
        if let (Some(trace), Some(trace_id)) = (&self.live_trace, trace_id) {
            trace.request_finished(trace_id, result);
        }
    }

    #[cfg(not(feature = "s1-live-acceptance"))]
    fn trace_result<T>(&self, _trace_id: Option<u64>, _result: &ProviderResult<T>) {}

    pub fn cmr_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        let mut last_error = None;
        for round in 0..CMR_REQUEST_ROUNDS {
            self.check_cancelled(cancelled)?;
            let request = self
                .client
                .get(self.endpoints.cmr_granules.clone())
                .query(params);
            match self.send_json(request, self.limits.max_json_bytes, cancelled) {
                Ok(value) if value.is_object() => return Ok(value),
                Ok(_) => {
                    last_error = Some(http_error(
                        "ASF002",
                        "CMR 备用检索失败：服务器返回内容不是 JSON 对象",
                        "invalid_json_shape",
                        None,
                    ));
                }
                Err(error) if error.code == "DL001" => return Err(error),
                Err(error) => last_error = Some(error),
            }
            if round + 1 < CMR_REQUEST_ROUNDS {
                wait_metadata_retry(round + 1, last_error.as_ref(), cancelled)?;
            }
        }
        Err(last_error.unwrap_or_else(|| {
            http_error(
                "ASF002",
                "CMR 备用检索失败：网络请求未完成",
                "request_failed",
                None,
            )
        }))
    }

    /// One bounded CMR UMM request. Catalogue pagination owns retry and request
    /// budgets, so this seam deliberately performs exactly one HTTP request.
    pub(crate) fn cmr_catalog_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.check_cancelled(cancelled)?;
        let request = self
            .client
            .get(self.endpoints.cmr_umm_granules.clone())
            .query(params);
        self.send_json(request, self.limits.max_json_bytes, cancelled)
    }

    /// One authenticated CMR granule JSON page for the restricted legacy SAR
    /// missions (ERS-1/2, RADARSAT-1, JERS-1). Their collections hide all
    /// granules from anonymous queries, so the request fails closed when no
    /// Earthdata token can be resolved instead of silently returning zero.
    pub(crate) fn cmr_legacy_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.check_cancelled(cancelled)?;
        let Some(token_provider) = self.legacy_cmr_token.as_ref() else {
            write_legacy_cmr_diagnostic(&self.endpoints.cmr_granules, params, false, None);
            return Err(http_error(
                "ASF_LEGACY_LOGIN_REQUIRED",
                "该历史卫星数据集为受限馆藏，检索需要 Earthdata 登录凭据",
                "legacy_cmr_token_missing",
                None,
            ));
        };
        let Some(token) = token_provider().filter(|token| !token.trim().is_empty()) else {
            write_legacy_cmr_diagnostic(&self.endpoints.cmr_granules, params, false, None);
            return Err(http_error(
                "ASF_LEGACY_LOGIN_REQUIRED",
                "该历史卫星数据集为受限馆藏，检索需要 Earthdata 登录凭据",
                "legacy_cmr_token_unavailable",
                None,
            ));
        };
        let mut last_error = None;
        for round in 0..CMR_REQUEST_ROUNDS {
            self.check_cancelled(cancelled)?;
            let request = self
                .client
                .get(self.endpoints.cmr_granules.clone())
                .query(params)
                .bearer_auth(token.trim());
            match self.send_json(request, self.limits.max_json_bytes, cancelled) {
                Ok(value) if value.is_object() => {
                    write_legacy_cmr_diagnostic(&self.endpoints.cmr_granules, params, true, None);
                    return Ok(value);
                }
                Ok(_) => {
                    last_error = Some(http_error(
                        "ASF002",
                        "CMR 受限数据集检索失败：服务器返回内容不是 JSON 对象",
                        "invalid_json_shape",
                        None,
                    ));
                }
                Err(error) if error.code == "DL001" => return Err(error),
                Err(error) => last_error = Some(error),
            }
            if round + 1 < CMR_REQUEST_ROUNDS {
                wait_metadata_retry(round + 1, last_error.as_ref(), cancelled)?;
            }
        }
        Err(last_error.unwrap_or_else(|| {
            http_error(
                "ASF002",
                "CMR 受限数据集检索失败：网络请求未完成",
                "request_failed",
                None,
            )
        }))
    }

    /// Authenticated CMR granule count for the restricted legacy SAR
    /// missions, read from the `CMR-Hits` response header of a one-record
    /// page (the same signal CMR exposes for open collections).
    pub(crate) fn cmr_legacy_count(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        self.check_cancelled(cancelled)?;
        let Some(token_provider) = self.legacy_cmr_token.as_ref() else {
            write_legacy_cmr_diagnostic(&self.endpoints.cmr_granules, params, false, None);
            return Err(http_error(
                "ASF_LEGACY_LOGIN_REQUIRED",
                "该历史卫星数据集为受限馆藏，检索需要 Earthdata 登录凭据",
                "legacy_cmr_token_missing",
                None,
            ));
        };
        let Some(token) = token_provider().filter(|token| !token.trim().is_empty()) else {
            write_legacy_cmr_diagnostic(&self.endpoints.cmr_granules, params, false, None);
            return Err(http_error(
                "ASF_LEGACY_LOGIN_REQUIRED",
                "该历史卫星数据集为受限馆藏，检索需要 Earthdata 登录凭据",
                "legacy_cmr_token_unavailable",
                None,
            ));
        };
        self.check_cancelled(cancelled)?;
        let request = self
            .client
            .get(self.endpoints.cmr_granules.clone())
            .query(params)
            .bearer_auth(token.trim());
        let started = Instant::now();
        let request_summary = safe_request_summary(&request);
        let response = match request.send() {
            Ok(response) => response,
            Err(error) => {
                let error = sanitized_transport_error(error);
                write_http_diagnostic(request_summary.as_ref(), started, None, None, Some(&error));
                return Err(error);
            }
        };
        self.check_cancelled(cancelled)?;
        let status = response.status().as_u16();
        let hits = response
            .headers()
            .get("CMR-Hits")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let bytes = match read_bounded(response, self.limits.max_count_bytes, cancelled) {
            Ok(bytes) => bytes,
            Err(error) => {
                write_http_diagnostic(
                    request_summary.as_ref(),
                    started,
                    Some(status),
                    None,
                    Some(&error),
                );
                return Err(error);
            }
        };
        write_http_diagnostic(
            request_summary.as_ref(),
            started,
            Some(status),
            returned_record_count(&bytes),
            None,
        );
        if !(200..300).contains(&status) {
            return Err(http_error(
                "ASF002",
                "CMR 受限数据集计数失败",
                "http_status",
                Some(status),
            ));
        }
        write_legacy_cmr_diagnostic(&self.endpoints.cmr_granules, params, true, hits);
        Ok(hits)
    }

    /// One bounded CMR UMM request using the server's stateless Search After
    /// contract. `page_num` is deliberately removed because CMR forbids mixing
    /// it with the `CMR-Search-After` request header.
    pub(crate) fn cmr_catalog_page(
        &self,
        params: &[(String, String)],
        search_after: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CmrUmmPage> {
        self.check_cancelled(cancelled)?;
        let search_params = params
            .iter()
            .filter(|(name, _)| name != "page_num")
            .collect::<Vec<_>>();
        let mut request = self
            .client
            .get(self.endpoints.cmr_umm_granules.clone())
            .query(&search_params);
        if let Some(value) = search_after {
            request = request.header("CMR-Search-After", value);
        }
        let started = Instant::now();
        let request_summary = safe_request_summary(&request);
        let response = match request.send() {
            Ok(response) => response,
            Err(error) => {
                let error = sanitized_transport_error(error);
                write_http_diagnostic(request_summary.as_ref(), started, None, None, Some(&error));
                return Err(error);
            }
        };
        self.check_cancelled(cancelled)?;
        let status = response.status().as_u16();
        let search_after = response
            .headers()
            .get("CMR-Search-After")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let timed_out = ["CMR-Time-Out", "CMR-Timed-Out"]
            .iter()
            .filter_map(|name| response.headers().get(*name))
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.eq_ignore_ascii_case("true"));
        let bytes = match read_bounded(response, self.limits.max_json_bytes, cancelled) {
            Ok(bytes) => bytes,
            Err(error) => {
                write_http_diagnostic(
                    request_summary.as_ref(),
                    started,
                    Some(status),
                    None,
                    Some(&error),
                );
                return Err(error);
            }
        };
        write_http_diagnostic(
            request_summary.as_ref(),
            started,
            Some(status),
            returned_record_count(&bytes),
            None,
        );
        let value = serde_json::from_slice(&bytes).map_err(|_| {
            http_error(
                "ASF002",
                "元数据服务返回的 JSON 内容无效",
                "invalid_json",
                None,
            )
        })?;
        Ok(CmrUmmPage {
            value,
            search_after,
            timed_out,
            search_after_capable: true,
        })
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    pub(crate) fn cmr_catalog_json_with_status(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<(u16, usize, Value)> {
        self.check_cancelled(cancelled)?;
        let request = self
            .client
            .get(self.endpoints.cmr_umm_granules.clone())
            .query(params);
        let response = request.send().map_err(sanitized_transport_error)?;
        self.check_cancelled(cancelled)?;
        let status = response.status().as_u16();
        let bytes = read_bounded(response, self.limits.max_json_bytes, cancelled)?;
        let body_bytes = bytes.len();
        let value = serde_json::from_slice(&bytes).map_err(|_| {
            http_error(
                "ASF002",
                "元数据服务返回的 JSON 内容无效",
                "invalid_json",
                None,
            )
        })?;
        Ok((status, body_bytes, value))
    }

    fn send_json(
        &self,
        request: RequestBuilder,
        limit: usize,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        let bytes = self.send_bytes(request, limit, cancelled)?;
        serde_json::from_slice(&bytes).map_err(|_| {
            http_error(
                "ASF002",
                "元数据服务返回的 JSON 内容无效",
                "invalid_json",
                None,
            )
        })
    }

    fn send_bytes(
        &self,
        request: RequestBuilder,
        limit: usize,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Vec<u8>> {
        self.check_cancelled(cancelled)?;
        let started = Instant::now();
        let request_summary = safe_request_summary(&request);
        let response = match request.send() {
            Ok(response) => response,
            Err(error) => {
                let error = sanitized_transport_error(error);
                write_http_diagnostic(request_summary.as_ref(), started, None, None, Some(&error));
                return Err(error);
            }
        };
        self.check_cancelled(cancelled)?;
        let status = response.status().as_u16();
        let result = read_bounded(response, limit, cancelled);
        match &result {
            Ok(bytes) => write_http_diagnostic(
                request_summary.as_ref(),
                started,
                Some(status),
                returned_record_count(bytes),
                None,
            ),
            Err(error) => write_http_diagnostic(
                request_summary.as_ref(),
                started,
                Some(status),
                None,
                Some(error),
            ),
        }
        result
    }

    fn check_cancelled(&self, cancelled: &(dyn Fn() -> bool + Sync)) -> ProviderResult<()> {
        if cancelled() {
            Err(cancelled_error())
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
struct SafeRequestSummary {
    method: String,
    endpoint: String,
}

fn safe_request_summary(builder: &RequestBuilder) -> Option<SafeRequestSummary> {
    let request = builder.try_clone()?.build().ok()?;
    Some(SafeRequestSummary {
        method: request.method().as_str().to_owned(),
        endpoint: safe_endpoint(request.url()),
    })
}

fn safe_endpoint(url: &Url) -> String {
    let host = url.host_str().unwrap_or("unknown-host");
    let port = url
        .port()
        .map(|value| format!(":{value}"))
        .unwrap_or_default();
    format!("{}://{}{}{}", url.scheme(), host, port, url.path())
}

fn diagnostic_enabled() -> bool {
    std::env::var("INSAR_DIAGNOSTIC_ASF_SEARCH")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
}

/// `INSAR_DIAGNOSTIC_CATALOGUE` gate, shared with the service layer's
/// catalogue diagnostics. Emits one structured line per legacy-CMR request:
/// endpoint path, requested collection short names, the response's CMR-Hits
/// header, and whether a bearer token was attached. The token value itself
/// is never included.
fn legacy_cmr_diagnostic_enabled() -> bool {
    std::env::var_os("INSAR_DIAGNOSTIC_CATALOGUE").is_some()
        || std::env::var_os("INSAR_DIAGNOSTIC_ASF_SEARCH").is_some()
}

fn write_legacy_cmr_diagnostic(
    endpoint: &Url,
    params: &[(String, String)],
    token_attached: bool,
    hits: Option<u64>,
) {
    if !legacy_cmr_diagnostic_enabled() {
        return;
    }
    let short_names = params
        .iter()
        .filter(|(name, _)| name == "shortName")
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let hits = hits
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unreported".to_owned());
    eprintln!(
        "INSAR_CATALOGUE_DIAGNOSTIC route=legacy_cmr endpoint={} collections={} token_attached={} hits={}",
        safe_endpoint(endpoint),
        short_names,
        token_attached,
        hits
    );
}

fn returned_record_count(bytes: &[u8]) -> Option<u64> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .get("features")
        .and_then(Value::as_array)
        .or_else(|| value.get("items").and_then(Value::as_array))
        .or_else(|| {
            value
                .get("feed")
                .and_then(|feed| feed.get("entry"))
                .and_then(Value::as_array)
        })
        .and_then(|items| u64::try_from(items.len()).ok())
}

fn write_http_diagnostic(
    request: Option<&SafeRequestSummary>,
    started: Instant,
    status: Option<u16>,
    returned_records: Option<u64>,
    error: Option<&ContractError>,
) {
    if !diagnostic_enabled() {
        return;
    }
    let event = serde_json::json!({
        "event": "asf_http_request_finished",
        "method": request.map(|value| value.method.as_str()),
        "endpoint": request.map(|value| value.endpoint.as_str()),
        "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "http_status": status,
        "returned_records": returned_records,
        "error_code": error.map(|value| value.code.as_str()),
    });
    eprintln!("INSAR_ASF_HTTP_DIAGNOSTIC {event}");
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn safe_endpoint_drops_credentials_query_and_fragment() {
        let url = Url::parse("https://owner:secret@example.test/search/path?token=secret#private")
            .expect("test URL");

        let endpoint = safe_endpoint(&url);

        assert_eq!(endpoint, "https://example.test/search/path");
        assert!(!endpoint.contains("owner"));
        assert!(!endpoint.contains("secret"));
        assert!(!endpoint.contains("token"));
    }

    #[test]
    fn returned_record_count_accepts_asf_and_cmr_shapes() {
        assert_eq!(returned_record_count(br#"{"features":[{},{}]}"#), Some(2));
        assert_eq!(returned_record_count(br#"{"items":[{}]}"#), Some(1));
        assert_eq!(
            returned_record_count(br#"{"feed":{"entry":[{},{},{}]}}"#),
            Some(3)
        );
        assert_eq!(returned_record_count(b"640"), None);
    }
}

fn asf_request_methods(endpoint: &Url, params: &BTreeMap<String, String>) -> Vec<Method> {
    if params.contains_key("intersectsWith") {
        return vec![Method::POST];
    }
    let mut encoded = endpoint.clone();
    {
        let mut query = encoded.query_pairs_mut();
        for (name, value) in params {
            query.append_pair(name, value);
        }
    }
    if encoded.as_str().len() > MAX_DIRECT_GET_URL_BYTES {
        vec![Method::POST]
    } else {
        vec![Method::GET, Method::POST]
    }
}

fn wait_metadata_retry(
    round: u32,
    error: Option<&ContractError>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ProviderResult<()> {
    let provider_delay = error
        .and_then(|error| error.debug_context.get("retry_after_seconds"))
        .and_then(Value::as_u64)
        .map(|seconds| Duration::from_secs(seconds.min(MAX_RETRY_AFTER_SECONDS)));
    let delay = provider_delay.unwrap_or_else(|| METADATA_RETRY_STEP.saturating_mul(round));
    let step = Duration::from_millis(50);
    let steps = delay.as_millis().div_ceil(step.as_millis()).max(1);
    for _ in 0..steps {
        if cancelled() {
            return Err(cancelled_error());
        }
        thread::sleep(step.min(delay));
    }
    if cancelled() {
        return Err(cancelled_error());
    }
    Ok(())
}

fn read_bounded(
    mut response: Response,
    limit: usize,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ProviderResult<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        let retry_after_seconds = parse_retry_after_seconds(&response);
        let mut diagnostic = read_redacted_error_body(&mut response, cancelled)?;
        diagnostic.retry_after_seconds = retry_after_seconds;
        let (code, message) = match status.as_u16() {
            408 => (
                "ASF_TIMEOUT",
                "元数据服务请求超时，请缩小检索范围或稍后重试",
            ),
            429 => (
                "ASF_RATE_LIMITED",
                "元数据服务当前请求过多，请按服务端建议稍后重试",
            ),
            500..=599 => ("ASF_SERVER_ERROR", "元数据服务暂时不可用，请稍后重试"),
            _ => ("ASF002", "元数据服务暂时未能完成请求"),
        };
        return Err(http_error_with_diagnostic(
            code,
            message,
            "http_status",
            Some(status.as_u16()),
            diagnostic,
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(http_error(
            "ASF_RESPONSE_TOO_LARGE",
            "元数据服务返回内容超过安全上限",
            "response_too_large",
            None,
        ));
    }
    let mut bytes =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(limit as u64) as usize);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if cancelled() {
            return Err(cancelled_error());
        }
        let count = response.read(&mut buffer).map_err(|error| {
            let category = if error.kind() == std::io::ErrorKind::TimedOut {
                "timeout"
            } else {
                "response_read"
            };
            let code = if category == "timeout" {
                "ASF_TIMEOUT"
            } else {
                "ASF_TRANSPORT_FAILED"
            };
            http_error(code, "元数据服务响应读取失败", category, None)
        })?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > limit {
            return Err(http_error(
                "ASF_RESPONSE_TOO_LARGE",
                "元数据服务返回内容超过安全上限",
                "response_too_large",
                None,
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if cancelled() {
        return Err(cancelled_error());
    }
    Ok(bytes)
}

#[derive(Debug)]
struct RedactedErrorBody {
    service_error_class: &'static str,
    safe_reason_code: Option<String>,
    field: Option<String>,
    body_sha256: String,
    body_bytes: usize,
    retry_after_seconds: Option<u64>,
}

fn read_redacted_error_body(
    response: &mut Response,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ProviderResult<RedactedErrorBody> {
    let declared_too_large = response
        .content_length()
        .is_some_and(|length| length > MAX_ERROR_BODY_BYTES as u64);
    let mut bytes = Vec::with_capacity(MAX_ERROR_BODY_BYTES.min(4096));
    let mut buffer = [0_u8; 4096];
    while bytes.len() < MAX_ERROR_BODY_BYTES {
        if cancelled() {
            return Err(cancelled_error());
        }
        let read_size = (MAX_ERROR_BODY_BYTES - bytes.len()).min(buffer.len());
        let count = response.read(&mut buffer[..read_size]).map_err(|error| {
            let category = if error.kind() == std::io::ErrorKind::TimedOut {
                "response_read_timeout"
            } else {
                "response_read"
            };
            let code = if category == "response_read_timeout" {
                "ASF_TIMEOUT"
            } else {
                "ASF_TRANSPORT_FAILED"
            };
            http_error(code, "元数据服务响应读取失败", category, None)
        })?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if cancelled() {
        return Err(cancelled_error());
    }

    let mut digest = Sha256::new();
    digest.update(&bytes);
    let body_sha256 = format!("{:x}", digest.finalize());
    let (service_error_class, safe_reason_code, field) = if declared_too_large {
        (
            "oversized_error_body",
            Some("body_too_large".to_owned()),
            None,
        )
    } else {
        classify_redacted_error_body(&bytes)
    };

    Ok(RedactedErrorBody {
        service_error_class,
        safe_reason_code,
        field,
        body_sha256,
        body_bytes: bytes.len(),
        retry_after_seconds: None,
    })
}

fn parse_retry_after_seconds(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn classify_redacted_error_body(bytes: &[u8]) -> (&'static str, Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return ("unparsed_error_body", None, None);
    };
    let Some(object) = value.as_object() else {
        return ("unparsed_error_body", None, None);
    };
    let source = object
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find_map(Value::as_object))
        .unwrap_or(object);
    let safe_reason_code = ["error_code", "code", "reason", "error_type", "type"]
        .iter()
        .find_map(|key| source.get(*key).and_then(Value::as_str))
        .and_then(safe_reason_code);
    let field = ["field", "parameter", "parameter_name"]
        .iter()
        .find_map(|key| source.get(*key).and_then(Value::as_str))
        .and_then(allowlisted_error_field);
    ("json_error_body", safe_reason_code, field)
}

fn safe_reason_code(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 64 {
        return None;
    }
    if value.bytes().all(|byte| {
        byte.is_ascii_uppercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'_' | b'-' | b'.' | b':')
    }) {
        Some(value.to_owned())
    } else {
        None
    }
}

fn allowlisted_error_field(value: &str) -> Option<String> {
    match value {
        "collection_concept_id"
        | "temporal"
        | "polygon"
        | "attribute[]"
        | "options[attribute][exclude_collection]"
        | "page_size"
        | "page_num"
        | "sort_key[]"
        | "granule_ur" => Some(value.to_owned()),
        _ => None,
    }
}

fn parse_count(bytes: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if let Ok(count) = text.parse::<u64>() {
        return Some(count);
    }
    let value: Value = serde_json::from_str(text).ok()?;
    if let Some(count) = value.as_u64() {
        return Some(count);
    }
    let object = value.as_object()?;
    ["count", "total", "totalResults", "hits"]
        .iter()
        .find_map(|key| {
            object.get(*key).and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            })
        })
}

fn sanitized_transport_error(error: reqwest::Error) -> ContractError {
    let (code, message, category) = if error.is_timeout() {
        (
            "ASF_TIMEOUT",
            "元数据服务请求超时，请缩小检索范围或检查网络代理",
            "timeout",
        )
    } else if error.is_connect() {
        (
            "ASF_TRANSPORT_FAILED",
            "无法连接元数据服务，请检查网络、DNS 或代理设置",
            "connect",
        )
    } else if error.is_builder() {
        ("ASF002", "元数据服务请求参数无效", "request_build")
    } else {
        (
            "ASF_TRANSPORT_FAILED",
            "元数据服务网络请求未完成",
            "transport",
        )
    };
    http_error(code, message, category, None)
}

fn retryable_metadata_error(error: &ContractError) -> bool {
    if error.code == "ASF_RESPONSE_TOO_LARGE" {
        return false;
    }
    match error.debug_context.get("status").and_then(Value::as_u64) {
        Some(408 | 425 | 429) => true,
        Some(400..=499) => false,
        _ => true,
    }
}

fn cancelled_error() -> ContractError {
    http_error("DL001", "ASF 检索已停止", "cancelled", None)
}

fn http_error(code: &str, message: &str, category: &str, status: Option<u16>) -> ContractError {
    let mut context =
        Map::from_iter([("category".to_string(), Value::String(category.to_string()))]);
    if let Some(status) = status {
        context.insert("status".to_string(), Value::from(status));
    }
    ContractError::new(code, message, context).expect("bounded HTTP context is safe")
}

fn http_error_with_diagnostic(
    code: &str,
    message: &str,
    category: &str,
    status: Option<u16>,
    diagnostic: RedactedErrorBody,
) -> ContractError {
    let mut context =
        Map::from_iter([("category".to_string(), Value::String(category.to_string()))]);
    if let Some(status) = status {
        context.insert("status".to_string(), Value::from(status));
    }
    context.insert(
        "service_error_class".to_owned(),
        Value::String(diagnostic.service_error_class.to_owned()),
    );
    if let Some(reason) = diagnostic.safe_reason_code {
        context.insert("safe_reason_code".to_owned(), Value::String(reason));
    }
    if let Some(field) = diagnostic.field {
        context.insert("field".to_owned(), Value::String(field));
    }
    context.insert(
        "body_sha256".to_owned(),
        Value::String(diagnostic.body_sha256),
    );
    context.insert("body_bytes".to_owned(), Value::from(diagnostic.body_bytes));
    if let Some(seconds) = diagnostic.retry_after_seconds {
        context.insert("retry_after_seconds".to_owned(), Value::from(seconds));
    }
    ContractError::new(code, message, context).expect("bounded HTTP diagnostic is safe")
}

fn wkt_bounding_box(wkt: &str) -> Option<(f64, f64, f64, f64)> {
    let mut west = f64::INFINITY;
    let mut south = f64::INFINITY;
    let mut east = f64::NEG_INFINITY;
    let mut north = f64::NEG_INFINITY;
    let mut any = false;
    for part in wkt.split(',') {
        let cleaned = part.trim_matches(|ch: char| {
            ch.is_ascii_alphabetic() || ch.is_ascii_whitespace() || ch == '(' || ch == ')'
        });
        let mut iter = cleaned.split_ascii_whitespace();
        let (Some(lon_raw), Some(lat_raw)) = (iter.next(), iter.next()) else {
            continue;
        };
        let (Ok(lon), Ok(lat)) = (lon_raw.parse::<f64>(), lat_raw.parse::<f64>()) else {
            continue;
        };
        if !lon.is_finite() || !lat.is_finite() {
            continue;
        }
        west = west.min(lon);
        east = east.max(lon);
        south = south.min(lat);
        north = north.max(lat);
        any = true;
    }
    (any && west < east && south < north).then_some((west, south, east, north))
}

fn cmr_entry_to_asf_geojson_feature(
    entry: &Map<String, Value>,
) -> Option<insar_core::AsfRawFeature> {
    let mut feature = super::cmr::cmr_entry_to_feature(entry)?;
    let scene_name = feature
        .properties
        .get("sceneName")
        .and_then(Value::as_str)?
        .to_string();
    let parsed = insar_core::parse_sentinel1_identifier(&scene_name).ok()?;
    let title = entry
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(scene_name.as_str());
    let file_id = if title.ends_with("-SLC") || title.contains("-GRD") {
        title.to_string()
    } else {
        format!("{scene_name}-{}", parsed.product_type)
    };
    feature
        .properties
        .insert("fileID".to_string(), Value::String(file_id.clone()));
    feature
        .properties
        .insert("productID".to_string(), Value::String(file_id));
    feature
        .properties
        .insert("granuleName".to_string(), Value::String(scene_name.clone()));
    feature
        .properties
        .entry("fileName".to_string())
        .or_insert_with(|| Value::String(format!("{scene_name}.zip")));
    let platform = match parsed.platform.as_str() {
        "S1A" => "Sentinel-1A",
        "S1B" => "Sentinel-1B",
        "S1C" => "Sentinel-1C",
        "S1D" => "Sentinel-1D",
        other => other,
    };
    feature
        .properties
        .insert("platform".to_string(), Value::String(platform.to_string()));
    feature
        .properties
        .insert("instrument".to_string(), Value::String("C-SAR".to_string()));
    let processing_level = if parsed.product_type == "GRD" {
        match (parsed.resolution.as_str(), parsed.polarization.as_str()) {
            ("H", "DV" | "DH") => "GRD_HD",
            ("H", _) => "GRD_HS",
            ("M", "DV" | "DH") => "GRD_MD",
            ("M", _) => "GRD_MS",
            ("F", _) => "GRD_FD",
            _ => "GRD_HD",
        }
    } else {
        parsed.product_type.as_str()
    };
    feature.properties.insert(
        "processingLevel".to_string(),
        Value::String(processing_level.to_string()),
    );
    feature.properties.insert(
        "beamMode".to_string(),
        Value::String(parsed.beam_mode.clone()),
    );
    feature.properties.insert(
        "beamModeType".to_string(),
        Value::String(parsed.beam_mode.clone()),
    );
    let polarization = match parsed.polarization.as_str() {
        "DV" => "VV+VH",
        "DH" => "HH+HV",
        "SV" => "VV",
        "SH" => "HH",
        other => other,
    };
    feature.properties.insert(
        "polarization".to_string(),
        Value::String(polarization.to_string()),
    );
    if let Some(path) = feature.properties.get("pathNumber").cloned() {
        feature.properties.insert("relativeOrbit".to_string(), path);
    }
    if let Some(start) = entry
        .get("time_start")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        feature
            .properties
            .insert("startTime".to_string(), Value::String(start.to_string()));
    }
    if let Some(stop) = entry
        .get("time_end")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        feature
            .properties
            .insert("stopTime".to_string(), Value::String(stop.to_string()));
    }
    Some(feature)
}

fn cmr_feature_matches_asf_params(
    feature: &insar_core::AsfRawFeature,
    params: &BTreeMap<String, String>,
) -> bool {
    let split_tokens = |raw: &str| -> Vec<String> {
        raw.split(',')
            .map(|item| item.trim().to_ascii_uppercase().replace(' ', "+"))
            .filter(|item| !item.is_empty())
            .collect()
    };

    if let Some(raw_pol) = params.get("polarization") {
        let wanted = split_tokens(raw_pol);
        if !wanted.is_empty() {
            let actual = feature
                .properties
                .get("polarization")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_ascii_uppercase().replace(' ', "+"))
                .unwrap_or_default();
            if !wanted.iter().any(|w| w == &actual) {
                return false;
            }
        }
    }

    if let Some(raw_beam) = params.get("beamMode") {
        let wanted = split_tokens(raw_beam);
        if !wanted.is_empty() {
            let actual = feature
                .properties
                .get("beamMode")
                .or_else(|| feature.properties.get("beamModeType"))
                .and_then(Value::as_str)
                .map(|s| s.trim().to_ascii_uppercase())
                .unwrap_or_default();
            if !wanted.iter().any(|w| w == &actual) {
                return false;
            }
        }
    }

    if let Some(raw_level) = params.get("processingLevel") {
        let wanted = split_tokens(raw_level);
        if !wanted.is_empty() {
            let actual = feature
                .properties
                .get("processingLevel")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_ascii_uppercase())
                .unwrap_or_default();
            let matched = wanted
                .iter()
                .any(|w| w == &actual || (w == "GRD" && actual.starts_with("GRD")));
            if !matched {
                return false;
            }
        }
    }

    if let Some(raw_dir) = params.get("flightDirection") {
        let normalize_dir = |s: &str| match s.trim().to_ascii_uppercase().as_str() {
            "A" | "ASC" | "ASCENDING" => Some("ASCENDING"),
            "D" | "DESC" | "DESCENDING" => Some("DESCENDING"),
            _ => None,
        };
        let wanted: Vec<&'static str> = raw_dir.split(',').filter_map(normalize_dir).collect();
        if !wanted.is_empty() {
            let actual = feature
                .properties
                .get("flightDirection")
                .and_then(Value::as_str)
                .and_then(normalize_dir);
            if !actual.is_some_and(|a| wanted.contains(&a)) {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod cmr_fallback_tests {
    use super::*;

    #[test]
    fn cmr_fallback_post_filter_enforces_polarization_beam_level_and_direction() {
        let entry = serde_json::json!({
            "title": "S1C_IW_SLC__1SDV_20261028T110512_20261028T110539_004512_008A1F_12AB-SLC",
            "time_start": "2026-10-28T11:05:12Z",
            "time_end": "2026-10-28T11:05:39Z",
            "polygons": [["30.0 103.0 30.0 104.0 31.0 104.0 31.0 103.0 30.0 103.0"]],
            "additional_attributes": [
                {"name": "ASCENDING_DESCENDING", "values": ["ASCENDING"]}
            ]
        });
        let feature = cmr_entry_to_asf_geojson_feature(entry.as_object().unwrap())
            .expect("S1C SLC entry parses");
        assert_eq!(
            feature.properties.get("platform").and_then(Value::as_str),
            Some("Sentinel-1C")
        );
        assert_eq!(
            feature
                .properties
                .get("polarization")
                .and_then(Value::as_str),
            Some("VV+VH")
        );

        let matching = BTreeMap::from([
            ("polarization".to_string(), "VV,VV+VH".to_string()),
            ("beamMode".to_string(), "IW".to_string()),
            ("processingLevel".to_string(), "SLC".to_string()),
            ("flightDirection".to_string(), "ASCENDING".to_string()),
        ]);
        assert!(cmr_feature_matches_asf_params(&feature, &matching));

        let hh_only = BTreeMap::from([("polarization".to_string(), "HH".to_string())]);
        assert!(
            !cmr_feature_matches_asf_params(&feature, &hh_only),
            "HH-only query must never leak VV+VH scenes via CMR fallback"
        );

        let desc_only = BTreeMap::from([("flightDirection".to_string(), "DESCENDING".to_string())]);
        assert!(!cmr_feature_matches_asf_params(&feature, &desc_only));
    }
}
