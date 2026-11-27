use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map};

use crate::{ContractError, OrbitProductType, ProviderResult, SarProduct};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopProductLayout {
    Flat,
    ProductSubdirectory,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopProductFamily {
    Sentinel1,
    Orbit,
}

fn family_name(family: DesktopProductFamily) -> &'static str {
    match family {
        DesktopProductFamily::Sentinel1 => "sentinel1",
        DesktopProductFamily::Orbit => "orbit",
    }
}

fn layout_name(layout: DesktopProductLayout) -> &'static str {
    match layout {
        DesktopProductLayout::Flat => "flat",
        DesktopProductLayout::ProductSubdirectory => "product_subdirectory",
    }
}

fn invalid_target_error(
    family: DesktopProductFamily,
    layout: DesktopProductLayout,
) -> ContractError {
    let context = Map::from_iter([
        ("family".to_string(), json!(family_name(family))),
        ("layout".to_string(), json!(layout_name(layout))),
    ]);
    ContractError::new("V3_INVALID_TARGET_PATH", "下载目标路径无效", context)
        .expect("the static target-path error context is safe")
}

fn is_windows_reserved_name(file_name: &str) -> bool {
    let stem = file_name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem.strip_prefix("COM").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || stem.strip_prefix("LPT").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
}

fn is_safe_file_name(file_name: &str) -> bool {
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.ends_with(['.', ' '])
        || file_name
            .chars()
            .any(|character| character.is_control() || r#"<>:"/\|?*"#.contains(character))
        || is_windows_reserved_name(file_name)
    {
        return false;
    }
    let mut components = Path::new(file_name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

pub fn plan_desktop_target(
    selected_root: &Path,
    file_name: &str,
    family: DesktopProductFamily,
    layout: DesktopProductLayout,
) -> ProviderResult<PathBuf> {
    if !is_safe_file_name(file_name) {
        return Err(invalid_target_error(family, layout));
    }
    let base = match (family, layout) {
        (DesktopProductFamily::Sentinel1, DesktopProductLayout::Flat)
        | (DesktopProductFamily::Orbit, DesktopProductLayout::Flat) => selected_root.to_path_buf(),
        (DesktopProductFamily::Sentinel1, DesktopProductLayout::ProductSubdirectory) => {
            selected_root.join("SLC")
        }
        (DesktopProductFamily::Orbit, DesktopProductLayout::ProductSubdirectory) => {
            let orbit_subdir = if file_name.contains("_AUX_RESORB_") {
                "AUX_RESORB"
            } else {
                "AUX_POEORB"
            };
            selected_root.join("Sentinel_Orbit").join(orbit_subdir)
        }
    };
    Ok(base.join(file_name))
}

pub fn plan_desktop_orbit_target(
    selected_root: &Path,
    file_name: &str,
    orbit_type: OrbitProductType,
    layout: DesktopProductLayout,
) -> ProviderResult<PathBuf> {
    if !is_safe_file_name(file_name) {
        return Err(invalid_target_error(DesktopProductFamily::Orbit, layout));
    }
    let base = match (orbit_type, layout) {
        (_, DesktopProductLayout::Flat) => selected_root.to_path_buf(),
        (OrbitProductType::POEORB, DesktopProductLayout::ProductSubdirectory) => {
            selected_root.join("Sentinel_Orbit").join("AUX_POEORB")
        }
        (OrbitProductType::RESORB, DesktopProductLayout::ProductSubdirectory) => {
            selected_root.join("Sentinel_Orbit").join("AUX_RESORB")
        }
        (
            OrbitProductType::MOEORB | OrbitProductType::UNKNOWN,
            DesktopProductLayout::ProductSubdirectory,
        ) => {
            return Err(invalid_target_error(DesktopProductFamily::Orbit, layout));
        }
    };
    Ok(base.join(file_name))
}

fn safe_directory_component(value: &str, fallback: &str) -> String {
    let mut rendered = String::new();
    let mut previous_was_separator = false;
    for character in value.trim().chars() {
        let safe = character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-');
        if safe {
            rendered.push(character);
            previous_was_separator = false;
        } else if !previous_was_separator {
            rendered.push('_');
            previous_was_separator = true;
        }
    }
    let rendered = rendered.trim_matches([' ', '.', '_']).to_string();
    if rendered.is_empty() {
        fallback.to_string()
    } else {
        rendered
    }
}

fn sar_filename(product: &SarProduct) -> Option<String> {
    if let Some(file_name) = product.file_name.as_deref().map(str::trim) {
        return is_safe_file_name(file_name).then(|| file_name.to_string());
    }
    if let Some(url) = product.download_url.as_deref() {
        let without_fragment = url.split('#').next().unwrap_or(url);
        let without_query = without_fragment
            .split('?')
            .next()
            .unwrap_or(without_fragment);
        if let Some(file_name) = without_query
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
        {
            return is_safe_file_name(file_name).then(|| file_name.to_string());
        }
    }
    let fallback = safe_directory_component(&product.product_id, "sar_product");
    is_safe_file_name(&fallback).then_some(fallback)
}

pub fn plan_sar_product_target(
    selected_root: &Path,
    product: &SarProduct,
    organize: bool,
) -> ProviderResult<PathBuf> {
    let Some(file_name) = sar_filename(product) else {
        let context = Map::from_iter([("dataset".to_string(), json!(product.dataset))]);
        return Err(ContractError::new(
            "V3_INVALID_TARGET_PATH",
            "涓嬭浇鐩爣璺緞鏃犳晥",
            context,
        )
        .expect("the SAR target-path error context is safe"));
    };
    if !organize {
        return Ok(selected_root.join(file_name));
    }
    let dataset =
        safe_directory_component(&product.dataset.to_ascii_uppercase(), "UNKNOWN_DATASET");
    let level_source = product
        .processing_level
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(product.product_family.as_str());
    let level = safe_directory_component(
        &level_source.to_ascii_uppercase().replace('.', "_"),
        "OTHER",
    );
    Ok(selected_root.join(dataset).join(level).join(file_name))
}

pub fn plan_sar_catalog_target(
    selected_root: &Path,
    dataset: &str,
    processing_level: &str,
    file_name: &str,
) -> ProviderResult<PathBuf> {
    if dataset.trim().is_empty()
        || processing_level.trim().is_empty()
        || dataset.contains(['/', '\\'])
        || processing_level.contains(['/', '\\'])
        || !is_safe_file_name(file_name)
    {
        return Err(ContractError::new(
            "V3_INVALID_TARGET_PATH",
            "SAR catalog target path is invalid",
            Map::new(),
        )
        .expect("empty target-path error context is safe"));
    }
    let dataset = safe_directory_component(&dataset.to_ascii_uppercase(), "UNKNOWN_DATASET");
    let level = safe_directory_component(
        &processing_level.to_ascii_uppercase().replace('.', "_"),
        "OTHER",
    );
    Ok(selected_root.join(dataset).join(level).join(file_name))
}
