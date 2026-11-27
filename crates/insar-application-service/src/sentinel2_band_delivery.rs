use super::*;

/// Shared scientific preparation for single-scene and batch band delivery.
/// A scene is returned only after every requested asset has been verified.
pub(super) fn prepare_sentinel2_band_items(
    source_item: &SnapshotItem,
    source_identity: &FrozenSentinel2AnalysisSource,
    asset_keys: &[String],
    evidence_port: &dyn LiveSentinel2BandEvidence,
    cancelled: &AtomicBool,
) -> Result<(String, Vec<SnapshotItem>), ApplicationServiceError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(ApplicationServiceError::Sentinel2BandCancelled);
    }
    if source_identity.source_provider_id
        == insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
    {
        let es_item_id = source_identity
            .earth_search_item_id
            .as_deref()
            .ok_or(ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let manifest = evidence_port
            .resolve_earth_search_band_manifest(
                es_item_id,
                &source_identity.safe_name,
                Some(&source_identity.acquisition_datetime),
                asset_keys,
                &|| cancelled.load(Ordering::Acquire),
            )
            .map_err(ApplicationServiceError::Provider)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::Sentinel2BandCancelled);
        }
        if manifest.item_id != es_item_id
            || manifest.safe_name != source_identity.safe_name
            || manifest
                .assets
                .iter()
                .map(|asset| &asset.asset_key)
                .collect::<Vec<_>>()
                != asset_keys.iter().collect::<Vec<_>>()
        {
            return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
        }
        insar_provider::catalog::validate_earth_search_sentinel2_source_id(
            &manifest.item_id,
            &manifest.safe_name,
            Some(&source_identity.acquisition_datetime),
        )
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let product_key = provider_product_key(
            insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
            insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
            &manifest.item_id,
        )
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let descriptive_assets = manifest
            .assets
            .iter()
            .map(|asset| {
                stable_asset_key(&product_key, &asset.asset_key)
                    .map(|stable_key| {
                        serde_json::json!({
                            "asset_key": asset.asset_key,
                            "stable_asset_key": stable_key,
                            "role": "primary",
                            "native_roles": ["data"],
                            "resolution_m": asset.resolution_m,
                        })
                    })
                    .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let reviewed_source = SnapshotItem {
            item_id: product_key.clone(),
            provider_id: insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            display_name: manifest.item_id.clone(),
            acquisition_datetime: Some(source_identity.acquisition_datetime.clone()),
            footprint_geojson: source_item.footprint_geojson.clone(),
            metadata: Map::from_iter([
                (
                    "logical_dataset".to_owned(),
                    Value::String("optical.sentinel-2.l2a".to_owned()),
                ),
                (
                    "native_collection_id".to_owned(),
                    Value::String(
                        insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID
                            .to_owned(),
                    ),
                ),
                (
                    "collection_profile_id".to_owned(),
                    Value::String(
                        insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID
                            .to_owned(),
                    ),
                ),
                (
                    "identity_rule_id".to_owned(),
                    Value::String(
                        insar_provider::catalog::EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID
                            .to_owned(),
                    ),
                ),
                (
                    "provider_product_id".to_owned(),
                    Value::String(manifest.item_id.clone()),
                ),
                (
                    "provider_product_key".to_owned(),
                    Value::String(product_key),
                ),
                (
                    "stac_item_id".to_owned(),
                    Value::String(manifest.item_id.clone()),
                ),
                (
                    "source_kind".to_owned(),
                    Value::String("sentinel2".to_owned()),
                ),
                (
                    "product_kind".to_owned(),
                    Value::String("optical_scene".to_owned()),
                ),
                ("assets".to_owned(), Value::Array(descriptive_assets)),
            ]),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        };
        reviewed_source
            .validate()
            .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let mut items = Vec::with_capacity(manifest.assets.len());
        for asset in &manifest.assets {
            if cancelled.load(Ordering::Acquire) {
                return Err(ApplicationServiceError::Sentinel2BandCancelled);
            }
            if sentinel2_l2a_reflectance_band_resolution(&asset.asset_key)
                != Some(asset.resolution_m)
            {
                return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
            }
            let filename =
                insar_provider::catalog::validate_earth_search_sentinel2_stable_band_url(
                    &manifest.item_id,
                    &asset.asset_key,
                    &asset.stable_href,
                )
                .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
            if filename != asset.file_name {
                return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
            }
            let frozen = evidence_port
                .probe_earth_search_band_asset(
                    &manifest.item_id,
                    &asset.asset_key,
                    &asset.stable_href,
                    &|| cancelled.load(Ordering::Acquire),
                )
                .map_err(ApplicationServiceError::Provider)?;
            if frozen.file_name != asset.file_name || frozen.expected_size_bytes == 0 {
                return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
            }
            items.push(
                build_earth_search_sentinel2_band_cog_item(
                    &reviewed_source,
                    &asset.asset_key,
                    &asset.stable_href,
                    &frozen,
                )
                .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?,
            );
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::Sentinel2BandCancelled);
        }
        return Ok((manifest.item_id, items));
    }
    let manifest = evidence_port
        .resolve_band_manifest(
            &source_identity.safe_name,
            Some(&source_identity.acquisition_datetime),
            asset_keys,
            &|| cancelled.load(Ordering::Acquire),
        )
        .map_err(ApplicationServiceError::Provider)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(ApplicationServiceError::Sentinel2BandCancelled);
    }
    if manifest.safe_name != source_identity.safe_name
        || manifest
            .assets
            .iter()
            .map(|asset| &asset.asset_key)
            .collect::<Vec<_>>()
            != asset_keys.iter().collect::<Vec<_>>()
    {
        return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
    }
    insar_provider::catalog::validate_mpc_sentinel2_source_id(
        &manifest.item_id,
        &manifest.safe_name,
        Some(&source_identity.acquisition_datetime),
    )
    .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
    let class = PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog;
    let product_key = provider_product_key(
        PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        class.collection_id(),
        &manifest.item_id,
    )
    .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
    let descriptive_assets = manifest
        .assets
        .iter()
        .map(|asset| {
            stable_asset_key(&product_key, &asset.asset_key).map(|stable_key| serde_json::json!({
            "asset_key": asset.asset_key, "stable_asset_key": stable_key,
            "role": "primary", "native_roles": ["data"], "resolution_m": asset.resolution_m,
        })).map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let reviewed_source = SnapshotItem {
        item_id: product_key.clone(),
        provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
        display_name: manifest.item_id.clone(),
        acquisition_datetime: Some(source_identity.acquisition_datetime.clone()),
        footprint_geojson: source_item.footprint_geojson.clone(),
        metadata: Map::from_iter([
            (
                "logical_dataset".to_owned(),
                Value::String(class.logical_dataset().to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String(class.collection_id().to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String(class.collection_profile_id().to_owned()),
            ),
            (
                "identity_rule_id".to_owned(),
                Value::String(MPC_REVIEWED_STAC_IDENTITY_RULE_ID.to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(manifest.item_id.clone()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(product_key),
            ),
            (
                "stac_item_id".to_owned(),
                Value::String(manifest.item_id.clone()),
            ),
            (
                "source_kind".to_owned(),
                Value::String("sentinel2".to_owned()),
            ),
            (
                "product_kind".to_owned(),
                Value::String("optical_scene".to_owned()),
            ),
            ("assets".to_owned(), Value::Array(descriptive_assets)),
        ]),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    reviewed_source
        .validate()
        .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
    let mut items = Vec::with_capacity(manifest.assets.len());
    for asset in &manifest.assets {
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::Sentinel2BandCancelled);
        }
        if sentinel2_l2a_reflectance_band_resolution(&asset.asset_key) != Some(asset.resolution_m) {
            return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
        }
        let filename = insar_provider::planetary_computer_asset::validate_planetary_computer_reviewed_asset_identity(
            class, &manifest.item_id, &asset.asset_key, &asset.stable_href,
        ).map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        let url = url::Url::parse(&asset.stable_href)
            .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        // The STAC Item ID omits the processing baseline. Bind the URL's full
        // SAFE directory too, before probing; otherwise another baseline can pass.
        if filename != asset.file_name
            || url.path_segments().and_then(|mut path| path.nth(7))
                != Some(manifest.safe_name.as_str())
        {
            return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
        }
        let frozen = evidence_port
            .probe_band_asset(
                &manifest.item_id,
                &asset.asset_key,
                &asset.stable_href,
                &|| cancelled.load(Ordering::Acquire),
            )
            .map_err(ApplicationServiceError::Provider)?;
        if frozen.file_name != asset.file_name || frozen.expected_size_bytes == 0 {
            return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
        }
        items.push(
            build_planetary_computer_reviewed_cog_item(
                &reviewed_source,
                class,
                &asset.asset_key,
                &asset.stable_href,
                &frozen,
            )
            .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?,
        );
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(ApplicationServiceError::Sentinel2BandCancelled);
    }
    Ok((manifest.item_id, items))
}

pub const SENTINEL2_L1C_BAND_ORDER: &[&str] = &[
    "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B10", "B11", "B12",
];

pub fn sentinel2_l1c_band_resolution(band: &str) -> Option<u16> {
    match band.trim().to_ascii_uppercase().as_str() {
        "B02" | "B03" | "B04" | "B08" => Some(10),
        "B05" | "B06" | "B07" | "B8A" | "B11" | "B12" => Some(20),
        "B01" | "B09" | "B10" => Some(60),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedSentinel2L1cBandAsset {
    pub asset_key: String,
    pub resolution_m: u16,
    pub file_name: String,
    pub stable_href: String,
    pub requires_cdse_auth: bool,
    pub expected_size_bytes: Option<u64>,
}

#[derive(Deserialize)]
struct GcsObjectsResponse {
    #[serde(default)]
    items: Vec<GcsObjectItem>,
}

#[derive(Deserialize)]
struct GcsObjectItem {
    name: String,
    #[serde(default)]
    size: Option<Value>,
}

#[derive(Deserialize)]
struct CdseNodesResponse {
    #[serde(default)]
    result: Vec<CdseNodeItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CdseNodeItem {
    name: String,
    #[serde(default)]
    content_length: Option<u64>,
}

pub(crate) fn build_optical_http_client(
    network_policy: ProviderNetworkPolicy,
    user_agent: &str,
    timeout: std::time::Duration,
) -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .referer(false)
        .user_agent(user_agent);
    match network_policy.route() {
        insar_core::ProxyPolicy::Direct => {
            builder = builder.no_proxy();
        }
        insar_core::ProxyPolicy::System => {}
        insar_core::ProxyPolicy::Manual(manual) => {
            let proxy = reqwest::Proxy::all(manual.url()).map_err(|_| {
                "OPTICAL_PROXY_INVALID: manual proxy configuration is invalid".to_owned()
            })?;
            builder = builder.proxy(proxy);
        }
    }
    builder
        .build()
        .map_err(|e| format!("OPTICAL_HTTP_INIT_FAILED: failed to initialize HTTP client: {e}"))
}

fn safe_jp2_basename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 160
        && !name.contains(['/', '\\', '\0', '?'])
        && !name.contains("..")
        && name.bytes().all(|b| b.is_ascii_graphic())
}

fn parse_s2_l1c_safe_and_tile(raw_name: &str) -> Option<(String, String, String, String)> {
    let trimmed = raw_name
        .trim()
        .strip_suffix(".zip")
        .unwrap_or(raw_name.trim());
    if trimmed.is_empty()
        || trimmed.contains(['/', '\\', '\0'])
        || trimmed.contains("..")
        || !trimmed.to_ascii_uppercase().contains("_MSIL1C_")
    {
        return None;
    }
    let safe_name = if trimmed.ends_with(".SAFE") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}.SAFE")
    };
    let tile = safe_name.split('_').find(|seg| {
        seg.len() == 6
            && seg.starts_with('T')
            && seg.as_bytes()[1..3].iter().all(u8::is_ascii_digit)
            && seg.as_bytes()[3..6].iter().all(u8::is_ascii_uppercase)
    })?;
    let utm = tile[1..3].to_owned();
    let lat = tile[3..4].to_owned();
    let sq = tile[4..6].to_owned();
    Some((safe_name, utm, lat, sq))
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<u64>().ok()))
}

fn match_jp2_for_band(file_name: &str, band: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.ends_with(&format!("_{}.jp2", band.to_ascii_lowercase()))
}

fn try_resolve_inline_l1c_assets(
    item: &SnapshotItem,
    asset_keys: &[String],
) -> Option<(String, Vec<ResolvedSentinel2L1cBandAsset>)> {
    let assets = item.metadata.get("assets")?.as_array()?;
    let product_dir = item
        .display_name
        .trim()
        .strip_suffix(".zip")
        .unwrap_or(item.display_name.trim())
        .strip_suffix(".SAFE")
        .unwrap_or(item.display_name.trim())
        .to_owned();
    let mut resolved = Vec::with_capacity(asset_keys.len());
    for key in asset_keys {
        let res_m = sentinel2_l1c_band_resolution(key)?;
        let obj = assets.iter().filter_map(Value::as_object).find(|a| {
            a.get("asset_key")
                .and_then(Value::as_str)
                .is_some_and(|k| k.eq_ignore_ascii_case(key))
        })?;
        let href = obj
            .get("url")
            .or_else(|| obj.get("href"))
            .and_then(Value::as_str)?
            .trim()
            .to_owned();
        if href.is_empty() {
            return None;
        }
        let file_name = obj
            .get("file_name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}_{}.jp2", product_dir, key));
        if !safe_jp2_basename(&file_name) {
            return None;
        }
        let requires_cdse_auth = href.starts_with("https://download.dataspace.copernicus.eu/");
        resolved.push(ResolvedSentinel2L1cBandAsset {
            asset_key: key.to_ascii_uppercase(),
            resolution_m: res_m,
            file_name,
            stable_href: href,
            requires_cdse_auth,
            expected_size_bytes: obj.get("size_bytes").and_then(json_u64),
        });
    }
    Some((product_dir, resolved))
}

