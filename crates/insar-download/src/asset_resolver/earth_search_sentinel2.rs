use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetRole,
    FrozenAssetAccess, StableAssetReference,
};
use insar_provider::catalog::{
    is_valid_earth_search_sentinel2_cog_url, validate_earth_search_sentinel2_stable_band_url,
    EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID, EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
};
use reqwest::Url;

use super::types::ResolvedHostPolicy;
use super::{
    AssetResolver, ResolveContext, ResolveError, ResolvedAsset, ResolvedMethod, ShadowAccessIntent,
    ShadowResolveContext,
};

pub const EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID: &str =
    "earth-search.sentinel-2-l2a.band-cog.v1";

#[derive(Clone, Copy, Debug, Default)]
pub struct EarthSearchSentinel2BandAssetResolver;

impl EarthSearchSentinel2BandAssetResolver {
    pub const fn new() -> Self {
        Self
    }

    fn validate_reference_and_descriptor(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
    ) -> Result<(String, String), ResolveError> {
        reference
            .validate()
            .map_err(|_| ResolveError::reference_invalid())?;
        descriptor
            .validate()
            .map_err(|_| ResolveError::descriptor_invalid())?;
        let stable_href = descriptor
            .stable_href
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        if reference.provider_id != EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
            || reference.collection_id != EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID
            || reference.canonical_product_key.is_some()
            || descriptor.resolver_id != EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID
            || descriptor.scheme != AssetAccessScheme::PublicHttps
            || descriptor.requires_credentials
            || descriptor.credential_profile.is_some()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        let expected_product_key = provider_product_key(
            EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
            EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
            &reference.provider_product_id,
        )
        .map_err(|_| ResolveError::reference_invalid())?;
        let expected_stable_key = stable_asset_key(&expected_product_key, &reference.asset_key)
            .map_err(|_| ResolveError::reference_invalid())?;
        if reference.provider_product_key != expected_product_key
            || reference.stable_asset_key != expected_stable_key
        {
            return Err(ResolveError::reference_invalid());
        }
        let file_name = validate_earth_search_sentinel2_stable_band_url(
            &reference.provider_product_id,
            &reference.asset_key,
            stable_href,
        )
        .map_err(|_| ResolveError::descriptor_invalid())?;
        Ok((stable_href.to_owned(), file_name))
    }
}

