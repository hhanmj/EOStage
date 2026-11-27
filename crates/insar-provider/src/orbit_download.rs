use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use insar_core::{
    parse_orbit_filenames, ContractError, OrbitParseRequest, OrbitProductType,
    ProviderNetworkPolicy, ProviderResult,
};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_RANGE, RANGE};
use reqwest::Url;
use serde_json::{Map, Value};

use crate::asf::download::ResolvedDownloadSource;

const ORBIT_SCENE_ENDPOINT: &str = "https://s1-orbits.asf.alaska.edu/scene/";

/// POEORB 精密轨道通常在场景采集后约 20 天发布。
const POEORB_EXPECTED_PUBLICATION_DAYS: i64 = 20;
/// 超过该天数仍未发布时，错误信息提示发布偏晚。
const POEORB_LATE_PUBLICATION_HINT_DAYS: i64 = 25;
/// 等待 POEORB 发布的时间上限（采集后 60 天）。
const POEORB_WAIT_LIMIT_DAYS: i64 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrbitResolverEndpoint(Url);

impl OrbitResolverEndpoint {
    pub fn production() -> Self {
        Self(Url::parse(ORBIT_SCENE_ENDPOINT).expect("official orbit URL is valid"))
    }

    #[doc(hidden)]
    pub fn loopback_for_tests(value: &str) -> Result<Self, String> {
        let url = Url::parse(value).map_err(|_| "test endpoint must be a valid URL")?;
        let loopback = url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
        if url.scheme() != "http"
            || !loopback
            || !url.username().is_empty()
            || url.password().is_some()
            || !url.path().ends_with('/')
        {
            return Err(
                "test endpoint must be an unauthenticated HTTP loopback directory".to_string(),
            );
        }
        Ok(Self(url))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone)]
pub struct OrbitHttpClient {
    client: Client,
    endpoint: OrbitResolverEndpoint,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl OrbitHttpClient {
    pub fn production() -> ProviderResult<Self> {
        Self::production_with_network_policy(ProviderNetworkPolicy::system())
    }

    pub fn production_with_network_policy(
        network_policy: ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        Self::build(OrbitResolverEndpoint::production(), network_policy, false)
    }

    #[doc(hidden)]
    pub fn with_test_endpoint(endpoint: OrbitResolverEndpoint) -> ProviderResult<Self> {
        Self::with_test_endpoint_and_network_policy(endpoint, ProviderNetworkPolicy::direct())
    }

    #[doc(hidden)]
    pub fn with_test_endpoint_and_network_policy(
        endpoint: OrbitResolverEndpoint,
        network_policy: ProviderNetworkPolicy,
    ) -> ProviderResult<Self> {
        if endpoint.0.scheme() != "http"
            || !endpoint
                .0
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|address| address.is_loopback())
        {
            return Err(error(
                "V3_PROVIDER_CONFIGURATION_INVALID",
                "测试 Orbit Provider 只允许本机回环地址",
                "invalid_test_endpoint",
            ));
        }
        Self::build(endpoint, network_policy, true)
    }

