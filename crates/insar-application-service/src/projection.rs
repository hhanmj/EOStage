use insar_core::{
    align_polygonal_to_longitude_domain_render, align_polygonal_to_longitude_reference,
    validated_authoritative_polygonal_geojson, wrapped_aoi_envelope, RegionAoiState,
    StoreRestoreResponse, WrappedAoiEnvelope,
};
use insar_store::{StoreCoreRestore, StoreWorkbenchPage, StoreWorkbenchPageRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ServiceReadiness;

/// A canonical source geometry is bounded by Core at 8 MiB. A render
/// projection may contain two periodic copies, so the wire budget is doubled
/// with fixed envelope/item overhead. The UI parser mirrors this limit.
pub const MAX_RENDER_GEOMETRY_JSON_BYTES: usize = 17 * 1024 * 1024;
pub const MAX_PAGE_RENDER_JSON_BYTES: usize = 17 * 1024 * 1024;
pub const MAX_PROJECTED_RENDER_LONGITUDE: f64 = insar_core::MAX_PROJECTED_RENDER_LONGITUDE;

/// Canonical business payload shared by the JSON Lines and Tauri adapters.
/// Transport envelopes (request IDs, protocol numbers, and terminal flags) do
/// not appear here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadonlyServiceSnapshot {
    pub readiness: ServiceReadiness,
    pub restore: StoreRestoreResponse,
}

/// Canonical bounded startup/status payload. It carries the Store-owned
/// candidate-free CoreRestore read model; it never crosses candidate arrays or
/// selected product payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreServiceSnapshot {
    pub readiness: ServiceReadiness,
    pub core: StoreCoreRestore,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aoi_display: Option<CoreAoiDisplayAuthority>,
}

/// Native-derived, read-only render authority. It is computed from the
/// Store-owned AOI at the ApplicationService boundary and is never persisted
/// or accepted from the renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreAoiDisplayAuthority {
    pub region_id: String,
    pub revision: u64,
    pub envelope: WrappedAoiEnvelope,
    /// Native-aligned copies for the current AOI display domain. This is a
    /// render-only projection; persistence and request windows remain canonical.
    pub render_geometry: Map<String, Value>,
    pub render_bbox: DisplayBbox,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayBbox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl DisplayBbox {
    fn from_array(value: [f64; 4]) -> Option<Self> {
        let [west, south, east, north] = value;
        (west.is_finite()
            && south.is_finite()
            && east.is_finite()
            && north.is_finite()
            && west >= -MAX_PROJECTED_RENDER_LONGITUDE
            && east <= MAX_PROJECTED_RENDER_LONGITUDE
            && west < east
            && south < north
            && south >= -90.0
            && north <= 90.0)
            .then_some(Self {
                west,
                south,
                east,
                north,
            })
    }
}

impl CoreAoiDisplayAuthority {
    fn from_region_aoi(aoi: &RegionAoiState) -> Option<Self> {
        let geometry = validated_authoritative_polygonal_geojson(&aoi.geometry)?;
        let envelope = wrapped_aoi_envelope(&geometry.geometry)?;
        let (render_geometry, render_bbox) = align_polygonal_to_longitude_domain_render(
            &geometry.geometry,
            envelope.display_west,
            envelope.display_east,
        )?;
        let render_bbox = DisplayBbox::from_array(render_bbox)?;
        let serialized_bytes = serde_json::to_vec(&render_geometry).ok()?.len();
        if serialized_bytes > MAX_RENDER_GEOMETRY_JSON_BYTES {
            return None;
        }
        Some(Self {
            region_id: aoi.region_id.clone(),
            revision: aoi.revision,
            envelope,
            render_geometry,
            render_bbox,
        })
    }
}

fn bounded_render_copy(
    copies: insar_core::AlignedPolygonalCopies,
) -> Option<(Map<String, Value>, DisplayBbox)> {
    if !copies.longitudes_within_render_contract() {
        return None;
    }
    let render_geometry = copies.geojson()?;
    let serialized_bytes = serde_json::to_vec(&render_geometry).ok()?.len();
    if serialized_bytes > MAX_RENDER_GEOMETRY_JSON_BYTES {
        return None;
    }
    let render_bbox = DisplayBbox::from_array(copies.display_bbox()?)?;
    Some((render_geometry, render_bbox))
}

