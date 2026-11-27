use std::process::ExitCode;

use insar_core::CatalogSearchQuery;
use insar_provider::asf::catalog_client::AsfCatalogClient;
use insar_provider::asf::query::build_alos2_cmr_query;
use insar_provider::boundary::DataVBoundaryProvider;
use insar_provider::catalog::{
    CollectionProfileRegistry, ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID,
    ALOS2_PROVIDER_ID,
};
use serde_json::{json, Map, Value};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ALOS2_COMPLEX_AOI_ACCEPTANCE_FAILED {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let boundary_provider =
        DataVBoundaryProvider::production().map_err(|error| error.to_string())?;
    let catalog = AsfCatalogClient::production().map_err(|error| error.to_string())?;
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .ok_or_else(|| "ALOS-2 profile missing".to_owned())?;

    let rectangle = json!({
        "type": "Polygon",
        "coordinates": [[[105.5, 29.0], [106.5, 29.0], [106.5, 30.0], [105.5, 30.0], [105.5, 29.0]]]
    });
    execute_case(
        &catalog,
        profile,
        "矩形",
        rectangle
            .as_object()
            .expect("rectangle geometry is an object")
            .clone(),
        30,
    )?;

    let mut national_geometry = None;
    for (label, adcode) in [
        ("重庆", "500000"),
        ("河北", "130000"),
        ("四川", "510000"),
        ("全国", "100000"),
    ] {
        let boundary = boundary_provider
            .resolve_exact_adcode(adcode)
            .map_err(|error| format!("{label} boundary: {error}"))?;
        let geometry = boundary
            .geometry
            .as_object()
            .ok_or_else(|| format!("{label} boundary is not polygonal GeoJSON"))?
            .clone();
        println!(
            "ALOS2_COMPLEX_AOI_BOUNDARY {}",
            serde_json::to_string(&json!({
                "label": label,
                "adcode": adcode,
                "geometry_type": boundary.geometry_kind,
                "bbox": boundary.bbox,
                "vertex_count": coordinate_vertex_count(boundary.geometry.get("coordinates")),
            }))
            .map_err(|error| error.to_string())?
        );
        execute_case(&catalog, profile, label, geometry.clone(), 30)?;
        if adcode == "100000" {
            national_geometry = Some(geometry);
        }
    }

    execute_case(
        &catalog,
        profile,
        "全国",
        national_geometry.ok_or_else(|| "national geometry missing".to_owned())?,
        200,
    )?;

    let zero_query = query(
        rectangle
            .as_object()
            .expect("rectangle geometry is an object")
            .clone(),
        30,
        object(
            json!({"datetime": {"start": "1970-01-01T00:00:00Z", "end": "1970-01-02T00:00:00Z"}}),
        ),
    );
    execute_query(&catalog, profile, "Zero hit", &zero_query)?;
    Ok(())
}

fn execute_case(
    catalog: &AsfCatalogClient,
    profile: &insar_provider::catalog::CollectionProfile,
    label: &str,
    geometry: Map<String, Value>,
    requested_limit: u64,
) -> Result<(), String> {
    execute_query(
        catalog,
        profile,
        label,
        &query(geometry, requested_limit, Map::new()),
    )
}

fn execute_query(
    catalog: &AsfCatalogClient,
    profile: &insar_provider::catalog::CollectionProfile,
    label: &str,
    query: &CatalogSearchQuery,
) -> Result<(), String> {
    let plan = build_alos2_cmr_query(query, profile)
        .map_err(|error| format!("{label} plan: {error:?}"))?;
    let response = catalog
        .search_alos2(query, profile, &|| false)
        .map_err(|error| format!("{label}: {error:?}"))?;
    println!(
        "ALOS2_COMPLEX_AOI_ACCEPTANCE {}",
        serde_json::to_string(&json!({
            "label": label,
            "geometry_type": query
                .aoi_geojson
                .as_ref()
                .and_then(|geometry| geometry.get("type"))
                .and_then(Value::as_str),
            "vertex_count": coordinate_vertex_count(
                query.aoi_geojson.as_ref().and_then(|geometry| geometry.get("coordinates"))
            ),
            "transport_mode": plan.spatial_transport_mode,
            "transport_parts": plan.transport_part_count,
            "stats": response.stats,
        }))
        .map_err(|error| error.to_string())?
    );
    Ok(())
}

fn query(
    aoi_geojson: Map<String, Value>,
    result_limit: u64,
    filters: Map<String, Value>,
) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(aoi_geojson),
        cursor: None,
        result_limit: Some(result_limit),
        filters,
    }
}

fn object(value: Value) -> Map<String, Value> {
    value
        .as_object()
        .expect("acceptance fixture is an object")
        .clone()
}

fn coordinate_vertex_count(value: Option<&Value>) -> usize {
    match value {
        Some(Value::Array(values))
            if values.len() >= 2
                && values.first().is_some_and(Value::is_number)
                && values.get(1).is_some_and(Value::is_number) =>
        {
            1
        }
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| coordinate_vertex_count(Some(value)))
            .sum(),
        _ => 0,
    }
}
