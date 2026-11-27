use insar_core::{
    provider_product_key, stable_asset_key, AsfRawFeature, AssetRole, BBox, ContractError,
    ProviderAsset, ProviderResult, SarProduct,
};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

use super::access::earthdata_https_descriptor;
use super::collection_profile::{profile_for_dataset, AsfCollectionProfile, AsfItemModel};

fn text(properties: &Map<String, Value>, aliases: &[&str]) -> Option<String> {
    aliases.iter().find_map(|alias| {
        properties.get(*alias).and_then(|value| match value {
            Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn text_with_field(
    properties: &Map<String, Value>,
    aliases: &[&'static str],
) -> Option<(&'static str, String)> {
    aliases
        .iter()
        .find_map(|alias| text(properties, &[*alias]).map(|value| (*alias, value)))
}

fn text_or_first_array_with_field(
    properties: &Map<String, Value>,
    aliases: &[&'static str],
) -> Option<(&'static str, String)> {
    aliases.iter().find_map(|alias| {
        let value = properties.get(*alias)?;
        let text = match value {
            Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
            Value::Array(values) => values.iter().find_map(|item| {
                item.as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            }),
            _ => None,
        }?;
        Some((*alias, text))
    })
}

fn integer(properties: &Map<String, Value>, aliases: &[&str]) -> Option<i64> {
    aliases.iter().find_map(|alias| {
        properties.get(*alias).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|item| i64::try_from(item).ok()))
                .or_else(|| value.as_str().and_then(|item| item.trim().parse().ok()))
        })
    })
}

fn unsigned(properties: &Map<String, Value>, aliases: &[&str]) -> Option<u64> {
    aliases
        .iter()
        .find_map(|alias| properties.get(*alias).and_then(unsigned_value))
}

fn unsigned_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|item| item.trim().parse().ok()))
}

fn selected_file_size(properties: &Map<String, Value>, file_name: Option<&str>) -> Option<u64> {
    unsigned(properties, &["bytes", "fileSize", "size"]).or_else(|| {
        let file_name = file_name?;
        properties
            .get("bytes")?
            .as_object()?
            .get(file_name)?
            .as_object()?
            .get("bytes")
            .and_then(unsigned_value)
    })
}

