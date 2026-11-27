//! Immutable, offline-only planner for a client-derived NISAR RSLC subset.
//!
//! This module consumes already-frozen catalogue identity plus reviewed local
//! geolocation evidence. It has no network, credential, resolver, or writer
//! capability and never falls back to a complete-product transfer.

use std::cmp::Ordering;

use geo::{Area, Geometry, Intersects, MultiPolygon, Polygon};
use geojson::Geometry as GeoJsonGeometry;
use insar_core::{
    validate_request_against_snapshot_item, AssetRole, ContractError, NisarSubsetByteEstimateV2,
    NisarSubsetChecksumAlgorithm, NisarSubsetChecksumV1, NisarSubsetExpectedOutputV1,
    NisarSubsetFrequency, NisarSubsetFrequencyCapabilityV2, NisarSubsetLogicalWindowV1,
    NisarSubsetMappingConfidence, NisarSubsetMappingV1, NisarSubsetPlanMetadataEvidenceV2,
    NisarSubsetPlanV2, NisarSubsetPlanV2Error, NisarSubsetPreparedIntentBuildInputV2,
    NisarSubsetPreparedIntentV2, NisarSubsetQualityStatusV2, NisarSubsetRequestManifestV1,
    NisarSubsetRequestV1, NisarSubsetSourceAssetIdentityV2, NisarSubsetSourceCapabilityV2,
    NisarSubsetSourceDependencyGraphV2, NisarSubsetSourceProductIdentityV2, NisarSubsetSourceV1,
    ProviderResult, SnapshotItem, NISAR_SUBSET_IMPLEMENTATION_SCOPE,
    NISAR_SUBSET_MAPPING_ALGORITHM, NISAR_SUBSET_MAPPING_ALGORITHM_VERSION, NISAR_SUBSET_MECHANISM,
    NISAR_SUBSET_METHOD, NISAR_SUBSET_OUTPUT_AUTHORITY, NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT,
    NISAR_SUBSET_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::asf::sparse_grid::{self, SparseGeoGrid, Window2D};
use crate::catalog::{
    NISAR_BAND, NISAR_FREQUENCY_A_POLARIZATIONS, NISAR_FREQUENCY_B_POLARIZATIONS,
    NISAR_NATIVE_COLLECTION_ID, NISAR_P05023_CRID, NISAR_P05023_PRODUCT_SPECIFICATION_VERSION,
    NISAR_PRODUCTION_CONFIGURATION, NISAR_PROFILE_ID, NISAR_PROVIDER_ID, NISAR_QUALITY_STATUS,
    NISAR_REQUIRED_PRODUCT_TYPE, NISAR_SCIENCE_LEVEL,
};

pub const NISAR_SUBSET_MAX_AOI_AREA_SQUARE_DEGREES: f64 = 1.0;
const NISAR_SUBSET_SWATH: &str = "LSAR_RSLC";
const NISAR_SSAR_SUBSET_SWATH: &str = "SSAR_RSLC";
const NISAR_SSAR_P00500_CRID: &str = "P00500";
const NISAR_SSAR_P00500_PRODUCT_SPECIFICATION_VERSION: &str = "1.2.1";
const MAX_AOI_COMPONENTS: usize = 64;
const MAX_AOI_RINGS: usize = 256;
const MAX_AOI_VERTICES: usize = 4_096;

/// Dataset-level polarizations supported by the reviewed RSLC planner.
/// Application/UI projections consume this helper instead of maintaining a
/// second scientific allow-list.
pub const fn supported_dataset_polarizations(
    frequency: &NisarSubsetFrequency,
) -> &'static [&'static str] {
    match frequency {
        NisarSubsetFrequency::A => &["HH", "HV", "VH", "VV"],
        NisarSubsetFrequency::B => &["HH", "HV", "VH", "VV"],
    }
}

pub const fn supported_ssar_dataset_polarizations() -> &'static [&'static str] {
    &["HH", "HV", "VH", "VV", "LH", "LV", "RH", "RV"]
}

fn live_geolocation_product_profile(
    frequency: &NisarSubsetFrequency,
    polarization: &str,
    swath: &str,
) -> ProviderResult<(&'static str, &'static str)> {
    match swath {
        NISAR_SUBSET_SWATH => {
            if !supported_dataset_polarizations(frequency).contains(&polarization) {
                return Err(not_supported("dataset_polarization"));
            }
            Ok((
                NISAR_P05023_PRODUCT_SPECIFICATION_VERSION,
                NISAR_P05023_CRID,
            ))
        }
        NISAR_SSAR_SUBSET_SWATH => {
            if *frequency != NisarSubsetFrequency::A {
                return Err(not_supported("frequency"));
            }
            if !supported_ssar_dataset_polarizations().contains(&polarization) {
                return Err(not_supported("dataset_polarization"));
            }
            Ok((
                NISAR_SSAR_P00500_PRODUCT_SPECIFICATION_VERSION,
                NISAR_SSAR_P00500_CRID,
            ))
        }
        _ => Err(not_supported("swath")),
    }
}

/// Parse one authoritative profile label into the canonical dataset-level
/// polarization set used by the planner and application projections.
///
/// Profile labels are intentionally kept in their source order at the
/// catalogue boundary (for example, `VV+VH`), while all downstream authority
/// uses sorted, unique values (`VH`, `VV`).
pub fn canonical_nisar_polarizations_for_frequency_label(
    frequency: &NisarSubsetFrequency,
    label: &str,
) -> Option<Vec<String>> {
    let profile_labels = match frequency {
        NisarSubsetFrequency::A => NISAR_FREQUENCY_A_POLARIZATIONS,
        NisarSubsetFrequency::B => NISAR_FREQUENCY_B_POLARIZATIONS,
    };
    if !profile_labels.contains(&label) {
        return None;
    }
    let mut polarizations = label.split('+').map(str::to_owned).collect::<Vec<_>>();
    if polarizations.is_empty()
        || polarizations
            .iter()
            .any(|value| !supported_dataset_polarizations(frequency).contains(&value.as_str()))
    {
        return None;
    }
    polarizations.sort();
    if polarizations.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    Some(polarizations)
}

