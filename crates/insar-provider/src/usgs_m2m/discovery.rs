use std::collections::VecDeque;
use std::sync::Arc;

use insar_core::ProviderNetworkPolicy;
use serde_json::{Map, Value};

use super::endpoint::{zeroize_json, SensitiveJson};
use super::error::{UsgsM2mEndpointClass, UsgsM2mError, UsgsM2mErrorCode};
use super::session::UsgsM2mSession;

const MAX_DATASET_RECORDS: usize = 256;
const MAX_DATASET_ID_BYTES: usize = 128;
const MAX_DATASET_ALIAS_BYTES: usize = 256;
const MAX_NAME_BYTES: usize = 1024;
const MAX_OWNER_BYTES: usize = 64;
const MAX_CATALOGS: usize = 16;
const MAX_CATALOG_BYTES: usize = 32;

const LANDSAT_C2_L2_COLLECTION_NAME: &str = "Landsat 8-9 OLI/TIRS C2 L2";
const LANDSAT_C2_L2_CATEGORY_NAME: &str = "Landsat Collection 2 Level-2";
const LANDSAT_DATA_OWNER: &str = "LSAA";
const EARTH_EXPLORER_CATALOG: &str = "EE";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsgsM2mDatasetIdentity {
    dataset_id: String,
    dataset_alias: String,
}

impl UsgsM2mDatasetIdentity {
    pub fn dataset_id(&self) -> &str {
        &self.dataset_id
    }