    fn build(
        endpoint: OrbitResolverEndpoint,
        network_policy: ProviderNetworkPolicy,
        test_endpoint: bool,
    ) -> ProviderResult<Self> {
        let mut builder = Client::builder()
            .user_agent("EOStage/3 orbit resolver")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60));
        if test_endpoint {
            builder = builder.tls_built_in_root_certs(false);
        }
        let client =
            crate::network_proxy::blocking_builder_with_proxy(builder, network_policy.route())
                .build()
                .map_err(|_| {
                    error(
                        "V3_PROVIDER_CONFIGURATION_INVALID",
                        "无法初始化 Orbit 下载解析器",
                        "client_build",
                    )
                })?;
        Ok(Self {
            client,
            endpoint,
            now: Arc::new(Utc::now),
        })
    }

    #[doc(hidden)]
    pub fn with_test_clock(mut self, now: DateTime<Utc>) -> Self {
        self.now = Arc::new(move || now);
        self
    }

    pub fn resolve(
        &self,
        scene_id: &str,
        acquisition_datetime: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<ResolvedDownloadSource> {
        self.resolve_with_orbit_type(
            scene_id,
            acquisition_datetime,
            OrbitProductType::POEORB,
            cancelled,
        )
    }

    /// Orbit scheduled-fetch S7: explicit manual temporary RESORB option.
    /// Default acquisition continues to call `resolve` (strict `POEORB`);
    /// only an explicit user choice requests `OrbitProductType::RESORB`.
    pub fn resolve_with_orbit_type(
        &self,
        scene_id: &str,
        acquisition_datetime: Option<&str>,
        requested_orbit_type: OrbitProductType,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<ResolvedDownloadSource> {
        validate_scene_id(scene_id)?;
        if cancelled() {
            return Err(error("DL001", "轨道下载解析已停止", "cancelled"));
        }
        let url = self
            .endpoint
            .0
            .join(scene_id)
            .map_err(|_| error("ORB001", "场景标识无法用于轨道下载", "invalid_scene_id"))?;
        let response = self
            .client
            .get(url)
            .header(RANGE, "bytes=0-0")
            .send()
            .map_err(|request_error| {
                let category = if request_error.is_timeout() {
                    "timeout"
                } else if request_error.is_connect() {
                    "connect"
                } else {
                    "transport"
                };
                error("ORB001", "无法连接 ASF 精密轨道服务", category)
            })?;
        if cancelled() {
            return Err(error("DL001", "轨道下载解析已停止", "cancelled"));
        }
        // Only "not found" means the orbit is not published yet; server or
        // access failures stay ORB001 so they are never scheduled as waits.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return match requested_orbit_type {
                OrbitProductType::POEORB => Err(unpublished_poeorb_error(
                    (self.now)(),
                    acquisition_datetime,
                    OrbitAbsence::NotReturned,
                )),
                OrbitProductType::RESORB | OrbitProductType::MOEORB | OrbitProductType::UNKNOWN => {
                    Err(error(
                        "ORB001",
                        "ASF 当前未返回该场景的 RESORB 快速轨道，请稍后重试",
                        "resorb_not_returned",
                    ))
                }
            };
        }
        if !response.status().is_success() {
            return Err(error(
                "ORB001",
                match requested_orbit_type {
                    OrbitProductType::POEORB => {
                        "ASF 当前未返回该场景的 POEORB 精密轨道，请稍后重试"
                    }
                    OrbitProductType::RESORB
                    | OrbitProductType::MOEORB
                    | OrbitProductType::UNKNOWN => {
                        "ASF 当前未返回该场景的 RESORB 快速轨道，请稍后重试"
                    }
                },
                "unavailable",
            ));
        }
        let file_name = response_file_name(&response).ok_or_else(|| {
            error(
                "ORB001",
                "ASF 轨道响应没有合法的 EOF 文件名",
                "missing_file_name",
            )
        })?;
        let parsed = parse_orbit_filenames(&OrbitParseRequest {
            file_names: vec![file_name.clone()],
        });
        let orbit = parsed.orbit_files.first().ok_or_else(|| {
            error(
                "ORB001",
                "ASF 轨道响应的 EOF 文件名无效",
                "invalid_orbit_name",
            )
        })?;
        if orbit.orbit_type != requested_orbit_type {
            return match requested_orbit_type {
                OrbitProductType::POEORB => Err(unpublished_poeorb_error(
                    (self.now)(),
                    acquisition_datetime,
                    OrbitAbsence::NonPoeorbReturned,
                )),
                OrbitProductType::RESORB | OrbitProductType::MOEORB | OrbitProductType::UNKNOWN => {
                    Err(error(
                        "ORB001",
                        "ASF 当前未返回该场景的 RESORB 快速轨道，请稍后重试",
                        "resorb_unavailable",
                    ))
                }
            };
        }
        let expected_size = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(content_range_total)
            .or_else(|| response.content_length());
        Ok(ResolvedDownloadSource::new(
            scene_id.to_string(),
            file_name,
            expected_size,
            response.url().clone(),
        ))
    }
}