#[derive(Clone, Debug, PartialEq)]
pub struct NisarSubsetNormalizedAoiV1 {
    pub canonical_geojson: Map<String, Value>,
    pub sha256: String,
    pub component_count: u16,
    pub ring_count: u16,
    pub vertex_count: u32,
    pub area_square_degrees: f64,
    geometry: MultiPolygon<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetEvidenceWindowV1 {
    pub component_id: u16,
    pub line_start: u64,
    pub line_end_exclusive: u64,
    pub sample_start: u64,
    pub sample_end_exclusive: u64,
    pub margin_lines: u64,
    pub margin_samples: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetGeolocationEvidenceV1 {
    pub aoi_sha256: String,
    pub algorithm: String,
    pub algorithm_version: u32,
    pub evidence_digest: String,
    pub product_specification_version: String,
    pub crid: String,
    pub frequency: NisarSubsetFrequency,
    pub swath: String,
    pub source_shape: [u64; 2],
    pub chunk_shape: [u64; 2],
    pub dtype: String,
    pub windows: Vec<NisarSubsetEvidenceWindowV1>,
    pub estimated_remote_bytes: u64,
    pub estimated_output_bytes: u64,
}

/// Prepared-intent input whose source identity, footprint and actual
/// polarization capability are always projected from the immutable SnapshotItem.
#[derive(Clone, Debug, PartialEq)]
pub struct AsfNisarSubsetPreparedIntentDraftV2 {
    pub requested_geometry: Map<String, Value>,
    pub selected_frequency: NisarSubsetFrequency,
    pub selected_polarizations: Vec<String>,
}

/// Phase-1 input whose metadata authority is added only after a prepared
/// intent has frozen the pre-metadata scientific selection.
#[derive(Clone, Debug, PartialEq)]
pub struct AsfNisarSubsetPlanDraftV2 {
    pub requested_geometry: Map<String, Value>,
    pub selected_frequency: NisarSubsetFrequency,
    pub selected_polarizations: Vec<String>,
    pub source_dependency_graph: NisarSubsetSourceDependencyGraphV2,
    pub transfer_estimate: Option<NisarSubsetByteEstimateV2>,
    pub derived_estimate: Option<NisarSubsetByteEstimateV2>,
    pub warnings: Vec<String>,
}

/// Builds one authoritative prepared intent without requiring metadata or
/// dependency-graph evidence and without mutating the frozen source.
pub fn prepare_nisar_subset_intent_v2(
    snapshot_item: &SnapshotItem,
    draft: AsfNisarSubsetPreparedIntentDraftV2,
) -> ProviderResult<NisarSubsetPreparedIntentV2> {
    validate_supported_snapshot(snapshot_item)?;
    if metadata_text(snapshot_item, "quality_status") != Some(NISAR_QUALITY_STATUS) {
        return Err(not_supported("quality_status"));
    }
    let footprint = snapshot_item
        .footprint_geojson
        .as_ref()
        .ok_or_else(|| mapping_error("source_footprint"))?
        .clone();
    let asset = snapshot_item
        .asset_access
        .as_ref()
        .ok_or_else(|| not_supported("primary_asset"))?;
    let file_name = asset
        .file_name
        .as_ref()
        .filter(|name| name.to_ascii_lowercase().ends_with(".h5"))
        .ok_or_else(|| not_supported("source_file"))?
        .clone();
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: NisarSubsetSourceProductIdentityV2 {
            provider_id: snapshot_item.provider_id.clone(),
            collection_id: asset.reference.collection_id.clone(),
            collection_profile_id: NISAR_PROFILE_ID.to_owned(),
            provider_product_id: asset.reference.provider_product_id.clone(),
            provider_product_key: asset.reference.provider_product_key.clone(),
            product_specification_version: NISAR_P05023_PRODUCT_SPECIFICATION_VERSION.to_owned(),
            crid: NISAR_P05023_CRID.to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        source_asset_identity: NisarSubsetSourceAssetIdentityV2 {
            asset_key: asset.reference.asset_key.clone(),
            stable_asset_key: asset.reference.stable_asset_key.clone(),
            file_name,
            checksum: snapshot_checksum_v2(snapshot_item)?,
        },
        source_capability: source_capability_v2(snapshot_item)?,
        source_footprint: footprint,
        requested_geometry: draft.requested_geometry,
        selected_frequency: draft.selected_frequency,
        selected_polarizations: draft.selected_polarizations,
    })
    .map_err(plan_v2_error)
}

/// Builds one authoritative V2 plan without mutating the Workbench AOI,
/// SnapshotItem, Store, Range engine, or writer.
pub fn plan_nisar_subset_v2(
    snapshot_item: &SnapshotItem,
    draft: AsfNisarSubsetPlanDraftV2,
) -> ProviderResult<NisarSubsetPlanV2> {
    let AsfNisarSubsetPlanDraftV2 {
        requested_geometry,
        selected_frequency,
        selected_polarizations,
        source_dependency_graph,
        transfer_estimate,
        derived_estimate,
        warnings,
    } = draft;
    let prepared_intent = prepare_nisar_subset_intent_v2(
        snapshot_item,
        AsfNisarSubsetPreparedIntentDraftV2 {
            requested_geometry,
            selected_frequency,
            selected_polarizations,
        },
    )?;
    let asset = snapshot_item
        .asset_access
        .as_ref()
        .ok_or_else(|| not_supported("primary_asset"))?;
    let source_estimate = asset.expected_size_bytes.map(|bytes| {
        let evidence = json!({
            "stable_asset_key": asset.reference.stable_asset_key,
            "bytes": bytes,
            "checksum": prepared_intent.source_asset_identity.checksum,
        });
        NisarSubsetByteEstimateV2 {
            bytes,
            evidence_sha256: sha256(canonical_value(&evidence).as_bytes()),
        }
    });
    NisarSubsetPlanV2::build_from_prepared_intent(
        prepared_intent,
        source_dependency_graph,
        NisarSubsetPlanMetadataEvidenceV2 {
            source_estimate,
            transfer_estimate,
            derived_estimate,
            warnings,
        },
    )
    .map_err(plan_v2_error)
}

pub fn normalize_nisar_subset_aoi(
    aoi_geojson: &Map<String, Value>,
) -> ProviderResult<NisarSubsetNormalizedAoiV1> {
    let keys = aoi_geojson.keys().map(String::as_str).collect::<Vec<_>>();
    if keys
        .iter()
        .any(|key| !matches!(*key, "type" | "coordinates"))
    {
        return Err(mapping_error("aoi_fields"));
    }
    let kind = aoi_geojson
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| mapping_error("aoi_type"))?;
    let coordinates = aoi_geojson
        .get("coordinates")
        .and_then(Value::as_array)
        .ok_or_else(|| mapping_error("aoi_coordinates"))?;
    let mut polygons = match kind {
        "Polygon" => vec![normalize_polygon(coordinates)?],
        "MultiPolygon" if !coordinates.is_empty() => coordinates
            .iter()
            .map(|polygon| {
                polygon
                    .as_array()
                    .ok_or_else(|| mapping_error("aoi_polygon"))
                    .and_then(|polygon| normalize_polygon(polygon))
            })
            .collect::<ProviderResult<Vec<_>>>()?,
        _ => return Err(mapping_error("aoi_type")),
    };
    if polygons.is_empty() || polygons.len() > MAX_AOI_COMPONENTS {
        return Err(mapping_error("aoi_components"));
    }
    polygons.sort_by_key(canonical_value);
    if polygons.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(mapping_error("duplicate_aoi_component"));
    }

    let canonical_coordinates = if polygons.len() == 1 {
        Value::Array(polygons[0].clone())
    } else {
        Value::Array(polygons.iter().cloned().map(Value::Array).collect())
    };
    let canonical_geojson = Map::from_iter([
        (
            "type".to_owned(),
            Value::String(
                if polygons.len() == 1 {
                    "Polygon"
                } else {
                    "MultiPolygon"
                }
                .to_owned(),
            ),
        ),
        ("coordinates".to_owned(), canonical_coordinates),
    ]);
    let geometry = parse_geometry(&canonical_geojson)?;
    if geometry.0.iter().enumerate().any(|(index, polygon)| {
        geometry
            .0
            .iter()
            .skip(index + 1)
            .any(|other| polygon.intersects(other))
    }) {
        return Err(mapping_error("intersecting_aoi_components"));
    }
    let area_square_degrees = geometry.unsigned_area();
    if !area_square_degrees.is_finite()
        || area_square_degrees <= 0.0
        || area_square_degrees > NISAR_SUBSET_MAX_AOI_AREA_SQUARE_DEGREES
    {
        return Err(mapping_error("aoi_area"));
    }
    let ring_count = polygons.iter().map(Vec::len).sum::<usize>();
    let vertex_count = polygons
        .iter()
        .flat_map(|polygon| polygon.iter())
        .map(|ring| ring.as_array().map_or(0, Vec::len))
        .sum::<usize>();
    if ring_count == 0
        || ring_count > MAX_AOI_RINGS
        || vertex_count == 0
        || vertex_count > MAX_AOI_VERTICES
    {
        return Err(mapping_error("aoi_complexity"));
    }
    let canonical_text =
        serde_json::to_string(&canonical_geojson).map_err(|_| mapping_error("aoi_canonical"))?;
    Ok(NisarSubsetNormalizedAoiV1 {
        sha256: sha256(canonical_text.as_bytes()),
        component_count: u16::try_from(polygons.len())
            .map_err(|_| mapping_error("aoi_components"))?,
        ring_count: u16::try_from(ring_count).map_err(|_| mapping_error("aoi_rings"))?,
        vertex_count: u32::try_from(vertex_count).map_err(|_| mapping_error("aoi_vertices"))?,
        area_square_degrees,
        canonical_geojson,
        geometry,
    })
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[doc(hidden)]
pub fn validate_nisar_catalogue_footprint_intersection(
    footprint_geojson: &Map<String, Value>,
    aoi_geojson: &Map<String, Value>,
) -> ProviderResult<()> {
    let aoi = normalize_nisar_subset_aoi(aoi_geojson)?;
    let footprint = parse_geometry(footprint_geojson)?;
    if footprint.intersects(&aoi.geometry) {
        Ok(())
    } else {
        Err(mapping_error("no_overlap"))
    }
}