impl CoreServiceSnapshot {
    pub fn from_parts(readiness: ServiceReadiness, core: StoreCoreRestore) -> Self {
        let aoi_display = core
            .region_aoi
            .as_ref()
            .and_then(CoreAoiDisplayAuthority::from_region_aoi);
        Self {
            readiness,
            core,
            aoi_display,
        }
    }
}

/// Canonical bounded workbench page payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationWorkbenchPage {
    pub readiness: ServiceReadiness,
    pub page: StoreWorkbenchPage,
    /// Native render copies for this exact page and AOI revision. Absent in
    /// legacy payloads or when no persisted AOI authority is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<WorkbenchPageRenderAuthority>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchPageRenderAuthority {
    pub region_id: String,
    pub revision: u64,
    pub items: Vec<WorkbenchPageRenderItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchPageRenderItem {
    pub item_id: String,
    pub render_geometry: Option<Map<String, Value>>,
    pub render_bbox: Option<DisplayBbox>,
}

/// Derives a bounded render authority for one Store page. A failed native
/// projection is represented by null geometry for that item, so the UI hides
/// that footprint instead of drawing the canonical geometry in a wrong world.
pub fn derive_workbench_page_render(
    page: &StoreWorkbenchPage,
    aoi_display: Option<&CoreAoiDisplayAuthority>,
) -> Option<WorkbenchPageRenderAuthority> {
    let authority = aoi_display?;
    let items: Vec<WorkbenchPageRenderItem> = page
        .items
        .iter()
        .map(|entry| WorkbenchPageRenderItem {
            item_id: entry.item.item_id.clone(),
            render_geometry: None,
            render_bbox: None,
        })
        .collect();
    let mut render = WorkbenchPageRenderAuthority {
        region_id: authority.region_id.clone(),
        revision: authority.revision,
        items,
    };
    let mut payload_bytes = serde_json::to_vec(&render).ok()?.len();
    if payload_bytes > MAX_PAGE_RENDER_JSON_BYTES {
        return None;
    }
    for (index, entry) in page.items.iter().enumerate() {
        let projection = entry
            .item
            .footprint_geojson
            .as_ref()
            .and_then(validated_authoritative_polygonal_geojson)
            .and_then(|geometry| {
                align_polygonal_to_longitude_reference(
                    &geometry.geometry,
                    authority.envelope.display_west,
                    authority.envelope.display_east,
                )
            })
            .and_then(bounded_render_copy);
        if let Some((geometry, bbox)) = projection {
            let candidate = WorkbenchPageRenderItem {
                item_id: entry.item.item_id.clone(),
                render_geometry: Some(geometry),
                render_bbox: Some(bbox),
            };
            let current_bytes = serde_json::to_vec(&render.items[index]).ok()?;
            let candidate_bytes = serde_json::to_vec(&candidate).ok()?;
            let current_len = current_bytes.len();
            let candidate_len = candidate_bytes.len();
            let item_delta = candidate_len.abs_diff(current_len);
            let prospective_bytes = if candidate_len >= current_len {
                payload_bytes.checked_add(item_delta)?
            } else {
                payload_bytes.checked_sub(item_delta)?
            };
            if prospective_bytes <= MAX_PAGE_RENDER_JSON_BYTES {
                render.items[index] = candidate;
                payload_bytes = prospective_bytes;
            }
        }
    }
    let final_bytes = serde_json::to_vec(&render).ok()?;
    if final_bytes.len() > MAX_PAGE_RENDER_JSON_BYTES {
        return None;
    }
    debug_assert!(final_bytes.len() <= MAX_PAGE_RENDER_JSON_BYTES);
    Some(render)
}

/// Canonical workbench page request carried through the service boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationWorkbenchPageRequest {
    pub request: StoreWorkbenchPageRequest,
}

pub fn canonical_snapshot_value(
    snapshot: &ReadonlyServiceSnapshot,
) -> Result<Value, serde_json::Error> {
    serde_json::to_value(snapshot)
}

pub fn canonical_restore_value(
    restore: &StoreRestoreResponse,
) -> Result<Map<String, Value>, serde_json::Error> {
    match serde_json::to_value(restore)? {
        Value::Object(value) => Ok(value),
        _ => unreachable!("StoreRestoreResponse serializes to an object"),
    }
}

pub fn canonical_core_snapshot_value(
    snapshot: &CoreServiceSnapshot,
) -> Result<Value, serde_json::Error> {
    serde_json::to_value(snapshot)
}

pub fn canonical_workbench_page_value(
    page: &ApplicationWorkbenchPage,
) -> Result<Value, serde_json::Error> {
    serde_json::to_value(page)
}