    pub fn upstream_dataset_identity(&self) -> &str {
        &self.dataset_alias
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mDatasetSupportStatus {
    Supported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsgsM2mDatasetDescriptor {
    identity: UsgsM2mDatasetIdentity,
    collection_name: String,
    collection_long_name: String,
    dataset_category_name: String,
    data_owner: String,
    catalogs: Vec<String>,
    support_status: UsgsM2mDatasetSupportStatus,
}

impl UsgsM2mDatasetDescriptor {
    pub const fn identity(&self) -> &UsgsM2mDatasetIdentity {
        &self.identity
    }

    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    pub fn collection_long_name(&self) -> &str {
        &self.collection_long_name
    }

    pub fn dataset_category_name(&self) -> &str {
        &self.dataset_category_name
    }

    pub fn data_owner(&self) -> &str {
        &self.data_owner
    }

    pub fn catalogs(&self) -> &[String] {
        &self.catalogs
    }

    pub const fn support_status(&self) -> UsgsM2mDatasetSupportStatus {
        self.support_status
    }

    fn matches_landsat_c2_l2_scope(&self) -> bool {
        self.collection_name == LANDSAT_C2_L2_COLLECTION_NAME
            && self.dataset_category_name == LANDSAT_C2_L2_CATEGORY_NAME
            && self.data_owner == LANDSAT_DATA_OWNER
            && self
                .catalogs
                .iter()
                .any(|catalog| catalog == EARTH_EXPLORER_CATALOG)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsgsM2mDatasetDiscoveryRequest(());

impl UsgsM2mDatasetDiscoveryRequest {
    pub const fn landsat_c2_l2() -> Self {
        Self(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsgsM2mDatasetDiscoveryResponse {
    descriptor: UsgsM2mDatasetDescriptor,
    records_examined: usize,
    candidate_count: usize,
    duplicate_count: usize,
}

impl UsgsM2mDatasetDiscoveryResponse {
    pub const fn descriptor(&self) -> &UsgsM2mDatasetDescriptor {
        &self.descriptor
    }

    pub const fn records_examined(&self) -> usize {
        self.records_examined
    }

    pub const fn candidate_count(&self) -> usize {
        self.candidate_count
    }

    pub const fn duplicate_count(&self) -> usize {
        self.duplicate_count
    }
}

pub struct UsgsM2mDatasetDiscoveryClient {
    session: Arc<UsgsM2mSession>,
}

impl UsgsM2mDatasetDiscoveryClient {
    pub fn new(session: Arc<UsgsM2mSession>) -> Self {
        Self { session }
    }

    pub fn network_policy(&self) -> ProviderNetworkPolicy {
        self.session.network_policy()
    }

    pub fn discover(
        &self,
        _request: UsgsM2mDatasetDiscoveryRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mDatasetDiscoveryResponse, UsgsM2mError> {
        if cancelled() {
            return Err(discovery_error(UsgsM2mErrorCode::Cancelled));
        }
        let data = self.session.request_dataset_discovery(cancelled)?;
        if cancelled() {
            return Err(discovery_error(UsgsM2mErrorCode::Cancelled));
        }
        select_landsat_c2_l2_descriptor(data)
    }
}

pub(super) fn select_landsat_c2_l2_descriptor(
    mut data: SensitiveJson,
) -> Result<UsgsM2mDatasetDiscoveryResponse, UsgsM2mError> {
    let records = match data.take() {
        Value::Array(records) if records.len() <= MAX_DATASET_RECORDS => records,
        mut invalid => {
            zeroize_json(&mut invalid);
            return Err(discovery_error(UsgsM2mErrorCode::ResponseInvalid));
        }
    };
    let records_examined = records.len();
    let mut remaining = VecDeque::from(records);
    let mut candidates = Vec::new();
    while let Some(record) = remaining.pop_front() {
        let descriptor = match parse_descriptor(record) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                for mut unparsed in remaining {
                    zeroize_json(&mut unparsed);
                }
                return Err(error);
            }
        };
        if descriptor.matches_landsat_c2_l2_scope() {
            candidates.push(descriptor);
        }
    }

    let candidate_count = candidates.len();
    let mut unique: Vec<UsgsM2mDatasetDescriptor> = Vec::new();
    let mut duplicate_count = 0_usize;
    for candidate in candidates {
        let matching = unique.iter().find(|existing| {
            existing.identity.dataset_alias == candidate.identity.dataset_alias
                || existing.identity.dataset_id == candidate.identity.dataset_id
        });
        match matching {
            Some(existing) if existing == &candidate => duplicate_count += 1,
            Some(_) => return Err(discovery_error(UsgsM2mErrorCode::DatasetConflict)),
            None => unique.push(candidate),
        }
    }

    match unique.len() {
        0 => Err(discovery_error(UsgsM2mErrorCode::DatasetNotFound)),
        1 => Ok(UsgsM2mDatasetDiscoveryResponse {
            descriptor: unique.pop().expect("one descriptor remains"),
            records_examined,
            candidate_count,
            duplicate_count,
        }),
        _ => Err(discovery_error(UsgsM2mErrorCode::DatasetAmbiguous)),
    }
}

fn parse_descriptor(value: Value) -> Result<UsgsM2mDatasetDescriptor, UsgsM2mError> {
    let mut protected = SensitiveJson::new(value);
    let value = protected.take();
    let Value::Object(mut map) = value else {
        let mut invalid = value;
        zeroize_json(&mut invalid);
        return Err(discovery_error(UsgsM2mErrorCode::ResponseInvalid));
    };

    let dataset_id = take_identifier(&mut map, "datasetId", MAX_DATASET_ID_BYTES)?;
    let dataset_alias = take_identifier(&mut map, "datasetAlias", MAX_DATASET_ALIAS_BYTES)?;
    let collection_name = take_text(&mut map, "collectionName", MAX_NAME_BYTES, false)?;
    let collection_long_name = take_text(&mut map, "collectionLongName", MAX_NAME_BYTES, false)?;
    let dataset_category_name = take_text(&mut map, "datasetCategoryName", MAX_NAME_BYTES, false)?;
    let data_owner = take_text(&mut map, "dataOwner", MAX_OWNER_BYTES, true)?;
    let catalogs = take_catalogs(&mut map)?;
    let mut unknown = Value::Object(map);
    zeroize_json(&mut unknown);

    Ok(UsgsM2mDatasetDescriptor {
        identity: UsgsM2mDatasetIdentity {
            dataset_id,
            dataset_alias,
        },
        collection_name,
        collection_long_name,
        dataset_category_name,
        data_owner,
        catalogs,
        support_status: UsgsM2mDatasetSupportStatus::Supported,
    })
}

fn take_identifier(
    map: &mut Map<String, Value>,
    field: &str,
    max_bytes: usize,
) -> Result<String, UsgsM2mError> {
    let value = take_field(map, field)?;
    match value {
        Value::String(text)
            if valid_text(&text, max_bytes, true)
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)) =>
        {
            Ok(text)
        }
        mut invalid => {
            zeroize_json(&mut invalid);
            Err(discovery_error(UsgsM2mErrorCode::ResponseInvalid))
        }
    }
}

fn take_text(
    map: &mut Map<String, Value>,
    field: &str,
    max_bytes: usize,
    ascii: bool,
) -> Result<String, UsgsM2mError> {
    let value = take_field(map, field)?;
    match value {
        Value::String(text) if valid_text(&text, max_bytes, ascii) => Ok(text),
        mut invalid => {
            zeroize_json(&mut invalid);
            Err(discovery_error(UsgsM2mErrorCode::ResponseInvalid))
        }
    }
}

fn take_catalogs(map: &mut Map<String, Value>) -> Result<Vec<String>, UsgsM2mError> {
    let value = take_field(map, "catalogs")?;
    let Value::Array(values) = value else {
        let mut invalid = value;
        zeroize_json(&mut invalid);
        return Err(discovery_error(UsgsM2mErrorCode::ResponseInvalid));
    };
    if values.is_empty() || values.len() > MAX_CATALOGS {
        let mut invalid = Value::Array(values);
        zeroize_json(&mut invalid);
        return Err(discovery_error(UsgsM2mErrorCode::ResponseInvalid));
    }
    let mut catalogs = Vec::with_capacity(values.len());
    for value in values {
        match value {
            Value::String(catalog)
                if valid_text(&catalog, MAX_CATALOG_BYTES, true)
                    && catalog
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)) =>
            {
                catalogs.push(catalog)
            }
            mut invalid => {
                zeroize_json(&mut invalid);
                return Err(discovery_error(UsgsM2mErrorCode::ResponseInvalid));
            }
        }
    }
    catalogs.sort();
    catalogs.dedup();
    Ok(catalogs)
}

fn take_field(map: &mut Map<String, Value>, field: &str) -> Result<Value, UsgsM2mError> {
    map.remove(field)
        .ok_or_else(|| discovery_error(UsgsM2mErrorCode::ResponseInvalid))
}

fn valid_text(value: &str, max_bytes: usize, ascii: bool) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_bytes
        && (!ascii || value.is_ascii())
        && !value.chars().any(char::is_control)
}

fn discovery_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::DatasetDiscovery)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use insar_core::ProviderNetworkPolicy;
    use serde_json::json;