pub fn plan_nisar_subset(
    snapshot_item: &SnapshotItem,
    aoi_geojson: &Map<String, Value>,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    swath: &str,
    evidence: &NisarSubsetGeolocationEvidenceV1,
) -> ProviderResult<NisarSubsetRequestManifestV1> {
    validate_supported_snapshot(snapshot_item)?;
    validate_dataset_polarization(snapshot_item, &frequency, polarization)?;
    if swath != NISAR_SUBSET_SWATH {
        return Err(not_supported("swath"));
    }
    let aoi = normalize_nisar_subset_aoi(aoi_geojson)?;
    validate_footprint_intersection(snapshot_item, &aoi.geometry)?;
    validate_evidence(evidence, &aoi, &frequency, swath)?;

    let asset = snapshot_item
        .asset_access
        .as_ref()
        .ok_or_else(|| not_supported("primary_asset"))?;
    let expected_size_bytes = asset
        .expected_size_bytes
        .ok_or_else(|| not_supported("source_size"))?;
    let checksum_algorithm = asset
        .checksum_algorithm
        .as_deref()
        .ok_or_else(|| not_supported("source_checksum"))?;
    let checksum_algorithm = if checksum_algorithm.eq_ignore_ascii_case("sha256") {
        NisarSubsetChecksumAlgorithm::Sha256
    } else if checksum_algorithm.eq_ignore_ascii_case("md5") {
        NisarSubsetChecksumAlgorithm::Md5
    } else {
        return Err(not_supported("source_checksum_algorithm"));
    };
    let checksum_value = asset
        .checksum_value
        .as_ref()
        .ok_or_else(|| not_supported("source_checksum"))?
        .clone();
    let file_name = asset
        .file_name
        .as_ref()
        .filter(|name| name.to_ascii_lowercase().ends_with(".h5"))
        .ok_or_else(|| not_supported("source_file"))?
        .clone();

    let source = NisarSubsetSourceV1 {
        provider_id: snapshot_item.provider_id.clone(),
        collection_id: asset.reference.collection_id.clone(),
        collection_profile_id: NISAR_PROFILE_ID.to_owned(),
        provider_product_id: asset.reference.provider_product_id.clone(),
        provider_product_key: asset.reference.provider_product_key.clone(),
        stable_asset_key: asset.reference.stable_asset_key.clone(),
        file_name: file_name.clone(),
        expected_size_bytes,
        checksum: Some(NisarSubsetChecksumV1 {
            algorithm: checksum_algorithm,
            value: checksum_value,
        }),
        product_specification_version: NISAR_P05023_PRODUCT_SPECIFICATION_VERSION.to_owned(),
        crid: NISAR_P05023_CRID.to_owned(),
    };

    let mut evidence_windows = evidence.windows.clone();
    evidence_windows.sort_by(compare_evidence_windows);
    let windows = evidence_windows
        .iter()
        .map(|window| NisarSubsetLogicalWindowV1 {
            component_id: window.component_id,
            dataset_path: format!(
                "/science/LSAR/RSLC/swaths/frequency{}/{polarization}",
                frequency_name(&frequency)
            ),
            frequency: frequency.clone(),
            polarization: polarization.to_owned(),
            swath: swath.to_owned(),
            line_start: window.line_start,
            line_end_exclusive: window.line_end_exclusive,
            sample_start: window.sample_start,
            sample_end_exclusive: window.sample_end_exclusive,
            source_shape: evidence.source_shape,
            expected_shape: [
                window.line_end_exclusive - window.line_start,
                window.sample_end_exclusive - window.sample_start,
            ],
            dtype: evidence.dtype.clone(),
            chunk_shape: evidence.chunk_shape,
            margin_lines: window.margin_lines,
            margin_samples: window.margin_samples,
            mapping_confidence: NisarSubsetMappingConfidence::ReferenceBounded,
            validity_mask_required: true,
        })
        .collect::<Vec<_>>();

    let identity_seed = json!({
        "source": source,
        "aoi_sha256": aoi.sha256,
        "frequency": frequency_name(&frequency),
        "polarization": polarization,
        "swath": swath,
        "evidence_digest": evidence.evidence_digest,
        "windows": windows,
    });
    let identity_text =
        serde_json::to_string(&identity_seed).map_err(|_| mapping_error("identity"))?;
    let artifact_hash = sha256(identity_text.as_bytes());
    let source_stem = file_name
        .strip_suffix(".h5")
        .ok_or_else(|| not_supported("source_file"))?;
    let manifest = NisarSubsetRequestManifestV1 {
        manifest_contract: NISAR_SUBSET_REQUEST_MANIFEST_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_SCHEMA_VERSION,
        mechanism: NISAR_SUBSET_MECHANISM.to_owned(),
        implementation_scope: NISAR_SUBSET_IMPLEMENTATION_SCOPE.to_owned(),
        output_authority: NISAR_SUBSET_OUTPUT_AUTHORITY.to_owned(),
        default_enabled: false,
        source,
        request: NisarSubsetRequestV1 {
            aoi_sha256: aoi.sha256,
            aoi_crs: "EPSG:4326".to_owned(),
            aoi_component_count: aoi.component_count,
            aoi_ring_count: aoi.ring_count,
            aoi_vertex_count: aoi.vertex_count,
            frequency,
            polarization: polarization.to_owned(),
            swath: swath.to_owned(),
            subset_method: NISAR_SUBSET_METHOD.to_owned(),
        },
        mapping: NisarSubsetMappingV1 {
            algorithm: evidence.algorithm.clone(),
            algorithm_version: evidence.algorithm_version,
            evidence_digest: evidence.evidence_digest.clone(),
            windows,
        },
        expected_output: NisarSubsetExpectedOutputV1 {
            derived_artifact_id: format!("nisar-subset-{}", &artifact_hash[..24]),
            file_name: format!("{source_stem}_subset_v1.h5"),
            estimated_output_bytes: evidence.estimated_output_bytes,
            estimated_remote_bytes: evidence.estimated_remote_bytes,
            full_product_bytes: expected_size_bytes,
        },
    };
    manifest
        .validate()
        .map_err(|_| mapping_error("manifest_contract"))?;
    validate_request_against_snapshot_item(&manifest, snapshot_item)
        .map_err(|_| not_supported("snapshot_identity"))?;
    Ok(manifest)
}