fn fetch_json_bounded<T: serde::de::DeserializeOwned>(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<Option<T>, String> {
    use std::io::Read;
    let response = client
        .get(url)
        .send()
        .map_err(|_| "OPTICAL_L1C_DISCOVERY_NETWORK: request failed".to_owned())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!(
            "OPTICAL_L1C_DISCOVERY_HTTP: status {}",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("OPTICAL_L1C_DISCOVERY_READ: {e}"))?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("OPTICAL_L1C_DISCOVERY_TOO_LARGE".to_owned());
    }
    serde_json::from_slice::<T>(&bytes)
        .map(Some)
        .map_err(|e| format!("OPTICAL_L1C_DISCOVERY_JSON: {e}"))
}

pub(crate) fn resolve_sentinel2_l1c_band_assets(
    item: &SnapshotItem,
    asset_keys: &[String],
    network_policy: ProviderNetworkPolicy,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(String, Vec<ResolvedSentinel2L1cBandAsset>), ApplicationServiceError> {
    if cancelled() {
        return Err(ApplicationServiceError::Sentinel2BandCancelled);
    }
    let mut seen = BTreeSet::new();
    for key in asset_keys {
        if sentinel2_l1c_band_resolution(key).is_none() || !seen.insert(key.to_ascii_uppercase()) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: format!("OPTICAL_DERIVED_BAND_UNSUPPORTED: unsupported or duplicate Sentinel-2 L1C band '{key}'"),
            });
        }
    }
    if let Some(inline) = try_resolve_inline_l1c_assets(item, asset_keys) {
        return Ok(inline);
    }
    let raw_safe = item
        .metadata
        .get("safe_name")
        .and_then(Value::as_str)
        .unwrap_or(&item.display_name);
    let (safe_name, utm, lat, sq) = parse_s2_l1c_safe_and_tile(raw_safe).ok_or_else(|| {
        ApplicationServiceError::InvalidRequest {
            reason: format!(
                "OPTICAL_DERIVED_EVIDENCE_INVALID: invalid Sentinel-2 L1C SAFE name '{}'",
                item.display_name
            ),
        }
    })?;
    let directory = safe_name.trim_end_matches(".SAFE").to_owned();
    let client = build_optical_http_client(
        network_policy,
        "EOStage/Sentinel2L1cDiscovery",
        std::time::Duration::from_secs(25),
    )
    .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?;

    let prefix = format!("tiles/{utm}/{lat}/{sq}/{safe_name}/GRANULE/");
    let mut gcs_url =
        url::Url::parse("https://storage.googleapis.com/storage/v1/b/gcp-public-data-sentinel-2/o")
            .expect("constant URL");
    gcs_url
        .query_pairs_mut()
        .append_pair("prefix", &prefix)
        .append_pair("maxResults", "500");

    if cancelled() {
        return Err(ApplicationServiceError::Sentinel2BandCancelled);
    }
    let mut gcs_matches: Vec<(String, Option<u64>)> = Vec::new();
    match fetch_json_bounded::<GcsObjectsResponse>(&client, gcs_url.as_str()) {
        Ok(Some(listing)) => {
            for obj in listing.items {
                if obj.name.contains("/IMG_DATA/") {
                    if let Some(file_name) = obj.name.rsplit('/').next() {
                        if safe_jp2_basename(file_name) {
                            gcs_matches
                                .push((obj.name.clone(), obj.size.as_ref().and_then(json_u64)));
                        }
                    }
                }
            }
        }
        Ok(None) => {}
        Err(_) => {
            // GCS public mirror can be unreachable in mainland environments;
            // log secret-free diagnostic and fail over to CDSE OData below.
            eprintln!("[sentinel2_band_delivery] GCS listing unavailable, falling back to CDSE");
        }
    }

    let mut out = Vec::with_capacity(asset_keys.len());
    let mut missing = Vec::new();
    for key in asset_keys {
        let normalized = key.to_ascii_uppercase();
        let res_m = sentinel2_l1c_band_resolution(&normalized).expect("validated above");
        if let Some((object_name, size)) = gcs_matches.iter().find(|(obj_name, _)| {
            obj_name
                .rsplit('/')
                .next()
                .is_some_and(|f| match_jp2_for_band(f, &normalized))
        }) {
            let file_name = object_name.rsplit('/').next().expect("checked").to_owned();
            let stable_href =
                format!("https://storage.googleapis.com/gcp-public-data-sentinel-2/{object_name}");
            out.push(ResolvedSentinel2L1cBandAsset {
                asset_key: normalized,
                resolution_m: res_m,
                file_name,
                stable_href,
                requires_cdse_auth: false,
                expected_size_bytes: *size,
            });
        } else {
            missing.push(normalized);
        }
    }

    if !missing.is_empty() {
        let product_uuid = item
            .metadata
            .get("provider_product_id")
            .and_then(Value::as_str)
            .filter(|v| canonical_odata_uuid(v))
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason: "OPTICAL_DERIVED_EVIDENCE_INVALID: Sentinel-2 L1C scene unavailable on GCS mirror and lacks CDSE OData UUID".to_owned(),
            })?;
        if cancelled() {
            return Err(ApplicationServiceError::Sentinel2BandCancelled);
        }
        let granules_url = format!(
            "https://catalogue.dataspace.copernicus.eu/odata/v1/Products({product_uuid})/Nodes({safe_name})/Nodes(GRANULE)/Nodes"
        );
        let granules = fetch_json_bounded::<CdseNodesResponse>(&client, &granules_url)
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason: "OPTICAL_DERIVED_BAND_MISSING: Sentinel-2 L1C granule directory not found on CDSE".to_owned(),
            })?;
        let granule_name = granules
            .result
            .into_iter()
            .map(|n| n.name)
            .find(|name| !name.is_empty() && !name.contains(['/', '\\', '.']))
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason: "OPTICAL_DERIVED_BAND_MISSING: Sentinel-2 L1C granule name missing in CDSE response".to_owned(),
            })?;
        if cancelled() {
            return Err(ApplicationServiceError::Sentinel2BandCancelled);
        }
        let img_data_url = format!(
            "https://catalogue.dataspace.copernicus.eu/odata/v1/Products({product_uuid})/Nodes({safe_name})/Nodes(GRANULE)/Nodes({granule_name})/Nodes(IMG_DATA)/Nodes"
        );
        let files = fetch_json_bounded::<CdseNodesResponse>(&client, &img_data_url)
            .map_err(|reason| ApplicationServiceError::InvalidRequest { reason })?
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason:
                    "OPTICAL_DERIVED_BAND_MISSING: Sentinel-2 L1C IMG_DATA nodes missing on CDSE"
                        .to_owned(),
            })?;

        out.clear();
        for key in asset_keys {
            let normalized = key.to_ascii_uppercase();
            let res_m = sentinel2_l1c_band_resolution(&normalized).expect("validated above");
            if let Some((object_name, size)) = gcs_matches.iter().find(|(obj_name, _)| {
                obj_name
                    .rsplit('/')
                    .next()
                    .is_some_and(|f| match_jp2_for_band(f, &normalized))
            }) {
                let file_name = object_name.rsplit('/').next().expect("checked").to_owned();
                out.push(ResolvedSentinel2L1cBandAsset {
                    asset_key: normalized,
                    resolution_m: res_m,
                    file_name,
                    stable_href: format!(
                        "https://storage.googleapis.com/gcp-public-data-sentinel-2/{object_name}"
                    ),
                    requires_cdse_auth: false,
                    expected_size_bytes: *size,
                });
                continue;
            }
            let node = files
                .result
                .iter()
                .find(|n| safe_jp2_basename(&n.name) && match_jp2_for_band(&n.name, &normalized))
                .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                    reason: format!("OPTICAL_DERIVED_BAND_MISSING: Sentinel-2 L1C band {normalized} not found in SAFE IMG_DATA"),
                })?;
            let stable_href = format!(
                "https://download.dataspace.copernicus.eu/odata/v1/Products({product_uuid})/Nodes({safe_name})/Nodes(GRANULE)/Nodes({granule_name})/Nodes(IMG_DATA)/Nodes({})/$value",
                node.name
            );
            out.push(ResolvedSentinel2L1cBandAsset {
                asset_key: normalized,
                resolution_m: res_m,
                file_name: node.name.clone(),
                stable_href,
                requires_cdse_auth: true,
                expected_size_bytes: node.content_length,
            });
        }
    }

    Ok((directory, out))
}

