//! Separate backend contracts for deterministic China identity and Global discovery.
//!
//! These types are prepared for A2 integration. They do not change the current
//! production routing or UI selector in A1.

use crate::{ChinaAdminAuthority, ChinaAdminUnit};
use insar_core::validated_authoritative_polygonal_geojson;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdministrativeBoundaryContractErrorCode {
    InvalidRequest,
    AuthorityVersionMismatch,
    UnknownAdcode,
    IdentityMismatch,
    InvalidGeometry,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{code:?}: {detail}")]
pub struct AdministrativeBoundaryContractError {
    code: AdministrativeBoundaryContractErrorCode,
    detail: String,
}

impl AdministrativeBoundaryContractError {
    pub fn code(&self) -> AdministrativeBoundaryContractErrorCode {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

fn contract_error(
    code: AdministrativeBoundaryContractErrorCode,
    detail: impl Into<String>,
) -> AdministrativeBoundaryContractError {
    AdministrativeBoundaryContractError {
        code,
        detail: detail.into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminRootsRequest {
    pub authority_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminChildrenRequest {
    pub authority_version: String,
    pub parent_adcode: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminUnitRequest {
    pub authority_version: String,
    pub adcode: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminUnitListResult {
    pub authority_version: String,
    pub units: Vec<ChinaAdminUnit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminUnitResult {
    pub authority_version: String,
    pub unit: ChinaAdminUnit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminGeometryResolveRequest {
    pub authority_version: String,
    pub adcode: String,
}

impl ChinaAdminRootsRequest {
    pub fn validate(
        &self,
        authority: &ChinaAdminAuthority,
    ) -> Result<(), AdministrativeBoundaryContractError> {
        validate_authority_version(&self.authority_version, authority)
    }
}

impl ChinaAdminChildrenRequest {
    pub fn validate(
        &self,
        authority: &ChinaAdminAuthority,
    ) -> Result<(), AdministrativeBoundaryContractError> {
        validate_authority_version(&self.authority_version, authority)?;
        validate_known_adcode(&self.parent_adcode, authority)
    }
}

impl ChinaAdminUnitRequest {
    pub fn validate(
        &self,
        authority: &ChinaAdminAuthority,
    ) -> Result<(), AdministrativeBoundaryContractError> {
        validate_authority_version(&self.authority_version, authority)?;
        validate_known_adcode(&self.adcode, authority)
    }
}

impl ChinaAdminGeometryResolveRequest {
    pub fn validate(
        &self,
        authority: &ChinaAdminAuthority,
    ) -> Result<(), AdministrativeBoundaryContractError> {
        validate_authority_version(&self.authority_version, authority)?;
        validate_known_adcode(&self.adcode, authority)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChinaAdminGeometryResult {
    pub authority_version: String,
    pub adcode: String,
    pub resolver_id: String,
    pub geometry: Value,
    /// `[west, south, east, north]` in EPSG:4326.
    pub bbox: [f64; 4],
    pub source: String,
    pub attribution: Option<String>,
}

impl ChinaAdminGeometryResult {
    pub fn validate_against(
        &self,
        request: &ChinaAdminGeometryResolveRequest,
    ) -> Result<(), AdministrativeBoundaryContractError> {
        if self.authority_version != request.authority_version || self.adcode != request.adcode {
            return Err(contract_error(
                AdministrativeBoundaryContractErrorCode::IdentityMismatch,
                "China geometry result does not preserve authority_version/adcode",
            ));
        }
        validate_token("resolver_id", &self.resolver_id, 128)?;
        validate_token("source", &self.source, 256)?;
        validate_authoritative_geometry_bbox(&self.geometry, &self.bbox)
    }
}

pub trait ChinaAdminGeometryResolver: Send + Sync {
    fn resolve_china_admin_geometry(
        &self,
        request: &ChinaAdminGeometryResolveRequest,
    ) -> Result<ChinaAdminGeometryResult, AdministrativeBoundaryContractError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalBoundaryDiscoveryRequest {
    pub query: String,
    pub limit: usize,
}

impl GlobalBoundaryDiscoveryRequest {
    pub fn validate(&self) -> Result<(), AdministrativeBoundaryContractError> {
        validate_token("query", &self.query, 256)?;
        if !(1..=100).contains(&self.limit) {
            return Err(contract_error(
                AdministrativeBoundaryContractErrorCode::InvalidRequest,
                "global discovery limit must be between 1 and 100",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalBoundaryResolveRequest {
    pub provider: String,
    pub provider_object_id: String,
}

impl GlobalBoundaryResolveRequest {
    pub fn validate(&self) -> Result<(), AdministrativeBoundaryContractError> {
        validate_token("provider", &self.provider, 64)?;
        validate_token("provider_object_id", &self.provider_object_id, 256)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalBoundaryCandidate {
    pub provider: String,
    pub provider_object_id: String,
    pub display_name: String,
    /// Candidate focus only; never administrative topology.
    pub bbox: [f64; 4],
    pub source: String,
    pub attribution: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalBoundaryResolved {
    pub provider: String,
    pub provider_object_id: String,
    pub display_name: String,
    pub geometry: Value,
    pub bbox: [f64; 4],
    pub source: String,
    pub attribution: Option<String>,
}

impl GlobalBoundaryResolved {
    pub fn validate_against(
        &self,
        request: &GlobalBoundaryResolveRequest,
    ) -> Result<(), AdministrativeBoundaryContractError> {
        if self.provider != request.provider
            || self.provider_object_id != request.provider_object_id
        {
            return Err(contract_error(
                AdministrativeBoundaryContractErrorCode::IdentityMismatch,
                "Global resolved boundary does not preserve provider identity",
            ));
        }
        validate_token("display_name", &self.display_name, 512)?;
        validate_token("source", &self.source, 256)?;
        validate_authoritative_geometry_bbox(&self.geometry, &self.bbox)
    }
}

pub trait GlobalBoundaryDiscoveryResolver: Send + Sync {
    fn discover_global_boundaries(
        &self,
        request: &GlobalBoundaryDiscoveryRequest,
    ) -> Result<Vec<GlobalBoundaryCandidate>, AdministrativeBoundaryContractError>;

    fn resolve_global_boundary(
        &self,
        request: &GlobalBoundaryResolveRequest,
    ) -> Result<GlobalBoundaryResolved, AdministrativeBoundaryContractError>;
}

fn validate_authority_version(
    requested: &str,
    authority: &ChinaAdminAuthority,
) -> Result<(), AdministrativeBoundaryContractError> {
    validate_token("authority_version", requested, 128)?;
    if requested != authority.authority_version() {
        return Err(contract_error(
            AdministrativeBoundaryContractErrorCode::AuthorityVersionMismatch,
            format!(
                "requested authority_version {requested} does not equal {}",
                authority.authority_version()
            ),
        ));
    }
    Ok(())
}

fn validate_known_adcode(
    adcode: &str,
    authority: &ChinaAdminAuthority,
) -> Result<(), AdministrativeBoundaryContractError> {
    if adcode.len() != 6 || !adcode.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(contract_error(
            AdministrativeBoundaryContractErrorCode::InvalidRequest,
            "adcode must be exactly six ASCII digits",
        ));
    }
    let unit = authority.get_china_admin_unit(adcode).ok_or_else(|| {
        contract_error(
            AdministrativeBoundaryContractErrorCode::UnknownAdcode,
            format!("adcode {adcode} is not in the requested authority"),
        )
    })?;
    if !unit.supported || !unit.selectable {
        return Err(contract_error(
            AdministrativeBoundaryContractErrorCode::UnknownAdcode,
            format!("adcode {adcode} is not a supported selectable identity"),
        ));
    }
    Ok(())
}

fn validate_token(
    field: &str,
    value: &str,
    max_chars: usize,
) -> Result<(), AdministrativeBoundaryContractError> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().count() > max_chars
        || value.chars().any(char::is_control)
    {
        return Err(contract_error(
            AdministrativeBoundaryContractErrorCode::InvalidRequest,
            format!("invalid {field}"),
        ));
    }
    Ok(())
}

fn validate_authoritative_geometry_bbox(
    geometry: &Value,
    bbox: &[f64; 4],
) -> Result<(), AdministrativeBoundaryContractError> {
    let geometry = geometry.as_object().ok_or_else(|| {
        contract_error(
            AdministrativeBoundaryContractErrorCode::InvalidGeometry,
            "resolved boundary must be an authoritative Polygon/MultiPolygon object",
        )
    })?;
    let validated = validated_authoritative_polygonal_geojson(geometry).ok_or_else(|| {
        contract_error(
            AdministrativeBoundaryContractErrorCode::InvalidGeometry,
            "resolved boundary must have valid non-degenerate EPSG:4326 Polygon/MultiPolygon topology",
        )
    })?;
    if validated.bbox != *bbox {
        return Err(contract_error(
            AdministrativeBoundaryContractErrorCode::InvalidGeometry,
            "returned bbox must exactly equal the strict geometry-derived bbox",
        ));
    }
    Ok(())
}