fn validate_supported_snapshot(item: &SnapshotItem) -> ProviderResult<()> {
    if item.provider_id != NISAR_PROVIDER_ID {
        return Err(not_supported("provider"));
    }
    item.validate().map_err(|_| not_supported("snapshot"))?;
    let asset = item
        .asset_access
        .as_ref()
        .ok_or_else(|| not_supported("primary_asset"))?;
    let policy = item
        .execution_policy
        .as_ref()
        .ok_or_else(|| not_supported("execution_policy"))?;
    if asset.role != AssetRole::Primary
        || asset.reference.provider_id != NISAR_PROVIDER_ID
        || asset.reference.collection_id != NISAR_NATIVE_COLLECTION_ID
        || policy.collection_profile_id.as_deref() != Some(NISAR_PROFILE_ID)
        || metadata_text(item, "collection_profile_id") != Some(NISAR_PROFILE_ID)
        || metadata_text(item, "band") != Some(NISAR_BAND)
        || metadata_text(item, "processing_level") != Some(NISAR_REQUIRED_PRODUCT_TYPE)
        || metadata_text(item, "science_level") != Some(NISAR_SCIENCE_LEVEL)
        || metadata_text(item, "production_configuration") != Some(NISAR_PRODUCTION_CONFIGURATION)
        || metadata_text(item, "crid") != Some(NISAR_P05023_CRID)
        || metadata_text(item, "product_specification_version")
            != Some(NISAR_P05023_PRODUCT_SPECIFICATION_VERSION)
    {
        return Err(not_supported("product_scope"));
    }
    Ok(())
}

fn validate_dataset_polarization(
    item: &SnapshotItem,
    frequency: &NisarSubsetFrequency,
    polarization: &str,
) -> ProviderResult<()> {
    let search_label = metadata_text(
        item,
        match frequency {
            NisarSubsetFrequency::A => "frequency_a_polarization",
            NisarSubsetFrequency::B => "frequency_b_polarization",
        },
    )
    .ok_or_else(|| not_supported("frequency_presence"))?;
    let available = canonical_nisar_polarizations_for_frequency_label(frequency, search_label)
        .is_some_and(|available| available.iter().any(|value| value == polarization));
    if !available {
        return Err(not_supported("dataset_polarization"));
    }
    Ok(())
}

fn validate_evidence(
    evidence: &NisarSubsetGeolocationEvidenceV1,
    aoi: &NisarSubsetNormalizedAoiV1,
    frequency: &NisarSubsetFrequency,
    swath: &str,
) -> ProviderResult<()> {
    if evidence.aoi_sha256 != aoi.sha256
        || evidence.algorithm != NISAR_SUBSET_MAPPING_ALGORITHM
        || evidence.algorithm_version != NISAR_SUBSET_MAPPING_ALGORITHM_VERSION
        || !is_sha256(&evidence.evidence_digest)
        || evidence.product_specification_version != NISAR_P05023_PRODUCT_SPECIFICATION_VERSION
        || evidence.crid != NISAR_P05023_CRID
        || &evidence.frequency != frequency
        || evidence.swath != swath
        || evidence.source_shape.contains(&0)
        || evidence.chunk_shape.contains(&0)
        || evidence.dtype.is_empty()
        || evidence.windows.is_empty()
        || evidence.windows.len() > 64
        || evidence.estimated_remote_bytes == 0
        || evidence.estimated_output_bytes == 0
    {
        return Err(mapping_error("geolocation_evidence"));
    }
    let mut covered = vec![false; usize::from(aoi.component_count)];
    for window in &evidence.windows {
        let component = usize::from(window.component_id);
        if component >= covered.len()
            || window.line_start >= window.line_end_exclusive
            || window.sample_start >= window.sample_end_exclusive
            || window.line_end_exclusive > evidence.source_shape[0]
            || window.sample_end_exclusive > evidence.source_shape[1]
        {
            return Err(mapping_error("geolocation_window"));
        }
        covered[component] = true;
    }
    if covered.iter().any(|covered| !covered) {
        return Err(mapping_error("component_coverage"));
    }
    Ok(())
}

fn validate_footprint_intersection(
    item: &SnapshotItem,
    aoi: &MultiPolygon<f64>,
) -> ProviderResult<()> {
    let footprint = item
        .footprint_geojson
        .as_ref()
        .ok_or_else(|| mapping_error("source_footprint"))?;
    let footprint = parse_geometry(footprint)?;
    if !footprint.intersects(aoi) {
        return Err(mapping_error("no_overlap"));
    }
    Ok(())
}

fn normalize_polygon(rings: &[Value]) -> ProviderResult<Vec<Value>> {
    if rings.is_empty() {
        return Err(mapping_error("aoi_rings"));
    }
    let mut normalized = rings
        .iter()
        .map(normalize_ring)
        .collect::<ProviderResult<Vec<_>>>()?;
    let exterior = normalized.remove(0);
    normalized.sort_by_key(canonical_value);
    let mut result = vec![exterior];
    result.extend(normalized);
    Ok(result)
}

