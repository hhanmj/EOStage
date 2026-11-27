//! Protected Bhoonidhi catalogue route for the NISAR S-band RSLC collection.
//!
//! This vertical slice intentionally has no ASF/CMR fallback and no credential
//! implementation. The native authentication agent supplies the small
//! [`BhoonidhiAccessTokenProvider`] boundary at runtime.

mod auth;
mod client;
mod mapper;
mod query;

#[cfg(test)]
mod fixtures;

pub use auth::{
    BhoonidhiAccessTokenLease, BhoonidhiAccessTokenProvider, BhoonidhiAuthError,
    BhoonidhiRefreshResult,
};
pub use client::{BhoonidhiCatalogClient, BhoonidhiCatalogLimits, BhoonidhiCatalogProgress};
pub use mapper::{bhoonidhi_collection, map_bhoonidhi_feature};
pub use query::{
    build_search_request, derive_bbox_from_geojson, BhoonidhiSearchRequest,
    BHOONIDHI_CURSOR_VERSION, BHOONIDHI_MAX_CURSOR_BYTES, BHOONIDHI_MAX_PAGE_SIZE,
};

pub const BHOONIDHI_PROVIDER_ID: &str = "isro.nrsc.bhoonidhi";
pub const BHOONIDHI_LOGICAL_DATASET: &str = "sar.nisar.s.ssar.rslc.v1";
pub const BHOONIDHI_PROFILE_ID: &str = "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1";
pub const BHOONIDHI_COLLECTION_ID: &str = "NISAR_SSAR_RSLC";
pub const BHOONIDHI_API_BASE_URL: &str = "https://bhoonidhi-api.nrsc.gov.in";
pub const BHOONIDHI_SEARCH_PATH: &str = "/data/search";
pub const BHOONIDHI_SEARCH_URL: &str = "https://bhoonidhi-api.nrsc.gov.in/data/search";
pub const BHOONIDHI_DOWNLOAD_PATH: &str = "/download";
pub const BHOONIDHI_IDENTITY_RULE_ID: &str = "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.identity.v1";
pub const BHOONIDHI_FILENAME_RULE_ID: &str = "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.filename.v1";
pub const BHOONIDHI_RESOLVER_POLICY_ID: &str = "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.resolver.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BhoonidhiSbandProductAvailability {
    AuthenticatedBhoonidhiSearch,
    AgencyPublicReleasePending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhoonidhiSbandProductDescriptor {
    pub product_type: &'static str,
    pub science_level: &'static str,
    pub collection_id: &'static str,
    pub availability: BhoonidhiSbandProductAvailability,
    pub search_url: &'static str,
    pub notes: &'static str,
}

/// ISRO Bhoonidhi S-SAR live authenticated STAC collections alongside the primary `NISAR_SSAR_RSLC` route.
pub const BHOONIDHI_SBAND_PRODUCTS: &[BhoonidhiSbandProductDescriptor] = &[
    BhoonidhiSbandProductDescriptor {
        product_type: "RSLC",
        science_level: "L1",
        collection_id: BHOONIDHI_COLLECTION_ID,
        availability: BhoonidhiSbandProductAvailability::AuthenticatedBhoonidhiSearch,
        search_url: BHOONIDHI_SEARCH_URL,
        notes: "Primary S-SAR L1 RSLC STAC collection (`NISAR_SSAR_RSLC`); requires authenticated Bhoonidhi session.",
    },
    BhoonidhiSbandProductDescriptor {
        product_type: "GSLC",
        science_level: "L2",
        collection_id: "NISAR_SSAR_GSLC",
        availability: BhoonidhiSbandProductAvailability::AuthenticatedBhoonidhiSearch,
        search_url: BHOONIDHI_SEARCH_URL,
        notes: "S-SAR L2 GSLC STAC collection (`NISAR_SSAR_GSLC`); requires authenticated Bhoonidhi session.",
    },
    BhoonidhiSbandProductDescriptor {
        product_type: "GCOV",
        science_level: "L2",
        collection_id: "NISAR_SSAR_GCOV",
        availability: BhoonidhiSbandProductAvailability::AuthenticatedBhoonidhiSearch,
        search_url: BHOONIDHI_SEARCH_URL,
        notes: "S-SAR L2 GCOV STAC collection (`NISAR_SSAR_GCOV`); requires authenticated Bhoonidhi session.",
    },
    BhoonidhiSbandProductDescriptor {
        product_type: "GUNW",
        science_level: "L2",
        collection_id: "NISAR_SSAR_GUNW",
        availability: BhoonidhiSbandProductAvailability::AuthenticatedBhoonidhiSearch,
        search_url: BHOONIDHI_SEARCH_URL,
        notes: "S-SAR L2 GUNW STAC collection (`NISAR_SSAR_GUNW`); requires authenticated Bhoonidhi session.",
    },
    BhoonidhiSbandProductDescriptor {
        product_type: "GOFF",
        science_level: "L2",
        collection_id: "NISAR_SSAR_GOFF",
        availability: BhoonidhiSbandProductAvailability::AuthenticatedBhoonidhiSearch,
        search_url: BHOONIDHI_SEARCH_URL,
        notes: "S-SAR L2 GOFF STAC collection (`NISAR_SSAR_GOFF`); requires authenticated Bhoonidhi session.",
    },
    BhoonidhiSbandProductDescriptor {
        product_type: "RIFG",
        science_level: "L1",
        collection_id: "NISAR_SSAR_RIFG",
        availability: BhoonidhiSbandProductAvailability::AuthenticatedBhoonidhiSearch,
        search_url: BHOONIDHI_SEARCH_URL,
        notes: "S-SAR L1 RIFG STAC collection (`NISAR_SSAR_RIFG`); requires authenticated Bhoonidhi session.",
    },
    BhoonidhiSbandProductDescriptor {
        product_type: "RUNW",
        science_level: "L1",
        collection_id: "NISAR_SSAR_RUNW",
        availability: BhoonidhiSbandProductAvailability::AuthenticatedBhoonidhiSearch,
        search_url: BHOONIDHI_SEARCH_URL,
        notes: "S-SAR L1 RUNW STAC collection (`NISAR_SSAR_RUNW`); requires authenticated Bhoonidhi session.",
    },
    BhoonidhiSbandProductDescriptor {
        product_type: "ROFF",
        science_level: "L1",
        collection_id: "NISAR_SSAR_ROFF",
        availability: BhoonidhiSbandProductAvailability::AuthenticatedBhoonidhiSearch,
        search_url: BHOONIDHI_SEARCH_URL,
        notes: "S-SAR L1 ROFF STAC collection (`NISAR_SSAR_ROFF`); requires authenticated Bhoonidhi session.",
    },
];

pub fn lookup_bhoonidhi_sband_product(
    product_type: &str,
) -> Option<&'static BhoonidhiSbandProductDescriptor> {
    let normalized = product_type.trim().to_ascii_uppercase();
    BHOONIDHI_SBAND_PRODUCTS
        .iter()
        .find(|entry| entry.product_type == normalized || entry.collection_id == normalized)
}

fn error(code: &str, message: &str, reason: &str) -> insar_core::ContractError {
    insar_core::ContractError::new(
        code,
        message,
        serde_json::Map::from_iter([(
            "reason".to_owned(),
            serde_json::Value::String(reason.to_owned()),
        )]),
    )
    .expect("bounded Bhoonidhi error is safe")
}