fn selected_file_checksum(
    properties: &Map<String, Value>,
    file_name: Option<&str>,
) -> Option<String> {
    text(properties, &["checksum", "md5", "sha256"]).or_else(|| {
        let file_name = file_name?;
        properties
            .get("md5sum")?
            .as_object()?
            .get(file_name)?
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn product_family(processing_level: &str) -> String {
    let level = processing_level.trim().to_ascii_uppercase();
    if level == "SLC" || matches!(level.as_str(), "L1" | "L1.1") {
        "SLC"
    } else if level.starts_with("GRD") || level == "L1.5" {
        "GRD"
    } else if level == "RAW" || matches!(level.as_str(), "L0" | "L1.0") {
        "RAW"
    } else if level.starts_with("RTC") {
        "RTC"
    } else if level == "OCN" {
        "OCN"
    } else if level == "BURST" {
        "BURST"
    } else {
        "OTHER"
    }
    .to_string()
}

fn collect_positions(value: &Value, positions: &mut Vec<(f64, f64)>) {
    match value {
        Value::Array(items)
            if items.len() >= 2 && items[0].as_f64().is_some() && items[1].as_f64().is_some() =>
        {
            positions.push((items[0].as_f64().unwrap(), items[1].as_f64().unwrap()));
        }
        Value::Array(items) => {
            for item in items {
                collect_positions(item, positions);
            }
        }
        Value::Object(object) => {
            if let Some(coordinates) = object.get("coordinates") {
                collect_positions(coordinates, positions);
            }
            if let Some(geometries) = object.get("geometries") {
                collect_positions(geometries, positions);
            }
        }
        _ => {}
    }
}

fn geometry_bbox(geometry: &Map<String, Value>) -> Option<BBox> {
    let mut positions = Vec::new();
    collect_positions(&Value::Object(geometry.clone()), &mut positions);
    let west = positions.iter().map(|value| value.0).reduce(f64::min)?;
    let east = positions.iter().map(|value| value.0).reduce(f64::max)?;
    let south = positions.iter().map(|value| value.1).reduce(f64::min)?;
    let north = positions.iter().map(|value| value.1).reduce(f64::max)?;
    let bbox = BBox {
        west,
        east,
        south,
        north,
        crs: "EPSG:4326".to_string(),
    };
    bbox.validate().ok().map(|()| bbox)
}

fn url_filename(url: &str) -> Option<String> {
    let path = url.split('#').next()?.split('?').next()?;
    path.rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn mapper_error(code: &str, user_message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        user_message,
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    )
    .expect("bounded ASF mapper context is safe")
}

fn asset_role(properties: &Map<String, Value>, file_name: Option<&str>) -> AssetRole {
    match text(properties, &["assetRole", "role"])
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "metadata" => AssetRole::Metadata,
        "thumbnail" => AssetRole::Thumbnail,
        "browse" => AssetRole::Browse,
        "auxiliary" => AssetRole::Auxiliary,
        "derived" => AssetRole::Derived,
        "primary" => AssetRole::Primary,
        _ => match file_name
            .and_then(|name| name.rsplit('.').next())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "xml" | "json" | "txt" => AssetRole::Metadata,
            "jpg" | "jpeg" | "png" => AssetRole::Browse,
            _ => AssetRole::Primary,
        },
    }
}

fn media_type(file_name: Option<&str>) -> Option<String> {
    let extension = file_name?.rsplit('.').next()?.to_ascii_lowercase();
    Some(
        match extension.as_str() {
            "zip" => "application/zip",
            "xml" => "application/xml",
            "json" => "application/json",
            "tif" | "tiff" => "image/tiff",
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "h5" | "hdf5" => "application/x-hdf5",
            _ => "application/octet-stream",
        }
        .to_string(),
    )
}

fn map_asset(
    profile: &AsfCollectionProfile,
    provider_key: &str,
    provider_product_id: &str,
    properties: &Map<String, Value>,
) -> ProviderResult<Option<ProviderAsset>> {
    let download_url = text(properties, &["url", "downloadUrl", "download_url"]);
    let file_name = text(properties, &["fileName", "filename"])
        .or_else(|| download_url.as_deref().and_then(url_filename));
    let asset_id = text(properties, profile.asset_id_fields).or_else(|| {
        (profile.item_model == AsfItemModel::OneFilePerProduct
            && (download_url.is_some() || file_name.is_some()))
        .then(|| provider_product_id.to_string())
    });
    let Some(asset_key) = asset_id else {
        return Ok(None);
    };
    let size_bytes = selected_file_size(properties, file_name.as_deref())
        .and_then(|value| i64::try_from(value).ok());
    let stable_key = stable_asset_key(provider_key, &asset_key).map_err(|reason| {
        mapper_error(
            "ASF_ASSET_IDENTITY_INVALID",
            "ASF 资产身份无效，资产未加入结果",
            &reason,
        )
    })?;
    let access_descriptor = download_url.as_deref().and_then(earthdata_https_descriptor);
    Ok(Some(ProviderAsset {
        asset_key: Some(asset_key),
        stable_asset_key: Some(stable_key),
        access_descriptor,
        role: asset_role(properties, file_name.as_deref()),
        native_roles: Vec::new(),
        url: download_url,
        media_type: media_type(file_name.as_deref()),
        file_name,
        size_bytes,
    }))
}

fn map_preview_asset(
    provider_key: &str,
    provider_product_id: &str,
    properties: &Map<String, Value>,
) -> ProviderResult<Option<ProviderAsset>> {
    let Some((field, url)) = text_or_first_array_with_field(
        properties,
        &[
            "thumbnail",
            "thumbnailUrl",
            "browse",
            "browseUrl",
            "browse_url",
        ],
    ) else {
        return Ok(None);
    };
    let file_name = url_filename(&url);
    let role = if field.to_ascii_lowercase().contains("thumbnail") {
        AssetRole::Thumbnail
    } else {
        AssetRole::Browse
    };
    let role_key = if role == AssetRole::Thumbnail {
        "thumbnail"
    } else {
        "browse"
    };
    let asset_key = format!("{provider_product_id}:{role_key}");
    let stable_key = stable_asset_key(provider_key, &asset_key).map_err(|reason| {
        mapper_error(
            "ASF_ASSET_IDENTITY_INVALID",
            "ASF 预览资产身份无效，资产未加入结果",
            &reason,
        )
    })?;
    Ok(Some(ProviderAsset {
        asset_key: Some(asset_key),
        stable_asset_key: Some(stable_key),
        access_descriptor: None,
        role,
        native_roles: vec![field.to_owned()],
        url: Some(url),
        media_type: media_type(file_name.as_deref()),
        file_name,
        size_bytes: None,
    }))
}

struct SeasatAssetProjection {
    assets: Vec<ProviderAsset>,
    download_url: String,
    file_name: String,
    file_size_remote: Option<u64>,
    checksum: Option<String>,
}

fn string_array(properties: &Map<String, Value>, aliases: &[&str]) -> Vec<String> {
    aliases
        .iter()
        .flat_map(|alias| {
            properties
                .get(*alias)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn seasat_file_fact<'a>(
    properties: &'a Map<String, Value>,
    file_name: &str,
) -> Option<&'a Map<String, Value>> {
    properties
        .get("bytes")?
        .as_object()?
        .get(file_name)?
        .as_object()
}

fn seasat_asset_role(file_name: &str, format: Option<&str>, is_browse_url: bool) -> AssetRole {
    let lower = file_name.to_ascii_lowercase();
    let normalized_format = format.unwrap_or_default().trim().to_ascii_lowercase();
    if (lower.ends_with(".h5") || lower.ends_with(".hdf5")) && normalized_format == "hdf5" {
        AssetRole::Primary
    } else if (lower.ends_with(".tif") || lower.ends_with(".tiff"))
        && normalized_format == "geotiff"
    {
        AssetRole::Derived
    } else if is_browse_url && (lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
        if lower.contains("thumbnail") {
            AssetRole::Thumbnail
        } else {
            AssetRole::Browse
        }
    } else if lower.ends_with(".xml") || lower.ends_with(".kml") {
        AssetRole::Metadata
    } else {
        AssetRole::Auxiliary
    }
}

fn asset_role_rank(role: &AssetRole) -> u8 {
    match role {
        AssetRole::Primary => 0,
        AssetRole::Derived => 1,
        AssetRole::Preview => 2,
        AssetRole::Browse => 3,
        AssetRole::Thumbnail => 4,
        AssetRole::Metadata => 5,
        AssetRole::Auxiliary => 6,
    }
}

fn seasat_media_type(file_name: &str, format: Option<&str>) -> Option<String> {
    let normalized_format = format.unwrap_or_default().trim().to_ascii_lowercase();
    if normalized_format == "text file" {
        Some("text/plain".to_string())
    } else if normalized_format == "kml" {
        Some("application/vnd.google-earth.kml+xml".to_string())
    } else {
        media_type(Some(file_name))
    }
}

fn map_seasat_assets(
    provider_key: &str,
    provider_product_id: &str,
    properties: &Map<String, Value>,
) -> ProviderResult<SeasatAssetProjection> {
    let browse_urls: BTreeSet<String> = string_array(
        properties,
        &[
            "browse",
            "browseUrl",
            "browse_url",
            "thumbnail",
            "thumbnailUrl",
        ],
    )
    .into_iter()
    .collect();
    let mut urls = BTreeSet::new();
    if let Some(url) = text(properties, &["url", "downloadUrl", "download_url"])
        .filter(|url| url.starts_with("https://"))
    {
        urls.insert(url);
    }
    urls.extend(
        string_array(properties, &["additionalUrls", "additional_urls"])
            .into_iter()
            .filter(|url| url.starts_with("https://")),
    );

    let mut assets = Vec::new();
    for url in urls {
        let Some(file_name) = url_filename(&url) else {
            continue;
        };
        let file_fact = seasat_file_fact(properties, &file_name);
        let format = file_fact
            .and_then(|value| value.get("format"))
            .and_then(Value::as_str);
        let role = seasat_asset_role(&file_name, format, browse_urls.contains(&url));
        let size_bytes = file_fact
            .and_then(|value| value.get("bytes"))
            .and_then(unsigned_value)
            .and_then(|value| i64::try_from(value).ok());
        let asset_key = format!("{provider_product_id}:{file_name}");
        let stable_key = stable_asset_key(provider_key, &asset_key).map_err(|reason| {
            mapper_error(
                "ASF_ASSET_IDENTITY_INVALID",
                "ASF 资产身份无效，资产未加入结果",
                &reason,
            )
        })?;
        assets.push(ProviderAsset {
            asset_key: Some(asset_key),
            stable_asset_key: Some(stable_key),
            access_descriptor: earthdata_https_descriptor(&url),
            role,
            native_roles: Vec::new(),
            url: Some(url),
            media_type: seasat_media_type(&file_name, format),
            file_name: Some(file_name),
            size_bytes,
        });
    }
    assets.sort_by(|left, right| {
        asset_role_rank(&left.role)
            .cmp(&asset_role_rank(&right.role))
            .then_with(|| left.file_name.cmp(&right.file_name))
            .then_with(|| left.url.cmp(&right.url))
    });

    let primary_assets: Vec<&ProviderAsset> = assets
        .iter()
        .filter(|asset| asset.role == AssetRole::Primary)
        .collect();
    if primary_assets.is_empty() {
        return Err(mapper_error(
            "ASF_PRIMARY_ASSET_MISSING",
            "SEASAT 结果缺少唯一 HDF5 主数据资产",
            "missing_hdf5_primary",
        ));
    }
    if primary_assets.len() > 1 {
        return Err(mapper_error(
            "ASF_PRIMARY_ASSET_AMBIGUOUS",
            "SEASAT 结果包含多个 HDF5 主数据资产",
            "ambiguous_hdf5_primary",
        ));
    }
    let primary = primary_assets[0];
    let file_name = primary
        .file_name
        .clone()
        .expect("SEASAT primary is classified from a URL filename");
    let download_url = primary
        .url
        .clone()
        .expect("SEASAT primary is classified from an HTTPS URL");
    let file_size_remote = primary
        .size_bytes
        .and_then(|value| u64::try_from(value).ok());
    let checksum = properties
        .get("md5sum")
        .and_then(Value::as_object)
        .and_then(|checksums| checksums.get(&file_name))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok(SeasatAssetProjection {
        assets,
        download_url,
        file_name,
        file_size_remote,
        checksum,
    })
}

/// Map one structured ASF GeoJSON record through its versioned Collection profile.
pub fn map_sar_feature(dataset: &str, feature: &AsfRawFeature) -> ProviderResult<SarProduct> {
    let properties = &feature.properties;
    let profile = profile_for_dataset(dataset).ok_or_else(|| {
        mapper_error(
            "ASF_COLLECTION_UNSUPPORTED",
            "ASF 数据集缺少已审核的身份规则",
            "unknown_collection",
        )
    })?;
    let (identity_field, provider_product_id) = text_with_field(properties, profile.item_id_fields)
        .ok_or_else(|| {
            mapper_error(
                "ASF_IDENTITY_MISSING",
                "ASF 结果缺少可靠的产品身份",
                "missing_profile_approved_item_id",
            )
        })?;
    if identity_field == "sceneName" && !profile.allow_scene_name_identity {
        return Err(mapper_error(
            "ASF_IDENTITY_MISSING",
            "ASF 结果中的场景名称仅可用于显示",
            "scene_name_not_approved_for_identity",
        ));
    }
    let display_alias = text(properties, profile.display_alias_fields)
        .unwrap_or_else(|| provider_product_id.clone());
    let processing_level = text(properties, &["processingLevel", "processingType", "level"]);
    let mut download_url = text(properties, &["url", "downloadUrl", "download_url"]);
    let mut file_name = text(properties, &["fileName", "filename"])
        .or_else(|| download_url.as_deref().and_then(url_filename));
    let mut file_size_remote = selected_file_size(properties, file_name.as_deref());
    let mut checksum = selected_file_checksum(properties, file_name.as_deref());
    let native_metadata = Map::from_iter([
        ("type".to_string(), Value::String(feature.r#type.clone())),
        ("properties".to_string(), Value::Object(properties.clone())),
    ]);
    let product_key = provider_product_key("asf.catalog", profile.dataset, &provider_product_id)
        .map_err(|reason| {
            mapper_error(
                "ASF_IDENTITY_INVALID",
                "ASF 产品身份无效，产品未加入结果",
                &reason,
            )
        })?;
    let assets = if profile.dataset == "SEASAT" {
        let projection = map_seasat_assets(&product_key, &provider_product_id, properties)?;
        download_url = Some(projection.download_url);
        file_name = Some(projection.file_name);
        file_size_remote = projection.file_size_remote;
        checksum = projection.checksum;
        projection.assets
    } else {
        map_asset(profile, &product_key, &provider_product_id, properties)?
            .into_iter()
            .chain(map_preview_asset(
                &product_key,
                &provider_product_id,
                properties,
            )?)
            .collect()
    };
    // Geometry is preserved losslessly in `footprint_geojson`; duplicating it
    // inside native metadata can double the dominant payload for large results.
    let product = SarProduct {
        product_key: product_key.clone(),
        provider_product_key: Some(product_key),
        provider_product_id: Some(provider_product_id.clone()),
        canonical_product_key: None,
        identity_rule_id: Some(profile.rule_id.to_string()),
        product_id: provider_product_id,
        scene_id: Some(display_alias),
        provider: "asf.catalog".to_string(),
        dataset: profile.dataset.to_string(),
        platform: text(properties, &["platform", "platformName", "satellite"]),
        instrument: text(properties, &["instrument", "sensor"]),
        processing_level: processing_level.clone(),
        product_family: product_family(processing_level.as_deref().unwrap_or("")),
        beam_mode: text(properties, &["beamModeType", "beamMode"]),
        beam_swath: text(properties, &["beamSwath", "subswath"]),
        polarization: text(properties, &["polarization"]),
        flight_direction: text(properties, &["flightDirection", "orbitDirection"]),
        look_direction: text(properties, &["lookDirection"]),
        relative_orbit: integer(properties, &["relativeOrbit", "relativeOrbitNumber"]),
        absolute_orbit: integer(
            properties,
            &["absoluteOrbit", "absoluteOrbitNumber", "orbit"],
        ),
        frame: integer(properties, &["frameNumber", "frame"]),
        asf_frame: text(properties, &["asfFrame", "asfframe"]),
        path: integer(properties, &["pathNumber", "path"]),
        acquisition_start: text(properties, &["startTime", "acquisitionStart", "start"]),
        acquisition_stop: text(properties, &["stopTime", "acquisitionStop", "end"]),
        footprint_geojson: feature.geometry.clone(),
        footprint_bbox: feature.geometry.as_ref().and_then(geometry_bbox),
        download_url,
        file_name,
        file_size_remote,
        checksum,
        local_path: None,
        native_metadata,
        burst: None,
        assets,
    };
    product.validate().map_err(|reason| {
        mapper_error(
            "ASF_METADATA_INVALID",
            "ASF 产品元数据无效，产品未加入结果",
            &reason,
        )
    })?;
    Ok(product)
}