impl AssetResolver for EarthSearchSentinel2BandAssetResolver {
    fn resolver_id(&self) -> &'static str {
        EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::PublicHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.resolver_id == EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID
            && descriptor.scheme == AssetAccessScheme::PublicHttps
            && !descriptor.requires_credentials
            && descriptor.credential_profile.is_none()
            && descriptor
                .stable_href
                .as_deref()
                .and_then(|href| Url::parse(href).ok())
                .is_some_and(|url| is_valid_earth_search_sentinel2_cog_url(&url))
    }

    fn validate_frozen(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        frozen
            .validate()
            .map_err(|_| ResolveError::descriptor_invalid())?;
        let (_, file_name) =
            self.validate_reference_and_descriptor(&frozen.reference, &frozen.descriptor)?;
        if frozen.role != AssetRole::Primary
            || frozen.file_name.as_deref() != Some(file_name.as_str())
            || frozen.expected_size_bytes.is_none_or(|size| size == 0)
            || frozen.checksum_algorithm.is_some()
            || frozen.checksum_value.is_some()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(())
    }

    fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        context.check_cancelled()?;
        let (stable_href, file_name) =
            self.validate_reference_and_descriptor(reference, descriptor)?;
        Ok(ResolvedAsset::new(&stable_href, ResolvedMethod::Get)?
            .with_host_policy(ResolvedHostPolicy::EarthSearchSentinel2CogsV1)
            .with_suggested_filename(Some(file_name))
            .with_asset_role(AssetRole::Primary))
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::not_configured())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AsfCollectionProfileRegistry;
    use crate::{
        build_earth_search_sentinel2_band_cog_item, classify_frozen_execution,
        AssetExecutionPolicyRegistry, CredentialMaterial, CredentialProvider,
        CredentialProviderError, FrozenExecution, SystemClock,
    };
    use insar_core::SnapshotItem;
    use insar_provider::catalog::{
        EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID, EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
    };
    use insar_provider::planetary_computer_asset::PlanetaryComputerReviewedAssetEvidence;
    use serde_json::{json, Map, Value};

    struct EmptyCredentials;

    impl CredentialProvider for EmptyCredentials {
        fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
            Err(CredentialProviderError::Missing)
        }
    }

    fn earth_search_reviewed_source_fixture(item_id: &str, band: &str) -> SnapshotItem {
        let product_key = provider_product_key(
            EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
            EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
            item_id,
        )
        .unwrap();
        let stable_key = stable_asset_key(&product_key, band).unwrap();
        let footprint: Map<String, Value> = serde_json::from_value(json!({
            "type": "Polygon",
            "coordinates": [[[117.0, 31.0], [118.0, 31.0], [118.0, 32.0], [117.0, 32.0], [117.0, 31.0]]]
        }))
        .unwrap();
        SnapshotItem {
            item_id: product_key.clone(),
            provider_id: EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            display_name: item_id.to_owned(),
            acquisition_datetime: Some("2026-06-09T02:56:41Z".to_owned()),
            footprint_geojson: Some(footprint),
            metadata: Map::from_iter([
                (
                    "logical_dataset".to_owned(),
                    Value::String("optical.sentinel-2.l2a".to_owned()),
                ),
                (
                    "native_collection_id".to_owned(),
                    Value::String(EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID.to_owned()),
                ),
                (
                    "collection_profile_id".to_owned(),
                    Value::String(EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID.to_owned()),
                ),
                (
                    "identity_rule_id".to_owned(),
                    Value::String(EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID.to_owned()),
                ),
                (
                    "provider_product_id".to_owned(),
                    Value::String(item_id.to_owned()),
                ),
                (
                    "provider_product_key".to_owned(),
                    Value::String(product_key),
                ),
                ("stac_item_id".to_owned(), Value::String(item_id.to_owned())),
                (
                    "source_kind".to_owned(),
                    Value::String("sentinel2".to_owned()),
                ),
                (
                    "product_kind".to_owned(),
                    Value::String("optical_scene".to_owned()),
                ),
                (
                    "assets".to_owned(),
                    Value::Array(vec![json!({
                        "asset_key": band,
                        "stable_asset_key": stable_key,
                        "role": "primary",
                        "native_roles": ["data"],
                        "resolution_m": 10
                    })]),
                ),
            ]),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        }
    }

    #[test]
    fn earth_search_sentinel2_band_cog_item_is_resolver_authoritative_and_public_https() {
        let item_id = "S2B_50SNA_20260609_0_L2A";
        let href = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/50/S/NA/2026/6/S2B_50SNA_20260609_0_L2A/B04.tif";
        let source = earth_search_reviewed_source_fixture(item_id, "B04");
        let evidence = PlanetaryComputerReviewedAssetEvidence {
            file_name: "S2B_50SNA_20260609_0_L2A_B04.tif".to_owned(),
            expected_size_bytes: 42_000_000,
        };

        let item = build_earth_search_sentinel2_band_cog_item(&source, "B04", href, &evidence)
            .expect("valid Earth Search Sentinel-2 band COG item");

        assert_eq!(item.provider_id, EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID);
        assert_eq!(item.display_name, "S2B_50SNA_20260609_0_L2A_B04.tif");

        let eligible = match classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ) {
            FrozenExecution::ResolverAuthoritative(eligible) => eligible,
            other => panic!("expected resolver authoritative, got {other:?}"),
        };
        assert_eq!(
            eligible.policy.auth_refresh_policy_id,
            crate::AuthRefreshPolicyId::PublicAnonymousHttpsV1
        );

        let resolver = EarthSearchSentinel2BandAssetResolver::new();
        assert!(resolver.can_resolve(&eligible.frozen_access.descriptor));
        resolver
            .validate_frozen(&eligible.frozen_access)
            .expect("frozen access validates");

        let creds = EmptyCredentials;
        let clock = SystemClock;
        let cancel = || false;
        let ctx = ResolveContext::new(&creds, &cancel, &clock);
        let resolved = resolver
            .resolve(
                &eligible.frozen_access.reference,
                &eligible.frozen_access.descriptor,
                &ctx,
            )
            .expect("public HTTPS COG resolves without credentials");
        assert_eq!(resolved.effective_url().as_str(), href);
        assert_eq!(
            resolved.host_policy(),
            Some(ResolvedHostPolicy::EarthSearchSentinel2CogsV1)
        );
        assert_eq!(
            resolved.suggested_filename(),
            Some("S2B_50SNA_20260609_0_L2A_B04.tif")
        );
    }

    #[test]
    fn earth_search_sentinel2_band_cog_rejects_untrusted_host_or_mismatched_band() {
        let item_id = "S2B_50SNA_20260609_0_L2A";
        let source = earth_search_reviewed_source_fixture(item_id, "B04");
        let evidence = PlanetaryComputerReviewedAssetEvidence {
            file_name: "S2B_50SNA_20260609_0_L2A_B04.tif".to_owned(),
            expected_size_bytes: 1024,
        };
        assert!(build_earth_search_sentinel2_band_cog_item(
            &source,
            "B04",
            "https://evil.example.com/sentinel-s2-l2a-cogs/50/S/NA/2026/6/S2B_50SNA_20260609_0_L2A/B04.tif",
            &evidence,
        )
        .is_err());
        assert!(build_earth_search_sentinel2_band_cog_item(
            &source,
            "B04",
            "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/50/S/NA/2026/6/S2B_50SNA_20260609_0_L2A/B08.tif",
            &evidence,
        )
        .is_err());
    }
}