fn normalize_ring(value: &Value) -> ProviderResult<Value> {
    let positions = value
        .as_array()
        .filter(|positions| positions.len() >= 4)
        .ok_or_else(|| mapping_error("aoi_ring"))?;
    let mut points = positions
        .iter()
        .map(|position| {
            let position = position
                .as_array()
                .filter(|position| position.len() == 2)
                .ok_or_else(|| mapping_error("aoi_position"))?;
            let longitude = position[0]
                .as_f64()
                .filter(|value| value.is_finite() && (-180.0..=180.0).contains(value))
                .ok_or_else(|| mapping_error("aoi_longitude"))?;
            let latitude = position[1]
                .as_f64()
                .filter(|value| value.is_finite() && (-90.0..=90.0).contains(value))
                .ok_or_else(|| mapping_error("aoi_latitude"))?;
            Ok([longitude, latitude])
        })
        .collect::<ProviderResult<Vec<_>>>()?;
    if points.first() != points.last() {
        return Err(mapping_error("aoi_ring_closure"));
    }
    points.pop();
    if points.len() < 3 || points.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(mapping_error("aoi_ring"));
    }
    let forward = minimum_rotation(&points);
    let mut reversed = points;
    reversed.reverse();
    let reversed = minimum_rotation(&reversed);
    let mut canonical = if compare_points(&forward, &reversed) == Ordering::Greater {
        reversed
    } else {
        forward
    };
    canonical.push(canonical[0]);
    Ok(Value::Array(
        canonical
            .into_iter()
            .map(|point| json!([point[0], point[1]]))
            .collect(),
    ))
}

fn minimum_rotation(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    (0..points.len())
        .map(|start| {
            points[start..]
                .iter()
                .chain(&points[..start])
                .copied()
                .collect::<Vec<_>>()
        })
        .min_by(|left, right| compare_points(left, right))
        .expect("validated ring has points")
}

fn compare_points(left: &[[f64; 2]], right: &[[f64; 2]]) -> Ordering {
    left.iter()
        .zip(right)
        .find_map(|(left, right)| {
            match left[0]
                .partial_cmp(&right[0])
                .unwrap_or(Ordering::Equal)
                .then_with(|| left[1].partial_cmp(&right[1]).unwrap_or(Ordering::Equal))
            {
                Ordering::Equal => None,
                other => Some(other),
            }
        })
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn parse_geometry(value: &Map<String, Value>) -> ProviderResult<MultiPolygon<f64>> {
    let geojson = GeoJsonGeometry::from_json_value(Value::Object(value.clone()))
        .map_err(|_| mapping_error("geojson"))?;
    let geometry = Geometry::try_from(geojson.value).map_err(|_| mapping_error("geojson"))?;
    match geometry {
        Geometry::Polygon(polygon) => validate_polygons(vec![polygon]),
        Geometry::MultiPolygon(polygons) => validate_polygons(polygons.0),
        _ => Err(mapping_error("geojson_type")),
    }
}

fn validate_polygons(polygons: Vec<Polygon<f64>>) -> ProviderResult<MultiPolygon<f64>> {
    if polygons.is_empty()
        || polygons
            .iter()
            .any(|polygon| !polygon.unsigned_area().is_finite() || polygon.unsigned_area() <= 0.0)
    {
        return Err(mapping_error("geojson_polygon"));
    }
    Ok(MultiPolygon(polygons))
}

/// Real metadata + hardened geometry -> immutable evidence.
///
/// This is the production entry point for constructing
/// `NisarSubsetGeolocationEvidenceV1` from the actual sparse geolocation grid
/// (`coordinateX`/`coordinateY`/`heightAboveEllipsoid` node axes) and the
/// actual science dataset shape/chunk. It executes the canonical
/// `sparse_grid_polygon_cells_v1` algorithm (single implementation promoted
/// from the historical lab) instead of a simplified production approximation.
#[allow(
    clippy::too_many_arguments,
    reason = "one bounded real-geolocation evidence constructor; grouping into a struct would churn the reviewed planner contract"
)]
pub fn build_live_geolocation_evidence(
    aoi_geojson: &Map<String, Value>,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    swath: &str,
    grid_zero_doppler_time: &[f64],
    grid_slant_range: &[f64],
    swath_zero_doppler_time: &[f64],
    swath_slant_range: &[f64],
    height_layers_m: Vec<f64>,
    longitudes: Vec<Vec<f64>>,
    latitudes: Vec<Vec<f64>>,
    source_shape: [usize; 2],
    chunk_shape: [usize; 2],
    minimum_hae_m: f64,
    maximum_hae_m: f64,
) -> ProviderResult<NisarSubsetGeolocationEvidenceV1> {
    let (product_specification_version, crid) =
        live_geolocation_product_profile(&frequency, polarization, swath)?;
    if grid_zero_doppler_time.len() < 2
        || grid_slant_range.len() < 2
        || swath_zero_doppler_time.is_empty()
        || swath_slant_range.is_empty()
    {
        return Err(mapping_error("grid"));
    }
    let (node_rows, row_diagnostics) =
        nearest_radar_axis_indices(grid_zero_doppler_time, swath_zero_doppler_time)
            .ok_or_else(|| mapping_error("grid_nodes"))?;
    let (node_columns, column_diagnostics) =
        nearest_radar_axis_indices(grid_slant_range, swath_slant_range)
            .ok_or_else(|| mapping_error("grid_nodes"))?;
    let longitudes = rebind_grid_coordinates(
        longitudes,
        grid_zero_doppler_time.len(),
        grid_slant_range.len(),
        row_diagnostics.kept_first_grid_index,
        row_diagnostics.kept_last_grid_index,
        column_diagnostics.kept_first_grid_index,
        column_diagnostics.kept_last_grid_index,
    )
    .ok_or_else(|| mapping_error("grid_construction"))?;
    let latitudes = rebind_grid_coordinates(
        latitudes,
        grid_zero_doppler_time.len(),
        grid_slant_range.len(),
        row_diagnostics.kept_first_grid_index,
        row_diagnostics.kept_last_grid_index,
        column_diagnostics.kept_first_grid_index,
        column_diagnostics.kept_last_grid_index,
    )
    .ok_or_else(|| mapping_error("grid_construction"))?;
    let grid = SparseGeoGrid::new(
        node_rows,
        node_columns,
        height_layers_m,
        longitudes,
        latitudes,
        source_shape,
        chunk_shape,
    )
    .map_err(|_| mapping_error("grid_construction"))?;
    let aoi = normalize_nisar_subset_aoi(aoi_geojson)?;
    let aoi_text = serde_json::to_string(aoi_geojson).map_err(|_| mapping_error("aoi"))?;
    let selection =
        sparse_grid::sparse_grid_polygon_cells_v1(&aoi_text, &grid, minimum_hae_m, maximum_hae_m)
            .map_err(sparse_grid_mapping_error)?;

    let mut windows = Vec::new();
    for component in &selection.components {
        let window = component.chunk_window;
        windows.push(NisarSubsetEvidenceWindowV1 {
            component_id: u16::try_from(component.geometry_part_index)
                .map_err(|_| mapping_error("component"))?,
            line_start: window.row_start as u64,
            line_end_exclusive: window.row_end as u64,
            sample_start: window.column_start as u64,
            sample_end_exclusive: window.column_end as u64,
            margin_lines: 0,
            margin_samples: 0,
        });
    }
    if windows.is_empty() {
        return Err(mapping_error("geometry"));
    }
    windows.sort_by(compare_evidence_windows);

    let source_shape_u64 = [
        u64::try_from(source_shape[0]).map_err(|_| mapping_error("shape"))?,
        u64::try_from(source_shape[1]).map_err(|_| mapping_error("shape"))?,
    ];
    let chunk_shape_u64 = [
        u64::try_from(chunk_shape[0]).map_err(|_| mapping_error("shape"))?,
        u64::try_from(chunk_shape[1]).map_err(|_| mapping_error("shape"))?,
    ];
    let logical_window_cells: u64 = windows
        .iter()
        .map(|window| {
            (window.line_end_exclusive - window.line_start)
                .saturating_mul(window.sample_end_exclusive - window.sample_start)
        })
        .sum();
    // Conservative non-zero placeholder estimate used only for evidence
    // validation. The download layer recomputes the real remote/output byte
    // totals from the actual compressed chunk layout before transfer.
    let placeholder_remote_bytes = logical_window_cells.saturating_mul(8).max(1);
    let evidence = NisarSubsetGeolocationEvidenceV1 {
        aoi_sha256: aoi.sha256,
        algorithm: sparse_grid::SPARSE_GRID_POLYGON_CELLS_V1.to_owned(),
        algorithm_version: 1,
        evidence_digest: String::new(),
        product_specification_version: product_specification_version.to_owned(),
        crid: crid.to_owned(),
        frequency: frequency.clone(),
        swath: swath.to_owned(),
        source_shape: source_shape_u64,
        chunk_shape: chunk_shape_u64,
        dtype: "CFloat32".to_owned(),
        windows,
        estimated_remote_bytes: placeholder_remote_bytes,
        estimated_output_bytes: placeholder_remote_bytes,
    };
    let digest_seed = serde_json::to_vec(&(
        &evidence.aoi_sha256,
        &evidence.algorithm,
        &evidence.algorithm_version,
        &evidence.product_specification_version,
        &evidence.crid,
        &frequency,
        polarization,
        swath,
        &evidence.source_shape,
        &evidence.chunk_shape,
        &evidence.dtype,
        &evidence.windows,
    ))
    .map_err(|_| mapping_error("evidence"))?;
    let mut evidence = evidence;
    evidence.evidence_digest = sha256(&digest_seed);
    Ok(evidence)
}

