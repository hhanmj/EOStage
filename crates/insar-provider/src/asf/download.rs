use std::fmt;
use std::path::Path;

use insar_core::{ContractError, ProviderResult};
use reqwest::Url;
use serde_json::{Map, Value};

use super::cmr::{cmr_entry_to_feature, collection_ids};

pub trait DownloadMetadataTransport: Send + Sync {
    fn cmr_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value>;
}

impl DownloadMetadataTransport for super::client::AsfHttpClient {
    fn cmr_json(
        &self,
        params: &[(String, String)],
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.cmr_json(params, cancelled)
    }
}

#[derive(Clone)]
pub struct ResolvedDownloadSource {
    pub item_id: String,
    pub file_name: String,
    pub expected_size: Option<u64>,
    url: Url,
}

impl ResolvedDownloadSource {
    pub(crate) fn new(
        item_id: String,
        file_name: String,
        expected_size: Option<u64>,
        url: Url,
    ) -> Self {
        Self {
            item_id,
            file_name,
            expected_size,
            url,
        }
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }
}

impl fmt::Debug for ResolvedDownloadSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedDownloadSource")
            .field("item_id", &self.item_id)
            .field("file_name", &self.file_name)
            .field("expected_size", &self.expected_size)
            .field("url", &"[REDACTED]")
            .finish()
    }
}

pub fn resolve_frozen_asf_asset(
    item_id: &str,
    raw_url: &str,
    file_name: &str,
    expected_size: Option<u64>,
) -> ProviderResult<ResolvedDownloadSource> {
    validate_identity(item_id, file_name)?;
    let url = Url::parse(raw_url).ok().filter(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(trusted_asf_download_host)
    });
    let Some(url) = url else {
        return Err(error(
            "DL003",
            "ASF 下载地址必须是可信官方 HTTPS 地址",
            "untrusted_download_url",
        ));
    };
    Ok(ResolvedDownloadSource::new(
        item_id.to_string(),
        file_name.to_string(),
        expected_size,
        url,
    ))
}

fn trusted_asf_download_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "asf.alaska.edu"
        || host.ends_with(".asf.alaska.edu")
        || host == "earthdatacloud.nasa.gov"
        || host.ends_with(".earthdatacloud.nasa.gov")
}

pub fn resolve_asf_download(
    transport: &dyn DownloadMetadataTransport,
    scene_id: &str,
    frozen_file_name: &str,
    product_type: &str,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ProviderResult<ResolvedDownloadSource> {
    validate_identity(scene_id, frozen_file_name)?;
    if cancelled() {
        return Err(error("DL001", "下载解析已停止", "cancelled"));
    }
    let collections = collection_ids(product_type).ok_or_else(|| {
        error(
            "DL005",
            "当前产品类型不支持实时下载解析",
            "unsupported_product",
        )
    })?;
    let suffix = product_type.trim().to_ascii_uppercase();
    let mut params = vec![
        ("page_size".to_string(), "10".to_string()),
        ("granule_ur[]".to_string(), scene_id.to_string()),
        ("granule_ur[]".to_string(), format!("{scene_id}-{suffix}")),
    ];
    params.extend(collections.iter().map(|collection| {
        (
            "collection_concept_id".to_string(),
            (*collection).to_string(),
        )
    }));
    let response = transport.cmr_json(&params, cancelled)?;
    if cancelled() {
        return Err(error("DL001", "下载解析已停止", "cancelled"));
    }
    let entries = response
        .get("feed")
        .and_then(Value::as_object)
        .and_then(|feed| feed.get("entry"))
        .and_then(Value::as_array)
        .ok_or_else(|| error("DL005", "CMR 返回的下载元数据无效", "invalid_cmr_shape"))?;

    for entry in entries.iter().filter_map(Value::as_object) {
        let Some(feature) = cmr_entry_to_feature(entry) else {
            continue;
        };
        if feature.properties.get("sceneName").and_then(Value::as_str) != Some(scene_id) {
            continue;
        }
        let Some(raw_url) = feature.properties.get("url").and_then(Value::as_str) else {
            continue;
        };
        let url = Url::parse(raw_url)
            .ok()
            .filter(|url| {
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
            .ok_or_else(|| error("DL005", "CMR 返回的下载地址无效", "invalid_download_url"))?;
        let expected_size = feature.properties.get("bytes").and_then(Value::as_u64);
        return Ok(ResolvedDownloadSource::new(
            scene_id.to_string(),
            frozen_file_name.to_string(),
            expected_size,
            url,
        ));
    }
    Err(error(
        "DL005",
        "未找到该场景当前可用的官方下载地址，请重新检索后重试",
        "download_not_found",
    ))
}

fn validate_identity(scene_id: &str, file_name: &str) -> ProviderResult<()> {
    if scene_id.trim().is_empty() {
        return Err(error("DL003", "下载场景标识无效", "invalid_scene_id"));
    }
    if file_name.trim().is_empty()
        || Path::new(file_name)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(file_name)
        || file_name.contains(['/', '\\', '\0'])
    {
        return Err(error(
            "DL003",
            "下载文件名必须是不含目录的安全文件名",
            "invalid_file_name",
        ));
    }
    Ok(())
}

fn error(code: &str, message: &str, category: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("category".to_string(), Value::String(category.to_string()))]),
    )
    .expect("bounded resolver error is valid")
}
