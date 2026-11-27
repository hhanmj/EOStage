use insar_core::{
    NisarSubsetDerivedIdentityAuthorityV2, NisarSubsetFrequency, NisarSubsetPlanV2,
    NisarSubsetPreparedIntentV2, NisarSubsetSourceAssetIdentityV2,
    NisarSubsetSourceProductIdentityV2, NisarSubsetTransferModeV2,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::StoreError;

pub const NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY: &str =
    "nisar_subset_prepared_intent_v2_envelope";
pub const NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_CONTRACT: &str =
    "NisarSubsetPreparedIntentEnvelopeV2";
pub const NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_SCHEMA_VERSION: u32 = 2;

pub const NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY: &str = "nisar_subset_plan_v2_envelope";
pub const NISAR_SUBSET_PLAN_V2_ENVELOPE_CONTRACT: &str = "NisarSubsetExecutionPlanEnvelopeV2";
pub const NISAR_SUBSET_PLAN_V2_ENVELOPE_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetPreparedIntentEnvelopeV2 {
    pub envelope_contract: String,
    pub schema_version: u32,
    pub canonical_intent_json: String,
    pub canonical_intent_sha256: String,
    pub source_product_identity: NisarSubsetSourceProductIdentityV2,
    pub source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    pub source_footprint_sha256: String,
    pub requested_geometry_sha256: String,
    pub effective_geometry_sha256: String,
    pub selected_frequency: NisarSubsetFrequency,
    pub selected_polarizations: Vec<String>,
    pub transfer_mode: NisarSubsetTransferModeV2,
    pub derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2,
}

impl NisarSubsetPreparedIntentEnvelopeV2 {
    pub fn from_intent(intent: &NisarSubsetPreparedIntentV2) -> Result<Self, StoreError> {
        intent
            .validate()
            .map_err(|_| StoreError::NisarSubsetPreparedIntentInvalid)?;
        let envelope = Self {
            envelope_contract: NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_CONTRACT.to_owned(),
            schema_version: NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_SCHEMA_VERSION,
            canonical_intent_json: intent
                .canonical_json()
                .map_err(|_| StoreError::NisarSubsetPreparedIntentInvalid)?,
            canonical_intent_sha256: intent.canonical_intent_sha256.clone(),
            source_product_identity: intent.source_product_identity.clone(),
            source_asset_identity: intent.source_asset_identity.clone(),
            source_footprint_sha256: intent.source_footprint_sha256.clone(),
            requested_geometry_sha256: intent.canonical_geometry_digest.requested_sha256.clone(),
            effective_geometry_sha256: intent.canonical_geometry_digest.effective_sha256.clone(),
            selected_frequency: intent.selected_frequency.clone(),
            selected_polarizations: intent.selected_polarizations.clone(),
            transfer_mode: intent.transfer_mode,
            derived_identity_authority: intent.derived_identity_authority,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn intent(&self) -> Result<NisarSubsetPreparedIntentV2, StoreError> {
        let intent: NisarSubsetPreparedIntentV2 = serde_json::from_str(&self.canonical_intent_json)
            .map_err(|_| StoreError::NisarSubsetPreparedIntentInvalid)?;
        intent
            .validate()
            .map_err(|_| StoreError::NisarSubsetPreparedIntentInvalid)?;
        if intent
            .canonical_json()
            .map_err(|_| StoreError::NisarSubsetPreparedIntentInvalid)?
            != self.canonical_intent_json
        {
            return Err(StoreError::NisarSubsetPreparedIntentInvalid);
        }
        Ok(intent)
    }

    pub fn validate(&self) -> Result<(), StoreError> {
        if self.envelope_contract != NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_CONTRACT
            || self.schema_version != NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_SCHEMA_VERSION
        {
            return Err(StoreError::NisarSubsetPreparedIntentInvalid);
        }
        let intent = self.intent()?;
        if self.canonical_intent_sha256 != intent.canonical_intent_sha256
            || self.source_product_identity != intent.source_product_identity
            || self.source_asset_identity != intent.source_asset_identity
            || self.source_footprint_sha256 != intent.source_footprint_sha256
            || self.requested_geometry_sha256 != intent.canonical_geometry_digest.requested_sha256
            || self.effective_geometry_sha256 != intent.canonical_geometry_digest.effective_sha256
            || self.selected_frequency != intent.selected_frequency
            || self.selected_polarizations != intent.selected_polarizations
            || self.transfer_mode != intent.transfer_mode
            || self.derived_identity_authority != intent.derived_identity_authority
        {
            return Err(StoreError::NisarSubsetPreparedIntentIdentityMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NisarSubsetExecutionPlanEnvelopeV2 {
    pub envelope_contract: String,
    pub schema_version: u32,
    pub canonical_plan_json: String,
    pub canonical_semantic_plan_digest: String,
    pub prepared_intent_sha256: String,
    pub source_product_identity: NisarSubsetSourceProductIdentityV2,
    pub source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    pub source_footprint_sha256: String,
    pub requested_geometry_sha256: String,
    pub effective_geometry_sha256: String,
    pub selected_frequency: NisarSubsetFrequency,
    pub selected_polarizations: Vec<String>,
    pub transfer_mode: NisarSubsetTransferModeV2,
    pub derived_identity_authority: NisarSubsetDerivedIdentityAuthorityV2,
}

impl NisarSubsetExecutionPlanEnvelopeV2 {
    pub fn from_plan(plan: &NisarSubsetPlanV2) -> Result<Self, StoreError> {
        plan.validate()
            .map_err(|_| StoreError::NisarSubsetExecutionPlanInvalid)?;
        let envelope = Self {
            envelope_contract: NISAR_SUBSET_PLAN_V2_ENVELOPE_CONTRACT.to_owned(),
            schema_version: NISAR_SUBSET_PLAN_V2_ENVELOPE_SCHEMA_VERSION,
            canonical_plan_json: plan
                .canonical_json()
                .map_err(|_| StoreError::NisarSubsetExecutionPlanInvalid)?,
            canonical_semantic_plan_digest: plan.canonical_semantic_plan_digest.clone(),
            prepared_intent_sha256: plan.prepared_intent_sha256.clone(),
            source_product_identity: plan.source_product_identity.clone(),
            source_asset_identity: plan.source_asset_identity.clone(),
            source_footprint_sha256: plan.source_footprint_sha256.clone(),
            requested_geometry_sha256: plan.canonical_geometry_digest.requested_sha256.clone(),
            effective_geometry_sha256: plan.canonical_geometry_digest.effective_sha256.clone(),
            selected_frequency: plan.selected_frequency.clone(),
            selected_polarizations: plan.selected_polarizations.clone(),
            transfer_mode: plan.transfer_mode,
            derived_identity_authority: plan.provenance.derived_identity_authority,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn plan(&self) -> Result<NisarSubsetPlanV2, StoreError> {
        let plan: NisarSubsetPlanV2 = serde_json::from_str(&self.canonical_plan_json)
            .map_err(|_| StoreError::NisarSubsetExecutionPlanInvalid)?;
        plan.validate()
            .map_err(|_| StoreError::NisarSubsetExecutionPlanInvalid)?;
        if plan
            .canonical_json()
            .map_err(|_| StoreError::NisarSubsetExecutionPlanInvalid)?
            != self.canonical_plan_json
        {
            return Err(StoreError::NisarSubsetExecutionPlanInvalid);
        }
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), StoreError> {
        if self.envelope_contract != NISAR_SUBSET_PLAN_V2_ENVELOPE_CONTRACT
            || self.schema_version != NISAR_SUBSET_PLAN_V2_ENVELOPE_SCHEMA_VERSION
        {
            return Err(StoreError::NisarSubsetExecutionPlanInvalid);
        }
        let plan = self.plan()?;
        if self.canonical_semantic_plan_digest != plan.canonical_semantic_plan_digest
            || self.prepared_intent_sha256 != plan.prepared_intent_sha256
            || self.source_product_identity != plan.source_product_identity
            || self.source_asset_identity != plan.source_asset_identity
            || self.source_footprint_sha256 != plan.source_footprint_sha256
            || self.requested_geometry_sha256 != plan.canonical_geometry_digest.requested_sha256
            || self.effective_geometry_sha256 != plan.canonical_geometry_digest.effective_sha256
            || self.selected_frequency != plan.selected_frequency
            || self.selected_polarizations != plan.selected_polarizations
            || self.transfer_mode != plan.transfer_mode
            || self.derived_identity_authority != plan.provenance.derived_identity_authority
        {
            return Err(StoreError::NisarSubsetExecutionPlanInvalid);
        }
        Ok(())
    }
}

pub fn embed_nisar_subset_prepared_intent_v2_envelope(
    metadata: &mut Map<String, Value>,
    intent: &NisarSubsetPreparedIntentV2,
) -> Result<(), StoreError> {
    if metadata.contains_key(NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY) {
        return Err(StoreError::NisarSubsetPreparedIntentAlreadyFrozen);
    }
    let value = serde_json::to_value(NisarSubsetPreparedIntentEnvelopeV2::from_intent(intent)?)
        .map_err(|_| StoreError::NisarSubsetPreparedIntentInvalid)?;
    metadata.insert(
        NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY.to_owned(),
        value,
    );
    Ok(())
}

pub fn parse_nisar_subset_prepared_intent_v2_envelope(
    metadata: &Map<String, Value>,
) -> Result<Option<NisarSubsetPreparedIntentEnvelopeV2>, StoreError> {
    metadata
        .get(NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY)
        .map(|value| {
            let envelope: NisarSubsetPreparedIntentEnvelopeV2 =
                serde_json::from_value(value.clone())
                    .map_err(|_| StoreError::NisarSubsetPreparedIntentInvalid)?;
            envelope.validate()?;
            Ok(envelope)
        })
        .transpose()
}

pub fn embed_nisar_subset_plan_v2_envelope(
    metadata: &mut Map<String, Value>,
    plan: &NisarSubsetPlanV2,
) -> Result<(), StoreError> {
    if metadata.contains_key(NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY) {
        return Err(StoreError::NisarSubsetExecutionPlanAlreadyFrozenConflict);
    }
    let envelope = NisarSubsetExecutionPlanEnvelopeV2::from_plan(plan)?;
    let value =
        serde_json::to_value(envelope).map_err(|_| StoreError::NisarSubsetExecutionPlanInvalid)?;
    metadata.insert(NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY.to_owned(), value);
    Ok(())
}

pub fn parse_nisar_subset_plan_v2_envelope(
    metadata: &Map<String, Value>,
) -> Result<Option<NisarSubsetExecutionPlanEnvelopeV2>, StoreError> {
    metadata
        .get(NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY)
        .map(|value| {
            let envelope: NisarSubsetExecutionPlanEnvelopeV2 =
                serde_json::from_value(value.clone())
                    .map_err(|_| StoreError::NisarSubsetExecutionPlanInvalid)?;
            envelope.validate()?;
            Ok(envelope)
        })
        .transpose()
}

pub(crate) fn validate_plan_matches_prepared_intent(
    plan: &NisarSubsetExecutionPlanEnvelopeV2,
    intent: &NisarSubsetPreparedIntentEnvelopeV2,
) -> Result<(), StoreError> {
    if plan.prepared_intent_sha256 != intent.canonical_intent_sha256
        || plan.source_product_identity != intent.source_product_identity
        || plan.source_asset_identity != intent.source_asset_identity
        || plan.source_footprint_sha256 != intent.source_footprint_sha256
        || plan.requested_geometry_sha256 != intent.requested_geometry_sha256
        || plan.effective_geometry_sha256 != intent.effective_geometry_sha256
        || plan.selected_frequency != intent.selected_frequency
        || plan.selected_polarizations != intent.selected_polarizations
        || plan.transfer_mode != intent.transfer_mode
        || plan.derived_identity_authority != intent.derived_identity_authority
    {
        return Err(StoreError::NisarSubsetExecutionPlanIntentMismatch);
    }
    Ok(())
}

pub(crate) fn validate_nisar_subset_plan_v2_metadata(
    metadata: &Map<String, Value>,
) -> Result<(), StoreError> {
    let intent = parse_nisar_subset_prepared_intent_v2_envelope(metadata)?;
    let plan = parse_nisar_subset_plan_v2_envelope(metadata)?;
    match (intent.as_ref(), plan.as_ref()) {
        (None, None) | (Some(_), None) => Ok(()),
        (None, Some(_)) => Err(StoreError::NisarSubsetPreparedIntentMissing),
        (Some(intent), Some(plan)) => validate_plan_matches_prepared_intent(plan, intent),
    }
}

pub(crate) fn validate_nisar_subset_prepared_intent_only_metadata(
    metadata: &Map<String, Value>,
) -> Result<(), StoreError> {
    validate_nisar_subset_plan_v2_metadata(metadata)?;
    if metadata.contains_key(NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY) {
        return Err(StoreError::NisarSubsetExecutionPlanAlreadyFrozenConflict);
    }
    Ok(())
}