    use super::*;
    use crate::usgs_m2m::endpoint::UsgsM2mSessionEndpoint;
    use crate::usgs_m2m::session::{UsgsM2mApiKey, UsgsM2mApplicationToken, UsgsM2mLoginMaterial};

    fn descriptor(dataset_id: &str, dataset_alias: &str) -> Value {
        json!({
            "datasetId": dataset_id,
            "datasetAlias": dataset_alias,
            "datasetCategoryName": LANDSAT_C2_L2_CATEGORY_NAME,
            "collectionName": LANDSAT_C2_L2_COLLECTION_NAME,
            "collectionLongName": "Fixture Landsat 8-9 Collection 2 Level-2",
            "dataOwner": LANDSAT_DATA_OWNER,
            "catalogs": [EARTH_EXPLORER_CATALOG],
            "supportCloudCover": true
        })
    }

    fn select(records: Vec<Value>) -> Result<UsgsM2mDatasetDiscoveryResponse, UsgsM2mError> {
        select_landsat_c2_l2_descriptor(SensitiveJson::new(Value::Array(records)))
    }

    #[test]
    fn unique_candidate_is_selected_without_freezing_a_production_constant() {
        let l1 = json!({
            "datasetId": "fixture-l1-id",
            "datasetAlias": "fixture_landsat_c2_l1",
            "datasetCategoryName": "Landsat Collection 2 Level-1",
            "collectionName": "Landsat 8-9 OLI/TIRS C2 L1",
            "collectionLongName": "Fixture Landsat 8-9 Collection 2 Level-1",
            "dataOwner": LANDSAT_DATA_OWNER,
            "catalogs": [EARTH_EXPLORER_CATALOG]
        });
        let result = select(vec![l1, descriptor("fixture-l2-id", "fixture_l2_alias")]).unwrap();

        assert_eq!(result.records_examined(), 2);
        assert_eq!(result.candidate_count(), 1);
        assert_eq!(result.duplicate_count(), 0);
        assert_eq!(result.descriptor().identity().dataset_id(), "fixture-l2-id");
        assert_eq!(
            result.descriptor().identity().upstream_dataset_identity(),
            "fixture_l2_alias"
        );
        assert_eq!(
            result.descriptor().support_status(),
            UsgsM2mDatasetSupportStatus::Supported
        );
    }

    #[test]
    fn zero_and_multiple_candidates_fail_closed() {
        assert_eq!(
            select(Vec::new()).unwrap_err().code(),
            UsgsM2mErrorCode::DatasetNotFound
        );
        assert_eq!(
            select(vec![
                descriptor("fixture-id-a", "fixture_alias_a"),
                descriptor("fixture-id-b", "fixture_alias_b"),
            ])
            .unwrap_err()
            .code(),
            UsgsM2mErrorCode::DatasetAmbiguous
        );
    }

    #[test]
    fn exact_duplicates_are_counted_and_authoritative_conflicts_are_rejected() {
        let first = descriptor("fixture-id", "fixture_alias");
        let duplicate = first.clone();
        let result = select(vec![first.clone(), duplicate]).unwrap();
        assert_eq!(result.candidate_count(), 2);
        assert_eq!(result.duplicate_count(), 1);

        let mut conflicting = first;
        conflicting["collectionLongName"] = Value::String("Conflicting authority".to_owned());
        assert_eq!(
            select(vec![descriptor("fixture-id", "fixture_alias"), conflicting])
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::DatasetConflict
        );

        assert_eq!(
            select(vec![
                descriptor("fixture-shared-id", "fixture_alias_a"),
                descriptor("fixture-shared-id", "fixture_alias_b"),
            ])
            .unwrap_err()
            .code(),
            UsgsM2mErrorCode::DatasetConflict
        );
    }