/// Map a hardened-geometry window back to a radar `Window2D` for qualification.
pub fn evidence_window_to_window2d(window: &NisarSubsetEvidenceWindowV1) -> Window2D {
    Window2D {
        row_start: window.line_start as usize,
        row_end: window.line_end_exclusive as usize,
        column_start: window.sample_start as usize,
        column_end: window.sample_end_exclusive as usize,
    }
}

fn compare_evidence_windows(
    left: &NisarSubsetEvidenceWindowV1,
    right: &NisarSubsetEvidenceWindowV1,
) -> Ordering {
    (
        left.component_id,
        left.line_start,
        left.line_end_exclusive,
        left.sample_start,
        left.sample_end_exclusive,
        left.margin_lines,
        left.margin_samples,
    )
        .cmp(&(
            right.component_id,
            right.line_start,
            right.line_end_exclusive,
            right.sample_start,
            right.sample_end_exclusive,
            right.margin_lines,
            right.margin_samples,
        ))
}

#[cfg(test)]
mod live_geolocation_profile_tests {
    use super::{
        build_live_geolocation_evidence, live_geolocation_product_profile,
        supported_ssar_dataset_polarizations, NISAR_SSAR_P00500_CRID,
        NISAR_SSAR_P00500_PRODUCT_SPECIFICATION_VERSION,
    };
    use crate::catalog::{NISAR_P05023_CRID, NISAR_P05023_PRODUCT_SPECIFICATION_VERSION};
    use insar_core::NisarSubsetFrequency;
    use serde_json::{Map, Value};

    #[test]
    fn live_geolocation_profile_is_closed_to_reviewed_lsar_and_ssar_semantics() {
        assert_eq!(
            live_geolocation_product_profile(&NisarSubsetFrequency::A, "HH", "LSAR_RSLC")
                .expect("reviewed L-band selection"),
            (
                NISAR_P05023_PRODUCT_SPECIFICATION_VERSION,
                NISAR_P05023_CRID
            )
        );
        for polarization in supported_ssar_dataset_polarizations() {
            assert_eq!(
                live_geolocation_product_profile(
                    &NisarSubsetFrequency::A,
                    polarization,
                    "SSAR_RSLC"
                )
                .expect("reviewed S-band dataset polarization"),
                (
                    NISAR_SSAR_P00500_PRODUCT_SPECIFICATION_VERSION,
                    NISAR_SSAR_P00500_CRID
                )
            );
        }
        assert!(
            live_geolocation_product_profile(&NisarSubsetFrequency::B, "RH", "SSAR_RSLC").is_err()
        );
        assert!(
            live_geolocation_product_profile(&NisarSubsetFrequency::A, "XX", "SSAR_RSLC").is_err()
        );
        assert!(
            live_geolocation_product_profile(&NisarSubsetFrequency::A, "RH", "UNKNOWN_RSLC")
                .is_err()
        );
    }

    #[test]
    fn live_geolocation_preserves_safe_sparse_grid_failure_reason() {
        let aoi = Map::from_iter([
            ("type".to_owned(), Value::String("Polygon".to_owned())),
            (
                "coordinates".to_owned(),
                serde_json::json!([[
                    [50.0, 70.0],
                    [50.1, 70.0],
                    [50.1, 70.1],
                    [50.0, 70.1],
                    [50.0, 70.0]
                ]]),
            ),
        ]);

        let error = build_live_geolocation_evidence(
            &aoi,
            NisarSubsetFrequency::A,
            "HH",
            "LSAR_RSLC",
            &[0.0, 1.0],
            &[0.0, 1.0],
            &[0.0, 1.0],
            &[0.0, 1.0],
            vec![0.0],
            vec![vec![100.0, 101.0, 100.0, 101.0]],
            vec![vec![30.0, 30.0, 31.0, 31.0]],
            [2, 2],
            [1, 1],
            0.0,
            0.0,
        )
        .expect_err("non-overlap must remain a typed mapping failure");

        assert_eq!(error.code, "NISAR_SUBSET_MAPPING_UNAVAILABLE");
        assert_eq!(
            error.debug_context.get("reason").and_then(Value::as_str),
            Some("no_overlap")
        );
    }
}

