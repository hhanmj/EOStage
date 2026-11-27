use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{Map, Value};

use crate::{canonical_utc_identity_timestamp, StoreError, TaskStore};

const MAX_GEOMETRY_JSON_BYTES: usize =
    insar_core::polygonal_topology::MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES;
const MAX_DISPLAY_NAME_CHARS: usize = 512;
const BBOX_TOLERANCE: f64 = 1.0e-9;
const SOURCE_KINDS: [&str; 4] = [
    "manual_bbox",
    "manual_polygon",
    "uploaded_geometry",
    "administrative_boundary",
];

#[derive(Clone, Debug, PartialEq)]
pub struct StoreStandaloneAoiInput {
    pub expected_revision: Option<u64>,
    pub source_kind: String,
    pub geometry: Map<String, Value>,
    pub bbox: Vec<f64>,
    pub crs: String,
    pub display_name: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoreStandaloneAoi {
    pub revision: u64,
    pub source_kind: String,
    pub geometry: Map<String, Value>,
    pub bbox: Vec<f64>,
    pub crs: String,
    pub display_name: String,
    pub updated_at: String,
}

#[derive(Debug)]
struct ValidatedStandaloneAoi {
    source_kind: String,
    geometry_json: String,
    geometry: Map<String, Value>,
    bbox: [f64; 4],
    display_name: String,
    updated_at: String,
}

impl TaskStore {
    pub fn load_standalone_aoi(&self) -> Result<Option<StoreStandaloneAoi>, StoreError> {
        load_from_connection(&self.connection)
    }

    pub fn save_standalone_aoi(
        &mut self,
        input: &StoreStandaloneAoiInput,
    ) -> Result<StoreStandaloneAoi, StoreError> {
        let validated = validate_input(input)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_revision = transaction
            .query_row(
                "SELECT revision FROM standalone_aoi_state WHERE slot = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let revision = match (existing_revision, input.expected_revision) {
            (None, None) => 0_i64,
            (Some(current), Some(expected)) if u64::try_from(current).ok() == Some(expected) => {
                current.checked_add(1).ok_or(StoreError::RevisionConflict)?
            }
            _ => return Err(StoreError::RevisionConflict),
        };
        transaction.execute(
            "INSERT INTO standalone_aoi_state( \
                 slot, revision, source_kind, geometry_json, west, south, east, north, crs, display_name, updated_at \
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'EPSG:4326', ?8, ?9) \
             ON CONFLICT(slot) DO UPDATE SET \
                 revision = excluded.revision, source_kind = excluded.source_kind, \
                 geometry_json = excluded.geometry_json, west = excluded.west, south = excluded.south, \
                 east = excluded.east, north = excluded.north, crs = excluded.crs, \
                 display_name = excluded.display_name, updated_at = excluded.updated_at",
            params![
                revision,
                validated.source_kind,
                validated.geometry_json,
                validated.bbox[0],
                validated.bbox[1],
                validated.bbox[2],
                validated.bbox[3],
                validated.display_name,
                validated.updated_at,
            ],
        )?;
        transaction.commit()?;
        Ok(StoreStandaloneAoi {
            revision: u64::try_from(revision)
                .map_err(|_| StoreError::invalid("standalone AOI revision is invalid"))?,
            source_kind: validated.source_kind,
            geometry: validated.geometry,
            bbox: validated.bbox.to_vec(),
            crs: "EPSG:4326".to_owned(),
            display_name: validated.display_name,
            updated_at: validated.updated_at,
        })
    }

    pub fn clear_standalone_aoi(
        &mut self,
        expected_revision: Option<u64>,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_revision = transaction
            .query_row(
                "SELECT revision FROM standalone_aoi_state WHERE slot = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        match (existing_revision, expected_revision) {
            (None, None) => {}
            (Some(current), Some(expected)) if u64::try_from(current).ok() == Some(expected) => {
                transaction.execute("DELETE FROM standalone_aoi_state WHERE slot = 1", [])?;
            }
            _ => return Err(StoreError::RevisionConflict),
        }
        transaction.commit()?;
        Ok(())
    }
}

fn load_from_connection(
    connection: &rusqlite::Connection,
) -> Result<Option<StoreStandaloneAoi>, StoreError> {
    let row = connection
        .query_row(
            "SELECT revision, source_kind, geometry_json, west, south, east, north, crs, display_name, updated_at \
             FROM standalone_aoi_state WHERE slot = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        revision,
        source_kind,
        geometry_json,
        west,
        south,
        east,
        north,
        crs,
        display_name,
        updated_at,
    )) = row
    else {
        return Ok(None);
    };
    let geometry = serde_json::from_str::<Value>(&geometry_json)
        .map_err(StoreError::Corrupt)?
        .as_object()
        .cloned()
        .ok_or_else(|| StoreError::invalid("standalone AOI geometry must be an object"))?;
    let input = StoreStandaloneAoiInput {
        expected_revision: None,
        source_kind,
        geometry,
        bbox: vec![west, south, east, north],
        crs,
        display_name,
        updated_at,
    };
    let validated = validate_input(&input)?;
    Ok(Some(StoreStandaloneAoi {
        revision: u64::try_from(revision)
            .map_err(|_| StoreError::invalid("standalone AOI revision is invalid"))?,
        source_kind: validated.source_kind,
        geometry: validated.geometry,
        bbox: validated.bbox.to_vec(),
        crs: "EPSG:4326".to_owned(),
        display_name: validated.display_name,
        updated_at: validated.updated_at,
    }))
}

fn validate_input(input: &StoreStandaloneAoiInput) -> Result<ValidatedStandaloneAoi, StoreError> {
    if !SOURCE_KINDS.contains(&input.source_kind.as_str()) {
        return Err(StoreError::invalid("standalone AOI source kind is invalid"));
    }
    if input.crs != "EPSG:4326" {
        return Err(StoreError::invalid("standalone AOI CRS must be EPSG:4326"));
    }
    let display_name = input.display_name.trim();
    if display_name.is_empty()
        || display_name.chars().count() > MAX_DISPLAY_NAME_CHARS
        || display_name.chars().any(char::is_control)
    {
        return Err(StoreError::invalid(
            "standalone AOI display name is invalid",
        ));
    }
    let updated_at = canonical_utc_identity_timestamp(&input.updated_at)
        .filter(|canonical| canonical == &input.updated_at)
        .ok_or_else(|| StoreError::invalid("standalone AOI timestamp is invalid"))?;
    let authoritative = insar_core::validated_authoritative_polygonal_geojson(&input.geometry)
        .ok_or_else(|| StoreError::invalid("standalone AOI geometry is invalid"))?;
    let exact_bbox = authoritative.bbox;
    if input.bbox.len() != 4
        || input.bbox.iter().any(|coordinate| !coordinate.is_finite())
        || input
            .bbox
            .iter()
            .zip(exact_bbox.iter())
            .any(|(provided, exact)| (provided - exact).abs() > BBOX_TOLERANCE)
    {
        return Err(StoreError::invalid(
            "standalone AOI bbox does not match its geometry",
        ));
    }
    let geometry_json = serde_json::to_string(&Value::Object(authoritative.geojson.clone()))
        .map_err(StoreError::Corrupt)?;
    if geometry_json.len() > MAX_GEOMETRY_JSON_BYTES {
        return Err(StoreError::invalid("standalone AOI geometry is too large"));
    }
    Ok(ValidatedStandaloneAoi {
        source_kind: input.source_kind.clone(),
        geometry_json,
        geometry: authoritative.geojson,
        bbox: exact_bbox,
        display_name: display_name.to_owned(),
        updated_at,
    })
}