    #[test]
    fn malformed_identity_scope_and_pagination_shapes_never_gain_authority() {
        let mut missing = descriptor("fixture-id", "fixture_alias");
        missing.as_object_mut().unwrap().remove("datasetAlias");
        assert_eq!(
            select(vec![missing]).unwrap_err().code(),
            UsgsM2mErrorCode::ResponseInvalid
        );

        for invalid_alias in [
            "control\nvalue",
            "",
            &"a".repeat(MAX_DATASET_ALIAS_BYTES + 1),
        ] {
            let mut invalid = descriptor("fixture-id", "fixture_alias");
            invalid["datasetAlias"] = Value::String(invalid_alias.to_owned());
            assert_eq!(
                select(vec![invalid]).unwrap_err().code(),
                UsgsM2mErrorCode::ResponseInvalid
            );
        }

        let mut outside_scope = descriptor("fixture-id", "fixture_alias");
        outside_scope["collectionName"] = Value::String("Landsat 7 Collection 2 L2".to_owned());
        assert_eq!(
            select(vec![outside_scope]).unwrap_err().code(),
            UsgsM2mErrorCode::DatasetNotFound
        );

        let mut unknown = descriptor("fixture-id", "fixture_alias");
        unknown["nextRecord"] = json!("must-not-be-trusted");
        unknown["processingLevel"] = json!("L2SP");
        assert!(select(vec![unknown]).is_ok());

        assert_eq!(
            select_landsat_c2_l2_descriptor(SensitiveJson::new(json!({
                "results": [descriptor("fixture-id", "fixture_alias")],
                "nextRecord": 2
            })))
            .unwrap_err()
            .code(),
            UsgsM2mErrorCode::ResponseInvalid
        );
    }

    struct FixtureEndpoint {
        policy: ProviderNetworkPolicy,
        data: Mutex<Option<Value>>,
        dataset_calls: AtomicUsize,
    }

    impl UsgsM2mSessionEndpoint for FixtureEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn login_token(
            &self,
            _material: &UsgsM2mLoginMaterial,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<UsgsM2mApiKey, UsgsM2mError> {
            UsgsM2mApiKey::from_wire("fixture-runtime-key".to_owned())
        }

        fn logout(
            &self,
            _api_key: &UsgsM2mApiKey,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<(), UsgsM2mError> {
            Ok(())
        }

        fn dataset_search(
            &self,
            api_key: &UsgsM2mApiKey,
            cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<SensitiveJson, UsgsM2mError> {
            assert_eq!(api_key.expose(), "fixture-runtime-key");
            if cancelled() {
                return Err(discovery_error(UsgsM2mErrorCode::Cancelled));
            }
            self.dataset_calls.fetch_add(1, Ordering::SeqCst);
            Ok(SensitiveJson::new(
                self.data.lock().unwrap().take().unwrap(),
            ))
        }
    }

    #[test]
    fn discovery_client_requires_an_authenticated_session_and_one_endpoint_call() {
        let policy = ProviderNetworkPolicy::direct();
        let endpoint = Arc::new(FixtureEndpoint {
            policy,
            data: Mutex::new(Some(Value::Array(vec![descriptor(
                "fixture-id",
                "fixture_alias",
            )]))),
            dataset_calls: AtomicUsize::new(0),
        });
        let endpoint_trait: Arc<dyn UsgsM2mSessionEndpoint> = endpoint.clone();
        let session = Arc::new(UsgsM2mSession::from_endpoint(policy, endpoint_trait).unwrap());
        let client = UsgsM2mDatasetDiscoveryClient::new(Arc::clone(&session));
        assert_eq!(client.network_policy(), policy);
        assert_eq!(
            client
                .discover(UsgsM2mDatasetDiscoveryRequest::landsat_c2_l2(), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::AuthRequired
        );
        assert_eq!(endpoint.dataset_calls.load(Ordering::SeqCst), 0);

        session
            .login(
                UsgsM2mLoginMaterial::new(
                    "fixture-user",
                    UsgsM2mApplicationToken::new("fixture-application-token").unwrap(),
                )
                .unwrap(),
                &|| false,
            )
            .unwrap();
        let result = client
            .discover(UsgsM2mDatasetDiscoveryRequest::landsat_c2_l2(), &|| false)
            .unwrap();
        assert_eq!(result.candidate_count(), 1);
        assert_eq!(endpoint.dataset_calls.load(Ordering::SeqCst), 1);
    }
}