/// Scientific diagnostics for one geolocation-grid axis bound to the radar
/// swath axis by actual value comparison.
///
/// This is evidence output only; it never participates in the evidence
/// digest (the mapped indices do, through the geometry windows).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarAxisMappingV1 {
    pub grid_length: usize,
    pub full_length: usize,
    pub grid_first: f64,
    pub grid_last: f64,
    pub full_first: f64,
    pub full_last: f64,
    pub grid_increasing: bool,
    pub full_increasing: bool,
    pub median_full_spacing: Option<f64>,
    pub max_nearest_residual: f64,
    pub mapped_first_index: usize,
    pub mapped_last_index: usize,
    pub strictly_increasing: bool,
    /// First kept grid node index (0-based into the original grid axis).
    pub kept_first_grid_index: usize,
    /// Last kept grid node index (inclusive, 0-based into the original grid
    /// axis).
    pub kept_last_grid_index: usize,
    /// Grid nodes dropped because their axis value lies outside the radar
    /// axis span (scene margin nodes with no valid radar coordinate).
    pub margin_nodes_dropped: usize,
}

/// Map every geolocation-grid axis coordinate to the nearest radar-axis
/// coordinate using the ACTUAL axis values (binary search, O(log n) per grid
/// node). The mapping is accepted only when:
///
/// - both axes are finite and strictly monotonic in the same direction;
/// - the mapped radar indices are strictly increasing (no duplicate collapse,
///   which would mean the grid is denser than the radar axis);
/// - every mapped index stays inside the full-axis bounds.
///
/// The maximum numerical residual is recorded so qualification evidence can
/// compare it against the local radar-axis spacing instead of inventing an
/// arbitrary physical tolerance.
pub fn nearest_radar_axis_indices(
    grid_values: &[f64],
    full_values: &[f64],
) -> Option<(Vec<usize>, RadarAxisMappingV1)> {
    let grid_length = grid_values.len();
    let full_length = full_values.len();
    if grid_length < 2
        || full_length < 2
        || !grid_values.iter().all(|value| value.is_finite())
        || !full_values.iter().all(|value| value.is_finite())
    {
        return None;
    }
    let grid_increasing = grid_values.windows(2).all(|pair| pair[0] < pair[1]);
    let grid_decreasing = grid_values.windows(2).all(|pair| pair[0] > pair[1]);
    let full_increasing = full_values.windows(2).all(|pair| pair[0] < pair[1]);
    let full_decreasing = full_values.windows(2).all(|pair| pair[0] > pair[1]);
    if (!grid_increasing && !grid_decreasing) || (!full_increasing && !full_decreasing) {
        return None;
    }
    if grid_increasing != full_increasing {
        return None;
    }
    let mapped = grid_values
        .iter()
        .map(|value| nearest_axis_index(full_values, full_increasing, *value))
        .collect::<Vec<_>>();
    // The geolocation-grid axis is a superset of the radar swath axis: it
    // extends roughly half a grid cell beyond the radar axis at both ends
    // (scene margin nodes). Those out-of-span grid nodes have no valid radar
    // coordinate; nearest matching would collapse them onto the boundary
    // index. The value-proven mapping therefore keeps only the in-span
    // sub-range and records the dropped margin nodes as diagnostics. Any
    // duplicate collapse inside the kept sub-range (a grid denser than the
    // radar axis) still fails closed.
    let span_lo = full_values[0].min(full_values[full_length - 1]);
    let span_hi = full_values[0].max(full_values[full_length - 1]);
    let first_kept = if grid_increasing {
        grid_values.iter().position(|value| *value >= span_lo)?
    } else {
        grid_values.iter().position(|value| *value <= span_hi)?
    };
    let last_kept = if grid_increasing {
        grid_values.iter().rposition(|value| *value <= span_hi)?
    } else {
        grid_values.iter().rposition(|value| *value >= span_lo)?
    };
    if last_kept < first_kept || last_kept - first_kept + 1 < 2 {
        return None;
    }
    let kept = &mapped[first_kept..=last_kept];
    if kept.windows(2).any(|pair| pair[0] >= pair[1]) {
        return None;
    }
    let strictly_increasing = true;
    let max_nearest_residual = grid_values[first_kept..=last_kept]
        .iter()
        .zip(kept)
        .map(|(value, index)| (value - full_values[*index]).abs())
        .fold(0.0_f64, f64::max);
    let diagnostics = RadarAxisMappingV1 {
        grid_length,
        full_length,
        grid_first: grid_values[0],
        grid_last: grid_values[grid_length - 1],
        full_first: full_values[0],
        full_last: full_values[full_length - 1],
        grid_increasing,
        full_increasing,
        median_full_spacing: median_abs_spacing(full_values),
        max_nearest_residual,
        mapped_first_index: kept[0],
        mapped_last_index: kept[kept.len() - 1],
        strictly_increasing,
        kept_first_grid_index: first_kept,
        kept_last_grid_index: last_kept,
        margin_nodes_dropped: grid_length - (last_kept - first_kept + 1),
    };
    Some((kept.to_vec(), diagnostics))
}

/// Rebind the per-height-layer geolocation coordinate arrays to the kept
/// in-scene grid-node sub-lattice (`kept lines x kept samples`) after
/// value-based radar-axis binding trimmed the scene-margin nodes.
fn rebind_grid_coordinates(
    layers: Vec<Vec<f64>>,
    grid_lines: usize,
    grid_samples: usize,
    first_line: usize,
    last_line: usize,
    first_sample: usize,
    last_sample: usize,
) -> Option<Vec<Vec<f64>>> {
    if grid_lines == 0 || grid_samples == 0 || last_line < first_line || last_sample < first_sample
    {
        return None;
    }
    if last_line >= grid_lines || last_sample >= grid_samples {
        return None;
    }
    let layer_len = grid_lines.checked_mul(grid_samples)?;
    let kept_lines = last_line.checked_sub(first_line)?.checked_add(1)?;
    let kept_samples = last_sample.checked_sub(first_sample)?.checked_add(1)?;
    let kept_count = kept_lines.checked_mul(kept_samples)?;
    let mut rebound = Vec::with_capacity(layers.len());
    for layer in layers {
        if layer.len() != layer_len {
            return None;
        }
        let mut kept = Vec::with_capacity(kept_count);
        for line in first_line..=last_line {
            let base = line.checked_mul(grid_samples)?;
            for sample in first_sample..=last_sample {
                kept.push(layer[base + sample]);
            }
        }
        rebound.push(kept);
    }
    Some(rebound)
}

