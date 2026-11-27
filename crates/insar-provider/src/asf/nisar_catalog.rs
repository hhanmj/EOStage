//! Live NASA ASF CMR multi-product catalog & UMM-G metadata mappers for
//! NASA ARIA S1-GUNW and NASA-ISRO NISAR L-band / S-band science products.

use serde_json::{json, Value};

/// Empty-state explanation when NASA CMR returns zero ARIA S1-GUNW granules.
pub const ARIA_S1_GUNW_EMPTY_REASON_ZH: &str =
    "当前研究区与时间窗口内暂无 NASA ARIA S1-GUNW 公开干涉产品（ARIA S1-GUNW 仅覆盖特定地震与构造形变任务帧，非全球连续覆盖）。";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NisarBand {
    L,
    S,
}

impl NisarBand {
    pub const fn code(self) -> &'static str {
        match self {
            Self::L => "L",
            Self::S => "S",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NisarProductType {
    Rslc,
    Gslc,
    Gcov,
    Gunw,
    Goff,
    Rifg,
    Runw,
    Roff,
    Sme2,
}

impl NisarProductType {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Rslc => "RSLC",
            Self::Gslc => "GSLC",
            Self::Gcov => "GCOV",
            Self::Gunw => "GUNW",
            Self::Goff => "GOFF",
            Self::Rifg => "RIFG",
            Self::Runw => "RUNW",
            Self::Roff => "ROFF",
            Self::Sme2 => "SME2",
        }
    }

    pub const fn science_level(self) -> &'static str {
        match self {
            Self::Rslc | Self::Rifg | Self::Runw | Self::Roff => "L1",
            Self::Gslc | Self::Gcov | Self::Gunw | Self::Goff => "L2",
            Self::Sme2 => "L3",
        }
    }

    pub const fn description_en(self) -> &'static str {
        match self {
            Self::Rslc => "Range Doppler Single Look Complex",
            Self::Gslc => "Geocoded Single Look Complex",
            Self::Gcov => "Geocoded Polarimetric Covariance",
            Self::Gunw => "Geocoded Unwrapped Interferogram",
            Self::Goff => "Geocoded Pixel Offsets",
            Self::Rifg => "Range Doppler Wrapped Interferogram",
            Self::Runw => "Range Doppler Unwrapped Interferogram",
            Self::Roff => "Range Doppler Pixel Offsets",
            Self::Sme2 => "Soil Moisture (E2E/L3)",
        }
    }

    pub const fn supports_client_subset_v2(self) -> bool {
        matches!(self, Self::Rslc)
    }

    pub fn from_code(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "RSLC" => Some(Self::Rslc),
            "GSLC" => Some(Self::Gslc),
            "GCOV" => Some(Self::Gcov),
            "GUNW" => Some(Self::Gunw),
            "GOFF" => Some(Self::Goff),
            "RIFG" => Some(Self::Rifg),
            "RUNW" => Some(Self::Runw),
            "ROFF" => Some(Self::Roff),
            "SME2" => Some(Self::Sme2),
            _ => None,
        }
    }

    pub const fn primary_l_band_selectable() -> &'static [Self] {
        &[Self::Rslc, Self::Gslc, Self::Gcov, Self::Gunw]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NisarReleaseTier {
    Validated,
    Provisional,
    AgencyStandard,
}

