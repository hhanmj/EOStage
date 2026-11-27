use insar_core::validated_authoritative_polygonal_geojson;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{canonical_utc_identity_timestamp, StoreError, TaskStore};

/// Default TTL (30 days) for cached administrative boundary polygons.
pub const ADMIN_BOUNDARY_CACHE_DEFAULT_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;
/// Maximum number of cached administrative boundaries retained in SQLite.
pub const ADMIN_BOUNDARY_CACHE_MAX_ENTRIES: usize = 256;
/// Maximum total geometry payload budget (64 MiB) for administrative boundary cache.
pub const ADMIN_BOUNDARY_CACHE_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredAdministrativeBoundary {
    pub provider: String,
    pub provider_object_id: String,
    pub name: String,
    pub display_name: String,
    pub country_code: Option<String>,
    pub admin_level: Option<String>,
    pub geometry: Map<String, Value>,
    pub bbox: [f64; 4],
    pub source: String,
    pub attribution: Option<String>,
    pub cached_at: String,
}

impl TaskStore {
    pub fn upsert_administrative_boundary(
        &mut self,
        boundary: &StoredAdministrativeBoundary,
    ) -> Result<(), StoreError> {
        let boundary = validated_boundary(boundary.clone())?;
        let geometry_json =
            serde_json::to_string(&boundary.geometry).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO administrative_boundary_cache (
                provider, provider_object_id, name, display_name, country_code,
                admin_level, geometry_json, west, south, east, north, source,
                attribution, cached_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
             )
             ON CONFLICT(provider, provider_object_id) DO UPDATE SET
                name = excluded.name,
                display_name = excluded.display_name,
                country_code = excluded.country_code,
                admin_level = excluded.admin_level,
                geometry_json = excluded.geometry_json,
                west = excluded.west,
                south = excluded.south,
                east = excluded.east,
                north = excluded.north,
                source = excluded.source,
                attribution = excluded.attribution,
                cached_at = excluded.cached_at",
            params![
                boundary.provider,
                boundary.provider_object_id,
                boundary.name,
                boundary.display_name,
                boundary.country_code,
                boundary.admin_level,
                geometry_json,
                boundary.bbox[0],
                boundary.bbox[1],
                boundary.bbox[2],
                boundary.bbox[3],
                boundary.source,
                boundary.attribution,
                boundary.cached_at,
            ],
        )?;
        prune_admin_boundary_cache_in_tx(
            &transaction,
            Some((&boundary.provider, &boundary.provider_object_id)),
            None,
            ADMIN_BOUNDARY_CACHE_MAX_ENTRIES,
            ADMIN_BOUNDARY_CACHE_MAX_BYTES,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn prune_administrative_boundary_cache_bounds(
        &mut self,
        cutoff_iso: Option<&str>,
        max_entries: usize,
        max_bytes: u64,
    ) -> Result<usize, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = prune_admin_boundary_cache_in_tx(
            &transaction,
            None,
            cutoff_iso,
            max_entries,
            max_bytes,
        )?;
        transaction.commit()?;
        Ok(removed)
    }

    pub fn load_administrative_boundary(
        &self,
        provider: &str,
        provider_object_id: &str,
    ) -> Result<Option<StoredAdministrativeBoundary>, StoreError> {
        validate_provider_identity(provider, provider_object_id)?;
        let row = self
            .connection
            .query_row(
                "SELECT name, display_name, country_code, admin_level,
                        geometry_json, west, south, east, north, source,
                        attribution, cached_at
                 FROM administrative_boundary_cache
                 WHERE provider = ?1 AND provider_object_id = ?2",
                params![provider, provider_object_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, f64>(5)?,
                        row.get::<_, f64>(6)?,
                        row.get::<_, f64>(7)?,
                        row.get::<_, f64>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            name,
            display_name,
            country_code,
            admin_level,
            geometry_json,
            west,
            south,
            east,
            north,
            source,
            attribution,
            cached_at,
        )) = row
        else {
            return Ok(None);
        };
        let geometry = serde_json::from_str(&geometry_json).map_err(StoreError::Corrupt)?;
        validated_boundary(StoredAdministrativeBoundary {
            provider: provider.to_owned(),
            provider_object_id: provider_object_id.to_owned(),
            name,
            display_name,
            country_code,
            admin_level,
            geometry,
            bbox: [west, south, east, north],
            source,
            attribution,
            cached_at,
        })
        .map(Some)
    }
}