fn nearest_axis_index(full_values: &[f64], increasing: bool, value: f64) -> usize {
    if increasing {
        let first_ge = full_values.partition_point(|candidate| *candidate < value);
        match first_ge {
            0 => 0,
            index if index >= full_values.len() => full_values.len() - 1,
            index => {
                if value - full_values[index - 1] <= full_values[index] - value {
                    index - 1
                } else {
                    index
                }
            }
        }
    } else {
        let first_le = full_values.partition_point(|candidate| *candidate > value);
        match first_le {
            0 => 0,
            index if index >= full_values.len() => full_values.len() - 1,
            index => {
                if full_values[index - 1] - value <= value - full_values[index] {
                    index - 1
                } else {
                    index
                }
            }
        }
    }
}

fn median_abs_spacing(values: &[f64]) -> Option<f64> {
    let mut spacing = values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .collect::<Vec<_>>();
    if spacing.is_empty() {
        return None;
    }
    spacing.sort_by(|left, right| left.total_cmp(right));
    let middle = spacing.len() / 2;
    if spacing.len().is_multiple_of(2) {
        Some((spacing[middle - 1] + spacing[middle]) / 2.0)
    } else {
        Some(spacing[middle])
    }
}

fn snapshot_checksum_v2(item: &SnapshotItem) -> ProviderResult<Option<NisarSubsetChecksumV1>> {
    let asset = item
        .asset_access
        .as_ref()
        .ok_or_else(|| not_supported("primary_asset"))?;
    match (
        asset.checksum_algorithm.as_deref(),
        asset.checksum_value.as_ref(),
    ) {
        (None, None) => Ok(None),
        (Some(algorithm), Some(value)) => {
            let algorithm = if algorithm.eq_ignore_ascii_case("md5") {
                NisarSubsetChecksumAlgorithm::Md5
            } else if algorithm.eq_ignore_ascii_case("sha1") {
                NisarSubsetChecksumAlgorithm::Sha1
            } else if algorithm.eq_ignore_ascii_case("sha256") {
                NisarSubsetChecksumAlgorithm::Sha256
            } else {
                return Err(not_supported("source_checksum_algorithm"));
            };
            Ok(Some(NisarSubsetChecksumV1 {
                algorithm,
                value: value.clone(),
            }))
        }
        _ => Err(not_supported("source_checksum")),
    }
}

fn source_capability_v2(item: &SnapshotItem) -> ProviderResult<NisarSubsetSourceCapabilityV2> {
    let asset = item
        .asset_access
        .as_ref()
        .ok_or_else(|| not_supported("primary_asset"))?;
    let mut frequencies = Vec::new();
    for (frequency, key) in [
        (NisarSubsetFrequency::A, "frequency_a_polarization"),
        (NisarSubsetFrequency::B, "frequency_b_polarization"),
    ] {
        let Some(label) = metadata_text(item, key) else {
            continue;
        };
        let polarizations = canonical_nisar_polarizations_for_frequency_label(&frequency, label)
            .ok_or_else(|| not_supported("source_polarization_capability"))?;
        frequencies.push(NisarSubsetFrequencyCapabilityV2 {
            frequency,
            available_polarizations: polarizations,
        });
    }
    if frequencies.is_empty() {
        return Err(not_supported("frequency_presence"));
    }
    let evidence = json!({
        "provider_product_key": &asset.reference.provider_product_key,
        "stable_asset_key": &asset.reference.stable_asset_key,
        "quality_status": "PROVISIONAL",
        "frequencies": &frequencies,
    });
    Ok(NisarSubsetSourceCapabilityV2 {
        evidence_sha256: sha256(canonical_value(&evidence).as_bytes()),
        frequencies,
    })
}

fn plan_v2_error(error: NisarSubsetPlanV2Error) -> ContractError {
    match error {
        NisarSubsetPlanV2Error::AntimeridianUnsupported { geometry } => {
            let reason = match geometry {
                insar_core::NisarSubsetGeometryRoleV2::SourceFootprint => {
                    "source_footprint_antimeridian"
                }
                insar_core::NisarSubsetGeometryRoleV2::Requested => "requested_antimeridian",
                insar_core::NisarSubsetGeometryRoleV2::Effective => "effective_antimeridian",
            };
            planner_error(
                "NISAR_SUBSET_ANTIMERIDIAN_UNSUPPORTED",
                "当前 NISAR AOI 子集不支持跨反子午线几何",
                reason,
            )
        }
        NisarSubsetPlanV2Error::EmptyEffectiveGeometry => planner_error(
            "NISAR_SUBSET_EFFECTIVE_GEOMETRY_EMPTY",
            "请求几何与源产品覆盖范围没有有效面交集",
            "empty_effective_geometry",
        ),
        NisarSubsetPlanV2Error::DependencyClosureInvalid(_)
        | NisarSubsetPlanV2Error::DependencyGraphInvalid(_) => planner_error(
            "NISAR_SUBSET_DEPENDENCY_CLOSURE_INVALID",
            "NISAR 子集科学依赖闭包不完整或不可识别",
            "dependency_closure",
        ),
        NisarSubsetPlanV2Error::PolarizationInvalid(_) => planner_error(
            "NISAR_SUBSET_POLARIZATION_UNSUPPORTED",
            "所选 NISAR 极化不在冻结源产品的实际能力中",
            "selected_polarization",
        ),
        NisarSubsetPlanV2Error::SourceCapabilityInvalid(_) => {
            not_supported("source_polarization_capability")
        }
        _ => planner_error(
            "NISAR_SUBSET_PLAN_V2_INVALID",
            "NISAR 子集计划无法形成确定性裁剪契约",
            "plan_v2_contract",
        ),
    }
}

fn canonical_value(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("normalized AOI values serialize")
}

fn metadata_text<'a>(item: &'a SnapshotItem, key: &str) -> Option<&'a str> {
    item.metadata.get(key).and_then(Value::as_str)
}

fn frequency_name(frequency: &NisarSubsetFrequency) -> &'static str {
    match frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn not_supported(reason: &'static str) -> ContractError {
    planner_error(
        "NISAR_SUBSET_NOT_SUPPORTED",
        "该冻结 NISAR 产品不支持离线远程裁剪规划",
        reason,
    )
}

fn mapping_error(reason: &'static str) -> ContractError {
    planner_error(
        "NISAR_SUBSET_MAPPING_UNAVAILABLE",
        "AOI 与 NISAR 雷达网格的离线映射证据不足",
        reason,
    )
}

fn sparse_grid_mapping_error(error: ContractError) -> ContractError {
    let reason = match error.debug_context.get("reason").and_then(Value::as_str) {
        Some("candidate_budget") => "candidate_budget",
        Some("height_envelope") => "height_envelope",
        Some("invalid_geometry") => "invalid_geometry",
        Some("invalid_grid") => "invalid_grid",
        Some("no_overlap") => "no_overlap",
        _ => "geometry",
    };
    mapping_error(reason)
}

fn planner_error(code: &'static str, message: &'static str, reason: &'static str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded NISAR subset planner error is safe")
}