impl NisarReleaseTier {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Validated => "Validated",
            Self::Provisional => "Provisional",
            Self::AgencyStandard => "AgencyStandard",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NisarProductAvailability {
    LiveAsfCmr,
    LiveLimitedValidatedAsfCmr,
    PendingAgencyRelease,
    ProtectedBhoonidhiPortal,
}

impl NisarProductAvailability {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LiveAsfCmr => "live_asf_cmr",
            Self::LiveLimitedValidatedAsfCmr => "live_limited_validated_asf_cmr",
            Self::PendingAgencyRelease => "pending_agency_release",
            Self::ProtectedBhoonidhiPortal => "protected_bhoonidhi_portal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NisarProductCatalogEntry {
    pub band: NisarBand,
    pub product_type: NisarProductType,
    pub science_level: &'static str,
    pub tier: NisarReleaseTier,
    pub short_name: &'static str,
    pub collection_concept_id: Option<&'static str>,
    pub availability: NisarProductAvailability,
    pub provider_endpoint: &'static str,
    pub supports_client_subset_v2: bool,
    pub notes: &'static str,
}

const ASF_CMR_UMM_URL: &str = "https://cmr.earthdata.nasa.gov/search/granules.umm_json";
const BHOONIDHI_SEARCH_ENDPOINT: &str = "https://bhoonidhi-api.nrsc.gov.in/data/search";

/// Live-verified (2026-11-15) catalog of NASA ASF L-band and ISRO Bhoonidhi S-band
/// NISAR collections across Level-1, Level-2, and Level-3 products.
pub const NISAR_MULTI_PRODUCT_CATALOG: &[NisarProductCatalogEntry] = &[
    // L-band Validated collections on ASF CMR
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Rslc,
        science_level: "L1",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L1_RSLC_V1",
        collection_concept_id: Some("C3622236985-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: true,
        notes: "Validated L-SAR L1 RSLC on NASA ASF CMR (~19.2k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Gslc,
        science_level: "L2",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L2_GSLC_V1",
        collection_concept_id: Some("C3622244601-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Validated L-SAR L2 GSLC on NASA ASF CMR (~19.1k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Gcov,
        science_level: "L2",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L2_GCOV_V1",
        collection_concept_id: Some("C3622241360-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Validated L-SAR L2 GCOV on NASA ASF CMR (~19.2k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Gunw,
        science_level: "L2",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L2_GUNW_V1",
        collection_concept_id: Some("C3622247503-ASF"),
        availability: NisarProductAvailability::LiveLimitedValidatedAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Validated L-SAR L2 GUNW on ASF CMR (140 granules; pair with Provisional for broad coverage).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Goff,
        science_level: "L2",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L2_GOFF_V1",
        collection_concept_id: Some("C3622250865-ASF"),
        availability: NisarProductAvailability::LiveLimitedValidatedAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Validated L-SAR L2 GOFF on ASF CMR (32 granules; Provisional holds ~27.7k).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Rifg,
        science_level: "L1",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L1_RIFG_V1",
        collection_concept_id: Some("C3622229381-ASF"),
        availability: NisarProductAvailability::LiveLimitedValidatedAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Validated L-SAR L1 RIFG on ASF CMR (33 granules; Provisional holds ~27.2k).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Runw,
        science_level: "L1",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L1_RUNW_V1",
        collection_concept_id: Some("C3622237369-ASF"),
        availability: NisarProductAvailability::LiveLimitedValidatedAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Validated L-SAR L1 RUNW on ASF CMR (33 granules; Provisional holds ~27.2k).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Roff,
        science_level: "L1",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L1_ROFF_V1",
        collection_concept_id: Some("C3622241997-ASF"),
        availability: NisarProductAvailability::LiveLimitedValidatedAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Validated L-SAR L1 ROFF on ASF CMR (32 granules; Provisional holds ~27.7k).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Sme2,
        science_level: "L3",
        tier: NisarReleaseTier::Validated,
        short_name: "NISAR_L3_SME2_V1",
        collection_concept_id: None,
        availability: NisarProductAvailability::PendingAgencyRelease,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Validated NISAR_L3_SME2_V1 not yet published on CMR (0 hits 2026-11); use Provisional.",
    },
    // L-band Provisional collections on ASF CMR
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Rslc,
        science_level: "L1",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L1_RSLC_PROVISIONAL_V1",
        collection_concept_id: Some("C2853145197-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: true,
        notes: "Provisional L-SAR L1 RSLC on NASA ASF CMR (~164.9k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Gslc,
        science_level: "L2",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L2_GSLC_PROVISIONAL_V1",
        collection_concept_id: Some("C2854332392-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Provisional L-SAR L2 GSLC on NASA ASF CMR (~162.9k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Gcov,
        science_level: "L2",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L2_GCOV_PROVISIONAL_V1",
        collection_concept_id: Some("C2854338529-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Provisional L-SAR L2 GCOV on NASA ASF CMR (~163.7k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Gunw,
        science_level: "L2",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L2_GUNW_PROVISIONAL_V1",
        collection_concept_id: Some("C2854335566-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Provisional L-SAR L2 GUNW on NASA ASF CMR (~94.2k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Goff,
        science_level: "L2",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L2_GOFF_PROVISIONAL_V1",
        collection_concept_id: Some("C2854341702-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Provisional L-SAR L2 GOFF on NASA ASF CMR (~27.7k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Rifg,
        science_level: "L1",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L1_RIFG_PROVISIONAL_V1",
        collection_concept_id: Some("C2853147928-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Provisional L-SAR L1 RIFG on NASA ASF CMR (~27.2k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Runw,
        science_level: "L1",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L1_RUNW_PROVISIONAL_V1",
        collection_concept_id: Some("C2853153429-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Provisional L-SAR L1 RUNW on NASA ASF CMR (~27.2k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Roff,
        science_level: "L1",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L1_ROFF_PROVISIONAL_V1",
        collection_concept_id: Some("C2853156054-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Provisional L-SAR L1 ROFF on NASA ASF CMR (~27.7k granules verified 2026-11).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::L,
        product_type: NisarProductType::Sme2,
        science_level: "L3",
        tier: NisarReleaseTier::Provisional,
        short_name: "NISAR_L3_SME2_PROVISIONAL_V1",
        collection_concept_id: Some("C2854344945-ASF"),
        availability: NisarProductAvailability::LiveAsfCmr,
        provider_endpoint: ASF_CMR_UMM_URL,
        supports_client_subset_v2: false,
        notes: "Provisional L-SAR L3 Soil Moisture on NASA ASF CMR (~103.9k granules verified 2026-11).",
    },
    // S-band collections via ISRO NRSC Bhoonidhi STAC
    NisarProductCatalogEntry {
        band: NisarBand::S,
        product_type: NisarProductType::Rslc,
        science_level: "L1",
        tier: NisarReleaseTier::AgencyStandard,
        short_name: "NISAR_SSAR_RSLC",
        collection_concept_id: None,
        availability: NisarProductAvailability::ProtectedBhoonidhiPortal,
        provider_endpoint: BHOONIDHI_SEARCH_ENDPOINT,
        supports_client_subset_v2: true,
        notes: "ISRO Bhoonidhi S-SAR L1 RSLC STAC route; requires Bhoonidhi user session (HTTP 401 anonymous).",
    },
    NisarProductCatalogEntry {
        band: NisarBand::S,
        product_type: NisarProductType::Gslc,
        science_level: "L2",
        tier: NisarReleaseTier::AgencyStandard,
        short_name: "NISAR_SSAR_GSLC",
        collection_concept_id: None,
        availability: NisarProductAvailability::PendingAgencyRelease,
        provider_endpoint: BHOONIDHI_SEARCH_ENDPOINT,
        supports_client_subset_v2: false,
        notes: "ISRO S-SAR L2 GSLC not publicly mirrored on NASA ASF CMR; pending ISRO Bhoonidhi public release.",
    },
    NisarProductCatalogEntry {
        band: NisarBand::S,
        product_type: NisarProductType::Gcov,
        science_level: "L2",
        tier: NisarReleaseTier::AgencyStandard,
        short_name: "NISAR_SSAR_GCOV",
        collection_concept_id: None,
        availability: NisarProductAvailability::PendingAgencyRelease,
        provider_endpoint: BHOONIDHI_SEARCH_ENDPOINT,
        supports_client_subset_v2: false,
        notes: "ISRO S-SAR L2 GCOV not publicly mirrored on NASA ASF CMR; pending ISRO Bhoonidhi public release.",
    },
    NisarProductCatalogEntry {
        band: NisarBand::S,
        product_type: NisarProductType::Gunw,
        science_level: "L2",
        tier: NisarReleaseTier::AgencyStandard,
        short_name: "NISAR_SSAR_GUNW",
        collection_concept_id: None,
        availability: NisarProductAvailability::PendingAgencyRelease,
        provider_endpoint: BHOONIDHI_SEARCH_ENDPOINT,
        supports_client_subset_v2: false,
        notes: "ISRO S-SAR L2 GUNW not publicly mirrored on NASA ASF CMR; pending ISRO Bhoonidhi public release.",
    },
];

pub fn nisar_product_catalog() -> &'static [NisarProductCatalogEntry] {
    NISAR_MULTI_PRODUCT_CATALOG
}

pub fn lookup_nisar_l_band_entries(
    product_type: NisarProductType,
) -> Vec<&'static NisarProductCatalogEntry> {
    NISAR_MULTI_PRODUCT_CATALOG
        .iter()
        .filter(|entry| {
            entry.band == NisarBand::L
                && entry.product_type == product_type
                && !matches!(
                    entry.availability,
                    NisarProductAvailability::PendingAgencyRelease
                )
        })
        .collect()
}

/// Resolve a user/API token (e.g. `GCOV`, `NISAR_GSLC`, `NISAR_L2_GUNW_V1`, `SME2`)
/// into the matching ASF CMR collection short name(s). Returns exact collection when
/// a full collection short name is supplied, or all live collections (Validated +
/// Provisional) for a bare NISAR product code.
pub fn resolve_nisar_l_band_cmr_short_names(
    raw: &str,
) -> Option<(NisarProductType, Vec<&'static str>)> {
    resolve_nisar_l_band_cmr_short_names_for_tiers(raw, true, true)
}

/// Resolve a user/API token into matching L-band ASF CMR collection short name(s),
/// optionally filtered by release tier (Validated V1 and/or Provisional).
/// If both flags are `false`, falls back to including both tiers.
pub fn resolve_nisar_l_band_cmr_short_names_for_tiers(
    raw: &str,
    include_validated: bool,
    include_provisional: bool,
) -> Option<(NisarProductType, Vec<&'static str>)> {
    let (allow_val, allow_prov) = if !include_validated && !include_provisional {
        (true, true)
    } else {
        (include_validated, include_provisional)
    };
    let normalized = raw.trim().to_ascii_uppercase();
    if let Some(exact) = NISAR_MULTI_PRODUCT_CATALOG.iter().find(|entry| {
        entry.band == NisarBand::L
            && !matches!(
                entry.availability,
                NisarProductAvailability::PendingAgencyRelease
            )
            && entry.short_name == normalized
    }) {
        return Some((exact.product_type, vec![exact.short_name]));
    }

    let candidate_code = normalized
        .strip_prefix("NISAR_L1_")
        .or_else(|| normalized.strip_prefix("NISAR_L2_"))
        .or_else(|| normalized.strip_prefix("NISAR_L3_"))
        .or_else(|| normalized.strip_prefix("NISAR_L_"))
        .or_else(|| normalized.strip_prefix("NISAR_"))
        .unwrap_or(normalized.as_str());
    let candidate_code = candidate_code
        .strip_suffix("_PROVISIONAL_V1")
        .or_else(|| candidate_code.strip_suffix("_V1"))
        .unwrap_or(candidate_code);

    let product_type = NisarProductType::from_code(candidate_code)?;
    let entries = lookup_nisar_l_band_entries(product_type);
    let filtered: Vec<&'static str> = entries
        .iter()
        .filter(|entry| match entry.tier {
            NisarReleaseTier::Validated => allow_val,
            NisarReleaseTier::Provisional => allow_prov,
            NisarReleaseTier::AgencyStandard => true,
        })
        .map(|entry| entry.short_name)
        .collect();
    // Never silently fall back to Provisional when the caller explicitly requested
    // Validated-only (e.g. SME2 has no published Validated collection on CMR yet).
    Some((product_type, filtered))
}

/// Strictly filter parsed NISAR items against the requested release tiers.
/// When both flags are `false`, falls back to allowing both tiers.
pub fn filter_nisar_items_by_release_tier(
    items: Vec<Value>,
    include_validated: bool,
    include_provisional: bool,
) -> Vec<Value> {
    let (allow_val, allow_prov) = if !include_validated && !include_provisional {
        (true, true)
    } else {
        (include_validated, include_provisional)
    };
    items
        .into_iter()
        .filter(
            |item| match item.get("release_tier").and_then(Value::as_str) {
                Some("validated") => allow_val,
                Some("provisional") => allow_prov,
                _ => true,
            },
        )
        .collect()
}

pub fn build_cmr_umm_search_url(
    short_names: &[&str],
    bbox: [f64; 4],
    temporal: Option<(&str, &str)>,
    page_size: usize,
) -> String {
    let mut url = String::from(ASF_CMR_UMM_URL);
    url.push('?');
    let mut first = true;
    for short_name in short_names {
        if !first {
            url.push('&');
        }
        first = false;
        if short_names.len() == 1 {
            url.push_str("short_name=");
        } else {
            url.push_str("short_name[]=");
        }
        url.push_str(short_name);
    }
    url.push_str(&format!(
        "&bounding_box={},{},{},{}&page_size={}&sort_key=-start_date",
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[3],
        page_size.clamp(1, 100)
    ));
    if let Some((start_raw, end_raw)) = temporal {
        let s = start_raw.trim();
        let e = end_raw.trim();
        if !s.is_empty() && !e.is_empty() {
            let s_temporal = if s.contains('T') {
                s.to_owned()
            } else {
                format!("{s}T00:00:00Z")
            };
            let e_temporal = if e.contains('T') {
                e.to_owned()
            } else {
                format!("{e}T23:59:59Z")
            };
            url.push_str(&format!("&temporal={s_temporal},{e_temporal}"));
        }
    }
    url
}

pub fn umm_additional_attr<'a>(umm: &'a Value, attr_name: &str) -> Option<&'a str> {
    umm.get("AdditionalAttributes")?
        .as_array()?
        .iter()
        .find(|attr| attr.get("Name").and_then(Value::as_str) == Some(attr_name))
        .and_then(|attr| attr.get("Values"))
        .and_then(Value::as_array)
        .and_then(|vals| vals.first())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub fn umm_spatial_polygon_and_bbox(umm: &Value, fallback_bbox: [f64; 4]) -> ([f64; 4], Value) {
    let mut poly_coords = Vec::new();
    let mut min_lon = f64::INFINITY;
    let mut min_lat = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut max_lat = f64::NEG_INFINITY;

    if let Some(points) = umm
        .pointer("/SpatialExtent/HorizontalSpatialDomain/Geometry/GPolygons/0/Boundary/Points")
        .and_then(Value::as_array)
    {
        for pt in points {
            if let (Some(lon), Some(lat)) = (
                pt.get("Longitude").and_then(Value::as_f64),
                pt.get("Latitude").and_then(Value::as_f64),
            ) {
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                poly_coords.push(json!([lon, lat]));
            }
        }
    }

    if poly_coords.len() >= 3 && poly_coords.first() != poly_coords.last() {
        if let Some(first) = poly_coords.first().cloned() {
            poly_coords.push(first);
        }
    }

    if poly_coords.len() >= 4 && min_lon.is_finite() && min_lat.is_finite() {
        return (
            [min_lon, min_lat, max_lon, max_lat],
            json!({ "type": "Polygon", "coordinates": [poly_coords] }),
        );
    }

    if let Some(rect) =
        umm.pointer("/SpatialExtent/HorizontalSpatialDomain/Geometry/BoundingRectangles/0")
    {
        if let (Some(w), Some(s), Some(e), Some(n)) = (
            rect.get("WestBoundingCoordinate").and_then(Value::as_f64),
            rect.get("SouthBoundingCoordinate").and_then(Value::as_f64),
            rect.get("EastBoundingCoordinate").and_then(Value::as_f64),
            rect.get("NorthBoundingCoordinate").and_then(Value::as_f64),
        ) {
            return (
                [w, s, e, n],
                json!({
                    "type": "Polygon",
                    "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]
                }),
            );
        }
    }

    (
        fallback_bbox,
        json!({
            "type": "Polygon",
            "coordinates": [[
                [fallback_bbox[0], fallback_bbox[1]],
                [fallback_bbox[2], fallback_bbox[1]],
                [fallback_bbox[2], fallback_bbox[3]],
                [fallback_bbox[0], fallback_bbox[3]],
                [fallback_bbox[0], fallback_bbox[1]],
            ]]
        }),
    )
}

pub fn umm_archive_size_bytes(umm: &Value, expected_ext: &str) -> Option<u64> {
    let entries = umm
        .pointer("/DataGranule/ArchiveAndDistributionInformation")
        .and_then(Value::as_array)?;
    let suffix = format!(".{expected_ext}").to_ascii_lowercase();
    for entry in entries {
        let name = entry
            .get("Name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.ends_with(&suffix) && !name.contains("_qa_stats.") {
            if let Some(bytes) = entry.get("SizeInBytes").and_then(Value::as_u64) {
                return Some(bytes);
            }
        }
    }
    None
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AriaS1GunwTitleParts {
    pub flight_direction: Option<String>,
    pub path: Option<u64>,
    pub reference_date: Option<String>,
    pub secondary_date: Option<String>,
    pub version: Option<String>,
}

fn format_yyyymmdd(raw: &str) -> Option<String> {
    if raw.len() == 8 && raw.bytes().all(|b| b.is_ascii_digit()) {
        Some(format!("{}-{}-{}", &raw[0..4], &raw[4..6], &raw[6..8]))
    } else {
        None
    }
}

/// Parse ARIA S1-GUNW granule naming convention, e.g.
/// `S1-GUNW-A-R-043-tops-20261024_20261012-163835-00028E_00026S-PP-9cdc-v3_0_1`.
pub fn parse_aria_s1_gunw_title(title: &str) -> AriaS1GunwTitleParts {
    let parts: Vec<&str> = title.trim().split('-').collect();
    let mut out = AriaS1GunwTitleParts::default();
    if parts.len() < 7 || !parts[0].eq_ignore_ascii_case("S1") {
        return out;
    }
    out.flight_direction = match parts.get(2).copied() {
        Some("A") => Some("ASCENDING".to_owned()),
        Some("D") => Some("DESCENDING".to_owned()),
        _ => None,
    };
    out.path = parts.get(4).and_then(|s| s.parse::<u64>().ok());
    if let Some(pair) = parts.iter().find(|tok| {
        tok.len() == 17
            && tok.as_bytes()[8] == b'_'
            && tok[..8].bytes().all(|b| b.is_ascii_digit())
            && tok[9..].bytes().all(|b| b.is_ascii_digit())
    }) {
        out.reference_date = format_yyyymmdd(&pair[..8]);
        out.secondary_date = format_yyyymmdd(&pair[9..]);
    }
    if let Some(ver) = parts.last().and_then(|s| s.strip_prefix('v')) {
        if !ver.is_empty() {
            out.version = Some(ver.replace('_', "."));
        }
    }
    out
}

pub fn parse_aria_s1_gunw_umm_items(cmr_resp: &Value, fallback_bbox: [f64; 4]) -> Vec<Value> {
    let Some(entries) = cmr_resp.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let meta = &entry["meta"];
        let umm = &entry["umm"];
        let id = umm["GranuleUR"]
            .as_str()
            .or_else(|| meta["native-id"].as_str())
            .unwrap_or("granule")
            .to_owned();
        let concept_id = meta["concept-id"].as_str().map(str::to_owned);
        let dt = umm
            .pointer("/TemporalExtent/RangeDateTime/BeginningDateTime")
            .and_then(Value::as_str)
            .unwrap_or("2024-01-01T00:00:00Z")
            .to_owned();
        let end_dt = umm
            .pointer("/TemporalExtent/RangeDateTime/EndingDateTime")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let mut data_url = String::new();
        let mut browse_url = String::new();
        if let Some(links) = umm.get("RelatedUrls").and_then(Value::as_array) {
            for lnk in links {
                let url = lnk.get("URL").and_then(Value::as_str).unwrap_or("");
                if !url.starts_with("https://") {
                    continue;
                }
                let kind = lnk.get("Type").and_then(Value::as_str).unwrap_or("");
                let clean = url.split('?').next().unwrap_or("").to_ascii_lowercase();
                if clean.ends_with(".nc") && kind == "GET DATA" && data_url.is_empty() {
                    data_url = url.to_owned();
                } else if clean.ends_with(".png")
                    && (kind == "GET RELATED VISUALIZATION" || browse_url.is_empty())
                    && browse_url.is_empty()
                {
                    browse_url = url.to_owned();
                }
            }
        }

        let title_parts = parse_aria_s1_gunw_title(&id);
        let platform_vehicle = umm
            .pointer("/Platforms/0/ShortName")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let product_type = umm_additional_attr(umm, "PRODUCT_TYPE").unwrap_or("GUNW");
        let product_type_desc = umm_additional_attr(umm, "PRODUCT_TYPE_DESC")
            .unwrap_or("Geocoded Unwrapped Interferogram");
        let beam_mode = umm_additional_attr(umm, "BEAM_MODE").unwrap_or("IW");
        let polarization = umm_additional_attr(umm, "POLARIZATION").unwrap_or("VV");
        let path = umm_additional_attr(umm, "PATH_NUMBER")
            .and_then(|s| s.parse::<u64>().ok())
            .or(title_parts.path);
        let frame = umm_additional_attr(umm, "FRAME_NUMBER").and_then(|s| s.parse::<u64>().ok());
        let flight_direction = umm_additional_attr(umm, "ASCENDING_DESCENDING")
            .map(str::to_owned)
            .or(title_parts.flight_direction);
        let perpendicular_baseline =
            umm_additional_attr(umm, "PERPENDICULAR_BASELINE").and_then(|s| s.parse::<f64>().ok());
        let temporal_baseline_days =
            umm_additional_attr(umm, "TEMPORAL_BASELINE_DAYS").and_then(|s| s.parse::<i64>().ok());
        let version = umm_additional_attr(umm, "VERSION")
            .map(str::to_owned)
            .or(title_parts.version);
        let reference_date = title_parts
            .reference_date
            .or_else(|| dt.get(..10).map(str::to_owned));
        let secondary_date = title_parts.secondary_date;
        let file_size_bytes = umm_archive_size_bytes(umm, "nc");
        let (item_bbox, geometry) = umm_spatial_polygon_and_bbox(umm, fallback_bbox);
        let preview_opt = if browse_url.is_empty() {
            None
        } else {
            Some(browse_url.clone())
        };

        items.push(json!({
            "id": id,
            "title": id,
            "concept_id": concept_id,
            "short_name": "ARIA_S1_GUNW",
            "platform": "Sentinel-1",
            "platform_vehicle": platform_vehicle,
            "product_type": product_type,
            "product_type_desc": product_type_desc,
            "beam_mode": beam_mode,
            "polarization": polarization,
            "path": path,
            "relative_orbit": path,
            "frame": frame,
            "flight_direction": flight_direction,
            "reference_date": reference_date,
            "secondary_date": secondary_date,
            "perpendicular_baseline": perpendicular_baseline,
            "temporal_baseline_days": temporal_baseline_days,
            "version": version,
            "datetime": dt,
            "end_date": end_dt,
            "href": data_url,
            "data_url": data_url,
            "download_url": data_url,
            "browse_url": browse_url,
            "thumbnail_url": preview_opt,
            "preview_url": preview_opt,
            "file_size_bytes": file_size_bytes,
            "size_bytes": file_size_bytes,
            "bbox": item_bbox,
            "geometry": geometry,
        }));
    }
    items
}

pub fn parse_nisar_l_band_umm_items(
    cmr_resp: &Value,
    default_short_name: &str,
    fallback_bbox: [f64; 4],
) -> Vec<Value> {
    let Some(entries) = cmr_resp.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let meta = &entry["meta"];
        let umm = &entry["umm"];
        let id = umm["GranuleUR"]
            .as_str()
            .or_else(|| meta["native-id"].as_str())
            .unwrap_or("granule")
            .to_owned();
        let concept_id = meta["concept-id"].as_str().map(str::to_owned);
        let collection_concept_id = meta["collection-concept-id"].as_str().map(str::to_owned);
        let dt = umm
            .pointer("/TemporalExtent/RangeDateTime/BeginningDateTime")
            .and_then(Value::as_str)
            .unwrap_or("2026-01-01T00:00:00Z")
            .to_owned();
        let end_dt = umm
            .pointer("/TemporalExtent/RangeDateTime/EndingDateTime")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let mut data_url = String::new();
        let mut browse_thumb = String::new();
        let mut browse_full = String::new();
        if let Some(links) = umm.get("RelatedUrls").and_then(Value::as_array) {
            for lnk in links {
                let url = lnk.get("URL").and_then(Value::as_str).unwrap_or("");
                if !url.starts_with("https://") {
                    continue;
                }
                let kind = lnk.get("Type").and_then(Value::as_str).unwrap_or("");
                let clean = url.split('?').next().unwrap_or("").to_ascii_lowercase();
                if clean.ends_with(".h5")
                    && kind == "GET DATA"
                    && !clean.contains("_qa_stats.h5")
                    && data_url.is_empty()
                {
                    data_url = url.to_owned();
                } else if clean.ends_with(".png") && kind == "GET RELATED VISUALIZATION" {
                    if clean.ends_with("_latlon_thumbnail.png") && browse_thumb.is_empty() {
                        browse_thumb = url.to_owned();
                    } else if clean.ends_with("_latlon.png") || browse_full.is_empty() {
                        browse_full = url.to_owned();
                    }
                }
            }
        }
        let browse_url = if !browse_thumb.is_empty() {
            browse_thumb
        } else {
            browse_full
        };

        let product_type_str = umm_additional_attr(umm, "PRODUCT_TYPE")
            .unwrap_or("RSLC")
            .to_ascii_uppercase();
        let parsed_product = NisarProductType::from_code(&product_type_str);
        let science_level = umm_additional_attr(umm, "PROCESSING_LEVEL")
            .map(str::to_owned)
            .or_else(|| parsed_product.map(|p| p.science_level().to_owned()))
            .unwrap_or_else(|| "L1".to_owned());
        let product_type_desc = umm_additional_attr(umm, "PRODUCT_TYPE_DESC")
            .map(str::to_owned)
            .or_else(|| parsed_product.map(|p| p.description_en().to_owned()));
        let freq_a_pol = umm_additional_attr(umm, "FREQUENCY_A_POLARIZATION_CONCAT");
        let freq_b_pol = umm_additional_attr(umm, "FREQUENCY_B_POLARIZATION_CONCAT");
        let polarization = match (freq_a_pol, freq_b_pol) {
            (Some(a), Some(b)) if a != b => format!("A:{a} B:{b}"),
            (Some(a), _) => a.to_owned(),
            (None, Some(b)) => b.to_owned(),
            (None, None) => "-".to_owned(),
        };
        let track = umm_additional_attr(umm, "TRACK_NUMBER").and_then(|s| s.parse::<u64>().ok());
        let frame = umm_additional_attr(umm, "FRAME_NUMBER").and_then(|s| s.parse::<u64>().ok());
        let flight_direction = umm_additional_attr(umm, "ASCENDING_DESCENDING").map(str::to_owned);
        let range_bandwidth = umm_additional_attr(umm, "RANGE_BANDWIDTH_CONCAT").map(str::to_owned);
        let look_direction = umm
            .pointer("/Platforms/0/Instruments/0/Characteristics")
            .and_then(Value::as_array)
            .and_then(|chars| {
                chars
                    .iter()
                    .find(|c| c.get("Name").and_then(Value::as_str) == Some("LookDirection"))
                    .and_then(|c| c.get("Value").and_then(Value::as_str))
            })
            .map(str::to_owned);
        let file_size_bytes = umm_archive_size_bytes(umm, "h5");
        let (item_bbox, geometry) = umm_spatial_polygon_and_bbox(umm, fallback_bbox);
        let preview_opt = if browse_url.is_empty() {
            None
        } else {
            Some(browse_url.clone())
        };
        let umm_collection_short_name = umm
            .pointer("/CollectionReference/ShortName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let catalog_entry = collection_concept_id
            .as_deref()
            .and_then(|cid| {
                NISAR_MULTI_PRODUCT_CATALOG
                    .iter()
                    .find(|e| e.collection_concept_id == Some(cid))
            })
            .or_else(|| {
                if let Some(sn) = umm_collection_short_name {
                    NISAR_MULTI_PRODUCT_CATALOG
                        .iter()
                        .find(|e| e.short_name.eq_ignore_ascii_case(sn))
                } else {
                    NISAR_MULTI_PRODUCT_CATALOG
                        .iter()
                        .find(|e| e.short_name.eq_ignore_ascii_case(default_short_name))
                }
            });
        // IMPORTANT (JPL D-102268): NASA NISAR uses `_L1_PR_` / `_L2_PR_` ("PR" = science
        // Product) in BOTH Validated (`NISAR_L1_RSLC_V1`) and Provisional
        // (`NISAR_L1_RSLC_PROVISIONAL_V1`) granule filenames. Never infer release_tier from
        // `_PR_` in the granule ID; always derive it from CMR collection identity.
        let (release_tier, tier_label, resolved_short_name) = if let Some(entry) = catalog_entry {
            let rt = match entry.tier {
                NisarReleaseTier::Validated => "validated",
                NisarReleaseTier::Provisional => "provisional",
                NisarReleaseTier::AgencyStandard => "agency_standard",
            };
            (rt, entry.tier.label(), entry.short_name)
        } else {
            let effective_short_name = umm_collection_short_name.unwrap_or(default_short_name);
            let upper_short = effective_short_name.to_ascii_uppercase();
            if upper_short.contains("PROVISIONAL") || upper_short.contains("BETA") {
                ("provisional", "Provisional", effective_short_name)
            } else {
                ("validated", "Validated", effective_short_name)
            }
        };

        items.push(json!({
            "id": id,
            "title": id,
            "concept_id": concept_id,
            "collection_concept_id": collection_concept_id,
            "short_name": resolved_short_name,
            "collection_short_name": resolved_short_name,
            "release_tier": release_tier,
            "tier": tier_label,
            "platform": "NISAR",
            "band": "L",
            "instrument": "L-SAR",
            "product_type": product_type_str,
            "product_type_desc": product_type_desc,
            "science_level": science_level,
            "beam_mode": science_level,
            "polarization": polarization,
            "frequency_a_polarization": freq_a_pol,
            "frequency_b_polarization": freq_b_pol,
            "range_bandwidth": range_bandwidth,
            "path": track,
            "track": track,
            "relative_orbit": track,
            "frame": frame,
            "flight_direction": flight_direction,
            "look_direction": look_direction,
            "supports_subset_v2": parsed_product.is_some_and(NisarProductType::supports_client_subset_v2),
            "datetime": dt,
            "end_date": end_dt,
            "href": data_url,
            "data_url": data_url,
            "download_url": data_url,
            "browse_url": browse_url,
            "thumbnail_url": preview_opt,
            "preview_url": preview_opt,
            "file_size_bytes": file_size_bytes,
            "size_bytes": file_size_bytes,
            "bbox": item_bbox,
            "geometry": geometry,
        }));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aria_s1_gunw_umm_parser_extracts_interferogram_metadata_and_empty_stays_empty() {
        let payload = json!({
            "hits": 1,
            "items": [{
                "meta": {
                    "concept-id": "G4397855195-ASF",
                    "native-id": "S1-GUNW-A-R-043-tops-20261024_20261012-163835-00028E_00026S-PP-9cdc-v3_0_1"
                },
                "umm": {
                    "GranuleUR": "S1-GUNW-A-R-043-tops-20261024_20261012-163835-00028E_00026S-PP-9cdc-v3_0_1",
                    "TemporalExtent": {
                        "RangeDateTime": {
                            "BeginningDateTime": "2026-10-24T16:38:09Z",
                            "EndingDateTime": "2026-10-24T16:39:01Z"
                        }
                    },
                    "Platforms": [{"ShortName": "Sentinel-1D"}],
                    "AdditionalAttributes": [
                        {"Name": "ASCENDING_DESCENDING", "Values": ["ASCENDING"]},
                        {"Name": "BEAM_MODE", "Values": ["IW"]},
                        {"Name": "FRAME_NUMBER", "Values": ["6632"]},
                        {"Name": "PATH_NUMBER", "Values": ["43"]},
                        {"Name": "PERPENDICULAR_BASELINE", "Values": ["-50.3544"]},
                        {"Name": "POLARIZATION", "Values": ["VV"]},
                        {"Name": "PRODUCT_TYPE", "Values": ["GUNW"]},
                        {"Name": "PRODUCT_TYPE_DESC", "Values": ["Geocoded Unwrapped Interferogram"]},
                        {"Name": "TEMPORAL_BASELINE_DAYS", "Values": ["12"]},
                        {"Name": "VERSION", "Values": ["3.0.1"]}
                    ],
                    "DataGranule": {
                        "ArchiveAndDistributionInformation": [
                            {"Name": "S1-GUNW-A-R-043-tops-20261024_20261012-163835-00028E_00026S-PP-9cdc-v3_0_1.nc", "SizeInBytes": 118956194}
                        ]
                    },
                    "SpatialExtent": {
                        "HorizontalSpatialDomain": {
                            "Geometry": {
                                "GPolygons": [{
                                    "Boundary": {
                                        "Points": [
                                            {"Latitude": -26.14, "Longitude": 29.10},
                                            {"Latitude": -24.61, "Longitude": 30.39},
                                            {"Latitude": -24.81, "Longitude": 27.87},
                                            {"Latitude": -26.14, "Longitude": 29.10}
                                        ]
                                    }
                                }]
                            }
                        }
                    },
                    "RelatedUrls": [
                        {
                            "Type": "GET DATA",
                            "URL": "https://cumulus.asf.earthdatacloud.nasa.gov/ARIA/ARIA_S1_GUNW/example/file.nc"
                        },
                        {
                            "Type": "GET RELATED VISUALIZATION",
                            "URL": "https://cumulus.asf.earthdatacloud.nasa.gov/BROWSE/ARIA/ARIA_S1_GUNW/example/file.png"
                        }
                    ]
                }
            }]
        });

        let items = parse_aria_s1_gunw_umm_items(&payload, [27.0, -27.0, 31.0, -24.0]);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item["platform"], "Sentinel-1");
        assert_eq!(item["platform_vehicle"], "Sentinel-1D");
        assert_eq!(item["product_type"], "GUNW");
        assert_eq!(item["beam_mode"], "IW");
        assert_eq!(item["reference_date"], "2026-10-24");
        assert_eq!(item["secondary_date"], "2026-10-12");
        assert_eq!(item["path"], 43);
        assert_eq!(item["frame"], 6632);
        assert_eq!(item["flight_direction"], "ASCENDING");
        assert_eq!(item["temporal_baseline_days"], 12);
        assert_eq!(item["file_size_bytes"], 118956194);
        assert_eq!(
            item["href"],
            "https://cumulus.asf.earthdatacloud.nasa.gov/ARIA/ARIA_S1_GUNW/example/file.nc"
        );
        assert_eq!(item["geometry"]["type"], "Polygon");

        let empty_items = parse_aria_s1_gunw_umm_items(
            &json!({"hits": 0, "items": []}),
            [103.5, 30.2, 104.5, 31.1],
        );
        assert!(empty_items.is_empty());
    }

    #[test]
    fn nisar_multi_product_catalog_resolves_l_band_and_documents_s_band() {
        for product in NisarProductType::primary_l_band_selectable() {
            let entries = lookup_nisar_l_band_entries(*product);
            assert!(
                entries.len() == 2,
                "{:?} should expose both Validated and Provisional ASF CMR entries",
                product
            );
            assert!(entries.iter().all(|e| e.collection_concept_id.is_some()));
        }

        let (gcov, short_names) = resolve_nisar_l_band_cmr_short_names("GCOV").expect("GCOV");
        assert_eq!(gcov, NisarProductType::Gcov);
        assert_eq!(
            short_names,
            vec!["NISAR_L2_GCOV_V1", "NISAR_L2_GCOV_PROVISIONAL_V1"]
        );

        let url = build_cmr_umm_search_url(
            &short_names,
            [-120.0, 34.0, -118.0, 36.0],
            Some(("2026-10-01", "2026-11-01")),
            15,
        );
        assert!(url.contains("short_name[]=NISAR_L2_GCOV_V1"));
        assert!(url.contains("short_name[]=NISAR_L2_GCOV_PROVISIONAL_V1"));
        assert!(url.contains("temporal=2026-10-01T00:00:00Z,2026-11-01T23:59:59Z"));

        let sband_entries: Vec<_> = nisar_product_catalog()
            .iter()
            .filter(|e| e.band == NisarBand::S)
            .collect();
        assert!(sband_entries
            .iter()
            .any(|e| e.product_type == NisarProductType::Rslc
                && e.availability == NisarProductAvailability::ProtectedBhoonidhiPortal));
        assert!(sband_entries
            .iter()
            .any(|e| e.product_type == NisarProductType::Gunw
                && e.availability == NisarProductAvailability::PendingAgencyRelease));

        let umm_gcov = json!({
            "hits": 1,
            "items": [{
                "meta": {"concept-id": "G123-ASF", "collection-concept-id": "C3622241360-ASF"},
                "umm": {
                    "GranuleUR": "NISAR_L2_PR_GCOV_035_127_A_009_2005_DVDV_A_20261113T235215_20261113T235237_P05036_N_P_J_001",
                    "TemporalExtent": {"RangeDateTime": {"BeginningDateTime": "2026-11-13T23:52:15Z", "EndingDateTime": "2026-11-13T23:52:37Z"}},
                    "Platforms": [{"ShortName": "NISAR", "Instruments": [{"ShortName": "L-SAR", "Characteristics": [{"Name": "LookDirection", "Value": "LEFT"}]}]}],
                    "AdditionalAttributes": [
                        {"Name": "PROCESSING_LEVEL", "Values": ["L2"]},
                        {"Name": "PRODUCT_TYPE", "Values": ["GCOV"]},
                        {"Name": "TRACK_NUMBER", "Values": ["127"]},
                        {"Name": "FRAME_NUMBER", "Values": ["9"]},
                        {"Name": "ASCENDING_DESCENDING", "Values": ["ASCENDING"]},
                        {"Name": "FREQUENCY_A_POLARIZATION_CONCAT", "Values": ["VV+VH"]}
                    ],
                    "DataGranule": {
                        "ArchiveAndDistributionInformation": [
                            {"Name": "NISAR_L2_PR_GCOV_example.h5", "SizeInBytes": 5368709120_u64}
                        ]
                    },
                    "RelatedUrls": [
                        {"Type": "GET DATA", "URL": "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/NISAR_L2_GCOV_V1/example.h5"},
                        {"Type": "GET DATA", "URL": "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/NISAR_L2_GCOV_V1/example_QA_STATS.h5"},
                        {"Type": "GET RELATED VISUALIZATION", "URL": "https://nisar.asf.earthdatacloud.nasa.gov/BROWSE/NISAR_L2_GCOV_V1/example_LATLON_thumbnail.png"}
                    ]
                }
            }]
        });
        let gcov_items = parse_nisar_l_band_umm_items(
            &umm_gcov,
            "NISAR_L2_GCOV_V1",
            [-120.0, 34.0, -118.0, 36.0],
        );
        assert_eq!(gcov_items.len(), 1);
        assert_eq!(gcov_items[0]["product_type"], "GCOV");
        assert_eq!(gcov_items[0]["science_level"], "L2");
        assert_eq!(gcov_items[0]["release_tier"], "validated");
        assert_eq!(gcov_items[0]["collection_short_name"], "NISAR_L2_GCOV_V1");
        assert_eq!(gcov_items[0]["supports_subset_v2"], false);
        assert_eq!(gcov_items[0]["look_direction"], "LEFT");
        assert_eq!(
            gcov_items[0]["href"],
            "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/NISAR_L2_GCOV_V1/example.h5"
        );

        let (_rslc, val_only) =
            resolve_nisar_l_band_cmr_short_names_for_tiers("RSLC", true, false).unwrap();
        assert_eq!(val_only, vec!["NISAR_L1_RSLC_V1"]);
        let (_rslc, prov_only) =
            resolve_nisar_l_band_cmr_short_names_for_tiers("RSLC", false, true).unwrap();
        assert_eq!(prov_only, vec!["NISAR_L1_RSLC_PROVISIONAL_V1"]);
        let (_rslc, both_tiers) =
            resolve_nisar_l_band_cmr_short_names_for_tiers("RSLC", true, true).unwrap();
        assert_eq!(
            both_tiers,
            vec!["NISAR_L1_RSLC_V1", "NISAR_L1_RSLC_PROVISIONAL_V1"]
        );

        // SME2 has no published Validated CMR collection yet: requesting Validated-only
        // must return an empty list rather than silently leaking Provisional granules.
        let (sme2, sme2_val_only) =
            resolve_nisar_l_band_cmr_short_names_for_tiers("SME2", true, false).unwrap();
        assert_eq!(sme2, NisarProductType::Sme2);
        assert!(
            sme2_val_only.is_empty(),
            "Validated-only SME2 must never silently fall back to Provisional"
        );
        let (_sme2, sme2_prov_only) =
            resolve_nisar_l_band_cmr_short_names_for_tiers("SME2", false, true).unwrap();
        assert_eq!(sme2_prov_only, vec!["NISAR_L3_SME2_PROVISIONAL_V1"]);
    }

    #[test]
    fn nisar_pr_product_marker_never_mislabels_validated_granules_as_provisional() {
        // Real NASA ASF CMR UMM-G structure (verified live 2026-11-19):
        // Validated granules carry `_L1_PR_RSLC_` ("PR" = science Product, NOT Provisional).
        let mixed_cmr = json!({
            "hits": 3,
            "items": [
                {
                    "meta": {
                        "concept-id": "G4417671784-ASF",
                        "collection-concept-id": "C3622236985-ASF"
                    },
                    "umm": {
                        "GranuleUR": "NISAR_L1_PR_RSLC_036_011_D_077_2005_DHDH_A_20261117T232437_20261117T232445_P05036_N_P_J_001",
                        "CollectionReference": {
                            "ShortName": "NISAR_L1_RSLC_V1",
                            "Version": "1"
                        },
                        "TemporalExtent": {"RangeDateTime": {"BeginningDateTime": "2026-11-17T23:24:37Z"}},
                        "AdditionalAttributes": [
                            {"Name": "PROCESSING_LEVEL", "Values": ["L1"]},
                            {"Name": "PRODUCT_TYPE", "Values": ["RSLC"]}
                        ]
                    }
                },
                {
                    // Even without meta.collection-concept-id, CollectionReference.ShortName
                    // on a future/unlisted validated collection must stay "validated" despite `_L1_PR_`.
                    "meta": {
                        "concept-id": "G9999999999-ASF"
                    },
                    "umm": {
                        "GranuleUR": "NISAR_L1_PR_RSLC_099_011_D_077_2005_DHDH_A_20261201T000000_20261201T000010_P05036_N_P_J_001",
                        "CollectionReference": {
                            "ShortName": "NISAR_L1_RSLC_V2",
                            "Version": "2"
                        },
                        "TemporalExtent": {"RangeDateTime": {"BeginningDateTime": "2026-12-01T00:00:00Z"}},
                        "AdditionalAttributes": [
                            {"Name": "PROCESSING_LEVEL", "Values": ["L1"]},
                            {"Name": "PRODUCT_TYPE", "Values": ["RSLC"]}
                        ]
                    }
                },
                {
                    "meta": {
                        "concept-id": "G4385152356-ASF",
                        "collection-concept-id": "C2853145197-ASF"
                    },
                    "umm": {
                        "GranuleUR": "NISAR_L1_PR_RSLC_033_139_D_091_4005_DHDH_A_20261021T203740_20261021T203744_P05023_N_P_J_001",
                        "CollectionReference": {
                            "ShortName": "NISAR_L1_RSLC_PROVISIONAL_V1",
                            "Version": "1"
                        },
                        "TemporalExtent": {"RangeDateTime": {"BeginningDateTime": "2026-10-21T20:37:40Z"}},
                        "AdditionalAttributes": [
                            {"Name": "PROCESSING_LEVEL", "Values": ["L1"]},
                            {"Name": "PRODUCT_TYPE", "Values": ["RSLC"]}
                        ]
                    }
                }
            ]
        });

        let parsed = parse_nisar_l_band_umm_items(
            &mixed_cmr,
            "NISAR_L1_RSLC_V1",
            [-120.0, 34.0, -118.0, 36.0],
        );
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0]["release_tier"], "validated");
        assert_eq!(parsed[0]["tier"], "Validated");
        assert_eq!(parsed[0]["collection_short_name"], "NISAR_L1_RSLC_V1");

        assert_eq!(parsed[1]["release_tier"], "validated");
        assert_eq!(parsed[1]["tier"], "Validated");
        assert_eq!(parsed[1]["collection_short_name"], "NISAR_L1_RSLC_V2");

        assert_eq!(parsed[2]["release_tier"], "provisional");
        assert_eq!(parsed[2]["tier"], "Provisional");
        assert_eq!(
            parsed[2]["collection_short_name"],
            "NISAR_L1_RSLC_PROVISIONAL_V1"
        );

        let val_only = filter_nisar_items_by_release_tier(parsed.clone(), true, false);
        assert_eq!(val_only.len(), 2);
        assert!(val_only.iter().all(|i| i["release_tier"] == "validated"));

        let prov_only = filter_nisar_items_by_release_tier(parsed.clone(), false, true);
        assert_eq!(prov_only.len(), 1);
        assert_eq!(prov_only[0]["release_tier"], "provisional");

        let both = filter_nisar_items_by_release_tier(parsed, true, true);
        assert_eq!(both.len(), 3);
    }
}