/// 远端在「该场景还没有可用的 POEORB」时的两种形态。
enum OrbitAbsence {
    NotReturned,
    NonPoeorbReturned,
}

/// 把「ASF 未给出 POEORB」按景龄分类为 ORB002（尚未发布）或 ORB003（超出等待上限）；
/// 没有可解析的采集时间时无法判定景龄，保持既有的 ORB001 失败分类。
fn unpublished_poeorb_error(
    now: DateTime<Utc>,
    acquisition_datetime: Option<&str>,
    absence: OrbitAbsence,
) -> ContractError {
    let (shape_zh, shape_category, limit_category) = match absence {
        OrbitAbsence::NotReturned => (
            "ASF 尚未发布该场景的 POEORB 精密轨道",
            "not_published",
            "wait_limit_exceeded",
        ),
        OrbitAbsence::NonPoeorbReturned => (
            "ASF 当前只返回了非 POEORB 轨道",
            "not_published_non_poeorb",
            "wait_limit_exceeded_non_poeorb",
        ),
    };
    let Some(age_days) = scene_age_days(acquisition_datetime, now) else {
        return match absence {
            OrbitAbsence::NotReturned => error(
                "ORB001",
                "ASF 当前未返回该场景的 POEORB 精密轨道，请稍后重试",
                "unavailable",
            ),
            OrbitAbsence::NonPoeorbReturned => error(
                "ORB001",
                "ASF 当前只返回了非 POEORB 轨道；精密轨道发布后再重试",
                "poeorb_unavailable",
            ),
        };
    };
    if age_days >= POEORB_WAIT_LIMIT_DAYS {
        return error(
            "ORB003",
            &format!("{shape_zh}，等待发布已超过上限，请手动确认后重试"),
            limit_category,
        );
    }
    let age_hint = if age_days >= POEORB_LATE_PUBLICATION_HINT_DAYS {
        "，发布偏晚，请稍后重试"
    } else if age_days >= POEORB_EXPECTED_PUBLICATION_DAYS {
        "，请稍后重试"
    } else {
        "，场景采集时间较近，请稍后重试"
    };
    error("ORB002", &format!("{shape_zh}{age_hint}"), shape_category)
}

fn scene_age_days(acquisition_datetime: Option<&str>, now: DateTime<Utc>) -> Option<i64> {
    let acquisition = DateTime::parse_from_rfc3339(acquisition_datetime?).ok()?;
    Some(
        now.signed_duration_since(acquisition.with_timezone(&Utc))
            .num_days(),
    )
}

fn validate_scene_id(scene_id: &str) -> ProviderResult<()> {
    if scene_id.trim().is_empty()
        || !scene_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(error("ORB001", "轨道场景标识无效", "invalid_scene_id"));
    }
    Ok(())
}

fn response_file_name(response: &reqwest::blocking::Response) -> Option<String> {
    let disposition = response
        .headers()
        .get(CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if let Some(name) = disposition.split(';').find_map(|part| {
        part.trim()
            .strip_prefix("filename=")
            .map(|value| value.trim().trim_matches('"').to_string())
    }) {
        if is_basename(&name) {
            return Some(name);
        }
        return None;
    }
    response
        .url()
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| is_basename(name))
        .map(str::to_string)
}

fn is_basename(value: &str) -> bool {
    !value.is_empty() && !value.contains(['/', '\\', '\0']) && value.is_ascii()
}

fn content_range_total(value: &str) -> Option<u64> {
    let total = value.rsplit_once('/')?.1.trim();
    (total != "*").then(|| total.parse().ok()).flatten()
}

fn error(code: &str, message: &str, category: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("category".to_string(), Value::String(category.to_string()))]),
    )
    .expect("bounded orbit resolver error is valid")
}