fn prune_admin_boundary_cache_in_tx(
    transaction: &Transaction<'_>,
    protected_key: Option<(&str, &str)>,
    cutoff_iso: Option<&str>,
    max_entries: usize,
    max_bytes: u64,
) -> Result<usize, StoreError> {
    let mut removed: usize = 0;
    if let Some(cutoff) = cutoff_iso.map(str::trim).filter(|s| !s.is_empty()) {
        removed = removed.saturating_add(match protected_key {
            Some((prov, obj_id)) => transaction.execute(
                "DELETE FROM administrative_boundary_cache
                 WHERE cached_at < ?1 AND NOT (provider = ?2 AND provider_object_id = ?3)",
                params![cutoff, prov, obj_id],
            )?,
            None => transaction.execute(
                "DELETE FROM administrative_boundary_cache WHERE cached_at < ?1",
                params![cutoff],
            )?,
        });
    }

    let rows: Vec<(String, String, u64)> = {
        let mut stmt = transaction.prepare(
            "SELECT provider, provider_object_id, LENGTH(CAST(geometry_json AS BLOB))
             FROM administrative_boundary_cache
             ORDER BY cached_at DESC, provider DESC, provider_object_id DESC",
        )?;
        let mapped = stmt.query_map([], |row| {
            let provider: String = row.get(0)?;
            let object_id: String = row.get(1)?;
            let bytes: i64 = row.get(2)?;
            Ok((provider, object_id, u64::try_from(bytes).unwrap_or(0)))
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };

    let mut kept_entries: usize = 0;
    let mut kept_bytes: u64 = 0;
    let mut to_evict: Vec<(String, String)> = Vec::new();
    for (provider, object_id, bytes) in rows {
        if protected_key == Some((provider.as_str(), object_id.as_str())) {
            kept_entries = kept_entries.saturating_add(1);
            kept_bytes = kept_bytes.saturating_add(bytes);
            continue;
        }
        let next_entries = kept_entries.saturating_add(1);
        let next_bytes = kept_bytes.saturating_add(bytes);
        if next_entries <= max_entries && next_bytes <= max_bytes {
            kept_entries = next_entries;
            kept_bytes = next_bytes;
        } else {
            to_evict.push((provider, object_id));
        }
    }

    for (provider, object_id) in to_evict {
        removed = removed.saturating_add(transaction.execute(
            "DELETE FROM administrative_boundary_cache
             WHERE provider = ?1 AND provider_object_id = ?2",
            params![provider, object_id],
        )?);
    }
    Ok(removed)
}

fn validated_boundary(
    mut boundary: StoredAdministrativeBoundary,
) -> Result<StoredAdministrativeBoundary, StoreError> {
    validate_provider_identity(&boundary.provider, &boundary.provider_object_id)?;
    validate_required_text("boundary name", &boundary.name, 2_048)?;
    validate_required_text("boundary display name", &boundary.display_name, 2_048)?;
    validate_required_text("boundary source", &boundary.source, 2_048)?;
    validate_optional_text(
        "boundary attribution",
        boundary.attribution.as_deref(),
        2_048,
    )?;
    validate_optional_text("boundary admin level", boundary.admin_level.as_deref(), 128)?;
    if boundary.country_code.as_deref().is_some_and(|value| {
        value.len() != 2 || !value.bytes().all(|byte| byte.is_ascii_uppercase())
    }) {
        return Err(StoreError::invalid("boundary country code is invalid"));
    }
    if canonical_utc_identity_timestamp(&boundary.cached_at).as_deref()
        != Some(boundary.cached_at.as_str())
    {
        return Err(StoreError::invalid("boundary cache timestamp is invalid"));
    }
    let authoritative = validated_authoritative_polygonal_geojson(&boundary.geometry)
        .ok_or_else(|| StoreError::invalid("cached administrative boundary geometry is invalid"))?;
    if boundary
        .bbox
        .iter()
        .zip(authoritative.bbox.iter())
        .any(|(stored, derived)| stored.to_bits() != derived.to_bits())
    {
        return Err(StoreError::invalid(
            "cached administrative boundary bbox is not geometry-derived",
        ));
    }
    boundary.geometry = authoritative.geojson;
    boundary.bbox = authoritative.bbox;
    Ok(boundary)
}

fn validate_provider_identity(provider: &str, provider_object_id: &str) -> Result<(), StoreError> {
    if provider.trim() != provider || provider_object_id.trim() != provider_object_id {
        return Err(StoreError::invalid(
            "boundary cache identity is not canonical",
        ));
    }
    match provider {
        "datav"
            if provider_object_id.len() == 6
                && provider_object_id.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            Ok(())
        }
        "tianditu"
            if (provider_object_id.len() == 6 || provider_object_id.len() == 9)
                && provider_object_id.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            Ok(())
        }
        "nominatim" => {
            let (object_type, object_id) = provider_object_id
                .split_once(':')
                .ok_or_else(|| StoreError::invalid("OSM boundary identity is invalid"))?;
            if !matches!(object_type, "node" | "way" | "relation")
                || object_id.parse::<u64>().ok().is_none_or(|value| value == 0)
            {
                return Err(StoreError::invalid("OSM boundary identity is invalid"));
            }
            Ok(())
        }
        _ => Err(StoreError::invalid(
            "administrative boundary cache provider is unsupported",
        )),
    }
}

fn validate_required_text(label: &str, value: &str, max_chars: usize) -> Result<(), StoreError> {
    if value.trim() != value || value.is_empty() || value.chars().count() > max_chars {
        return Err(StoreError::invalid(format!("{label} is invalid")));
    }
    Ok(())
}

fn validate_optional_text(
    label: &str,
    value: Option<&str>,
    max_chars: usize,
) -> Result<(), StoreError> {
    if value.is_some_and(|value| {
        value.trim() != value || value.is_empty() || value.chars().count() > max_chars
    }) {
        return Err(StoreError::invalid(format!("{label} is invalid")));
    }
    Ok(())
}