pub(crate) fn prepare_sentinel2_l1c_band_items(
    source_item: &SnapshotItem,
    asset_keys: &[String],
    network_policy: ProviderNetworkPolicy,
    cancelled: &AtomicBool,
) -> Result<(String, Vec<SnapshotItem>), ApplicationServiceError> {
    if asset_keys.is_empty() || asset_keys.len() > SENTINEL2_L1C_BAND_ORDER.len() {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "Sentinel-2 L1C band selection is invalid".to_owned(),
        });
    }
    let (directory, assets) =
        resolve_sentinel2_l1c_band_assets(source_item, asset_keys, network_policy, &|| {
            cancelled.load(Ordering::Acquire)
        })?;
    if !valid_catalogue_display_name(&directory) || directory.len() > 128 {
        return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
    }
    let mut items = Vec::with_capacity(assets.len());
    for asset in assets {
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::Sentinel2BandCancelled);
        }
        let item_id = format!(
            "{}-{}",
            source_item.item_id,
            asset.asset_key.to_ascii_lowercase()
        );
        if !valid_catalogue_opaque_id(&item_id) || !valid_catalogue_display_name(&asset.file_name) {
            return Err(ApplicationServiceError::Sentinel2BandEvidenceInvalid);
        }
        let mut metadata = source_item.metadata.clone();
        metadata.insert(
            "source_item_id".to_owned(),
            Value::String(source_item.item_id.clone()),
        );
        metadata.insert(
            "delivery_asset_key".to_owned(),
            Value::String(asset.asset_key.clone()),
        );
        metadata.insert(
            "delivery_asset_role".to_owned(),
            Value::String("primary".to_owned()),
        );
        metadata.insert(
            "delivery_product_directory".to_owned(),
            Value::String(directory.clone()),
        );
        metadata.insert("resolution_m".to_owned(), Value::from(asset.resolution_m));
        let spec = serde_json::json!({
            "source_url": asset.stable_href,
            "file_name": asset.file_name,
            "directory": directory,
            "band_key": asset.asset_key,
            "resolution_m": asset.resolution_m,
            "requires_cdse_auth": asset.requires_cdse_auth,
            "expected_size_bytes": asset.expected_size_bytes,
        });
        let mut payload = Map::new();
        payload.insert("sentinel2_l1c_band_spec".to_owned(), spec);
        let item = SnapshotItem {
            item_id,
            provider_id: source_item.provider_id.clone(),
            display_name: asset.file_name,
            acquisition_datetime: source_item.acquisition_datetime.clone(),
            footprint_geojson: source_item.footprint_geojson.clone(),
            metadata,
            payload,
            asset_access: None,
            execution_policy: None,
        };
        item.validate()
            .map_err(|_| ApplicationServiceError::Sentinel2BandEvidenceInvalid)?;
        items.push(item);
    }
    Ok((directory, items))
}

pub(crate) fn download_http_asset_to_path_with_cdse_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    part_path: &Path,
    cdse_session: Option<&Arc<insar_download::CdseOAuthSession>>,
    requires_cdse_auth: bool,
    interrupted: &(dyn Fn() -> bool + Sync),
    on_bytes: &mut dyn FnMut(u64),
) -> Result<u64, (String, String, bool)> {
    use std::io::{Read, Write};

    if let Some(parent) = part_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            (
                "S2_L1C_OUTPUT_UNAVAILABLE".to_owned(),
                format!("无法创建输出目录: {e}"),
                false,
            )
        })?;
    }

    let mut refreshed_after_401 = false;
    loop {
        if interrupted() {
            return Err((
                "S2_L1C_INTERRUPTED".to_owned(),
                "下载已暂停或取消".to_owned(),
                true,
            ));
        }
        let mut token_gen = None;
        let mut current_url = url.to_owned();
        let mut send_auth = requires_cdse_auth;
        let mut resp = None;
        for _redirect_hop in 0..=3 {
            let mut req = client.get(&current_url);
            if send_auth {
                let session = cdse_session.ok_or_else(|| {
                    (
                        "CDSE_CREDENTIALS_MISSING".to_owned(),
                        "该最新 L1C 影像尚未同步至公开镜像，需要配置并登录 CDSE 账号以下载波段"
                            .to_owned(),
                        false,
                    )
                })?;
                let (gen, bearer) = session.bearer_token_for_download().map_err(|err| {
                    (
                        err.code().to_owned(),
                        format!("CDSE 认证不可用（{}），请检查账号连接", err.code()),
                        false,
                    )
                })?;
                token_gen = Some(gen);
                req = req.bearer_auth(bearer.as_str());
            }
            let r = req.send().map_err(|_| {
                (
                    "S2_L1C_NETWORK_FAILED".to_owned(),
                    "波段下载网络请求失败，请检查网络连接或代理设置".to_owned(),
                    true,
                )
            })?;
            if r.status().is_redirection() {
                if let Some(loc) = r
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                {
                    let parsed_loc = url::Url::parse(loc)
                        .or_else(|_| url::Url::parse(&current_url).and_then(|base| base.join(loc)))
                        .map_err(|_| {
                            (
                                "S2_L1C_REDIRECT_INVALID".to_owned(),
                                "下载重定向地址无效".to_owned(),
                                false,
                            )
                        })?;
                    if parsed_loc.scheme() != "https" && !cfg!(test) {
                        return Err((
                            "S2_L1C_REDIRECT_INSECURE".to_owned(),
                            "拒绝非 HTTPS 的下载重定向".to_owned(),
                            false,
                        ));
                    }
                    send_auth = parsed_loc.host_str() == Some("download.dataspace.copernicus.eu");
                    current_url = parsed_loc.into();
                    continue;
                }
            }
            resp = Some(r);
            break;
        }
        let mut resp = resp.ok_or_else(|| {
            (
                "S2_L1C_REDIRECT_LIMIT".to_owned(),
                "下载重定向次数过多".to_owned(),
                true,
            )
        })?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && requires_cdse_auth
            && !refreshed_after_401
        {
            refreshed_after_401 = true;
            if let (Some(session), Some(gen)) = (cdse_session, token_gen) {
                match session.refresh_after_rejection(gen) {
                    Ok(_) => continue,
                    Err(err) => {
                        let retryable = matches!(
                            err,
                            insar_download::CdseAuthError::Transport
                                | insar_download::CdseAuthError::Timeout
                        );
                        return Err((
                            err.code().to_owned(),
                            format!("CDSE 认证令牌刷新失败：{err}"),
                            retryable,
                        ));
                    }
                }
            }
        }
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let retryable = status == 429 || status >= 500;
            return Err((
                format!("S2_L1C_HTTP_{status}"),
                format!("远端返回 HTTP {status}"),
                retryable,
            ));
        }

        let mut file = std::fs::File::create(part_path).map_err(|e| {
            (
                "S2_L1C_OUTPUT_UNAVAILABLE".to_owned(),
                format!("无法创建临时下载文件: {e}"),
                false,
            )
        })?;
        let mut written = 0u64;
        let mut buf = [0u8; 64 * 1024];
        loop {
            if interrupted() {
                drop(file);
                let _ = std::fs::remove_file(part_path);
                return Err((
                    "S2_L1C_INTERRUPTED".to_owned(),
                    "下载已暂停或取消".to_owned(),
                    true,
                ));
            }
            let n = match resp.read(&mut buf) {
                Ok(n) => n,
                Err(_) => {
                    drop(file);
                    let _ = std::fs::remove_file(part_path);
                    return Err((
                        "S2_L1C_READ_FAILED".to_owned(),
                        "读取波段数据流失败，请检查网络连接".to_owned(),
                        true,
                    ));
                }
            };
            if n == 0 {
                break;
            }
            if let Err(e) = file.write_all(&buf[..n]) {
                drop(file);
                let _ = std::fs::remove_file(part_path);
                return Err((
                    "S2_L1C_WRITE_FAILED".to_owned(),
                    format!("写入磁盘失败: {e}"),
                    false,
                ));
            }
            written = written.saturating_add(n as u64);
            on_bytes(n as u64);
        }
        if let Err(e) = file.flush() {
            drop(file);
            let _ = std::fs::remove_file(part_path);
            return Err((
                "S2_L1C_WRITE_FAILED".to_owned(),
                format!("刷新文件缓冲失败: {e}"),
                false,
            ));
        }
        return Ok(written);
    }
}
