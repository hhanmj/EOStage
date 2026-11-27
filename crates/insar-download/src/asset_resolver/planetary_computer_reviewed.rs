use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetRole,
    FrozenAssetAccess, ProviderNetworkPolicy, StableAssetReference,
};
use insar_provider::planetary_computer_asset::{
    validate_planetary_computer_reviewed_asset_identity, validate_planetary_computer_stable_asset,
    PlanetaryComputerAssetSigner, PlanetaryComputerReviewedAssetClass,
    PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};

use super::types::ResolvedHostPolicy;
use super::{
    AssetResolver, ResolveContext, ResolveError, ResolvedAsset, ResolvedMethod, ShadowAccessIntent,
    ShadowResolveContext,
};

pub const PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID: &str =
    "planetary-computer.sentinel-1-grd.cog.v1";
pub const PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID: &str =
    "planetary-computer.cop-dem-glo-30.cog.v1";
pub const PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID: &str =
    "planetary-computer.sentinel-2-l2a.band-cog.v1";

pub struct PlanetaryComputerReviewedAssetResolver {
    class: PlanetaryComputerReviewedAssetClass,
    signer: PlanetaryComputerAssetSigner,
}

impl PlanetaryComputerReviewedAssetResolver {
    pub fn sentinel1_grd(network_policy: ProviderNetworkPolicy) -> Result<Self, ResolveError> {
        Self::new(
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            network_policy,
        )
    }

    pub fn cop_dem_glo30(network_policy: ProviderNetworkPolicy) -> Result<Self, ResolveError> {
        Self::new(
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
            network_policy,
        )
    }

    pub fn sentinel2_l2a_band(network_policy: ProviderNetworkPolicy) -> Result<Self, ResolveError> {
        Self::new(
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
            network_policy,
        )
    }

    fn new(
        class: PlanetaryComputerReviewedAssetClass,
        network_policy: ProviderNetworkPolicy,
    ) -> Result<Self, ResolveError> {
        let signer =
            PlanetaryComputerAssetSigner::new(network_policy).map_err(map_provider_error)?;
        Ok(Self { class, signer })
    }

    fn expected_resolver_id(&self) -> &'static str {
        match self.class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
                PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID
            }
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => {
                PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID
            }
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID
            }
        }
    }

    fn validate_reference_and_descriptor(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
    ) -> Result<String, ResolveError> {
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
        if reference.provider_id != PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID
            || reference.collection_id != self.class.collection_id()
            || reference.canonical_product_key.is_some()
            || descriptor.resolver_id != self.expected_resolver_id()
            || descriptor.scheme != AssetAccessScheme::PublicHttps
            || descriptor.requires_credentials
            || descriptor.credential_profile.is_some()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        let expected_product_key = provider_product_key(
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
            self.class.collection_id(),
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
        validate_planetary_computer_reviewed_asset_identity(
            self.class,
            &reference.provider_product_id,
            &reference.asset_key,
            stable_href,
        )
        .map_err(map_provider_error)
    }
}

impl AssetResolver for PlanetaryComputerReviewedAssetResolver {
    fn resolver_id(&self) -> &'static str {
        self.expected_resolver_id()
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::PublicHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.resolver_id == self.expected_resolver_id()
            && descriptor.scheme == AssetAccessScheme::PublicHttps
            && !descriptor.requires_credentials
            && descriptor.credential_profile.is_none()
            && descriptor.stable_href.as_deref().is_some_and(|href| {
                validate_planetary_computer_stable_asset(self.class, href).is_ok()
            })
    }

    fn validate_frozen(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        frozen
            .validate()
            .map_err(|_| ResolveError::descriptor_invalid())?;
        let file_name =
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
        let file_name = self.validate_reference_and_descriptor(reference, descriptor)?;
        let stable_href = descriptor
            .stable_href
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let runtime = self
            .signer
            .sign_reviewed_asset(self.class, stable_href, &|| {
                context.check_cancelled().is_err()
            })
            .map_err(map_provider_error)?;
        context.check_cancelled()?;
        let (url, expires_at) = runtime.into_parts();
        let host_policy = match self.class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
                ResolvedHostPolicy::PlanetaryComputerSentinel1GrdV1
            }
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => {
                ResolvedHostPolicy::PlanetaryComputerCopDemGlo30V1
            }
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                ResolvedHostPolicy::PlanetaryComputerSentinel2L2aV1
            }
        };
        Ok(ResolvedAsset::new(url.as_str(), ResolvedMethod::Get)?
            .with_host_policy(host_policy)
            .with_expiry(expires_at)
            .with_suggested_filename(Some(file_name))
            .with_asset_role(AssetRole::Primary))
    }

    fn prepare_auth_refresh(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        self.validate_frozen(frozen)?;
        self.signer.invalidate_collection_sas(self.class);
        Ok(())
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::not_configured())
    }
}

fn map_provider_error(error: insar_core::ContractError) -> ResolveError {
    match error.code.as_str() {
        "MPC_ASSET_CANCELLED" => ResolveError::cancelled(),
        "MPC_ASSET_CONFIG_INVALID" | "MPC_ASSET_REJECTED" => ResolveError::descriptor_invalid(),
        code if code.starts_with("MPC_ASSET_SIGN_")
            && matches!(
                error
                    .debug_context
                    .get("reason")
                    .and_then(serde_json::Value::as_str),
                Some("transport" | "http_status_transient")
            ) =>
        {
            ResolveError::timeout()
        }
        _ => ResolveError::failed(),
    }
}

#[cfg(test)]
mod tests {
    use insar_core::AssetResolverParameters;

    use super::*;

    const S1_ID: &str = "S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2";
    const S1_HREF: &str = "https://sentinel1euwest.blob.core.windows.net/s1-grd/GRD/2025/1/23/IW/DV/S1A_IW_GRDH_1SDV_20250123T215225_20250123T215254_057577_0717D2_887E/measurement/iw-vv.tiff";
    const DEM_ID: &str = "Copernicus_DSM_COG_10_N25_00_E121_00_DEM";
    const DEM_HREF: &str = "https://elevationeuwest.blob.core.windows.net/copernicus-dem/COP30_hh/Copernicus_DSM_COG_10_N25_00_E121_00_DEM.tif";
    const S2_ID: &str = "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916";
    const S2_HREF: &str = "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_20260706T065916.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/T48SYC_20260706T032511_B02_10m.tif";

    fn frozen(
        class: PlanetaryComputerReviewedAssetClass,
        product_id: &str,
        asset_key: &str,
        href: &str,
        file_name: &str,
    ) -> FrozenAssetAccess {
        let product_key = provider_product_key(
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
            class.collection_id(),
            product_id,
        )
        .unwrap();
        FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
                collection_id: class.collection_id().to_owned(),
                provider_product_id: product_id.to_owned(),
                provider_product_key: product_key.clone(),
                canonical_product_key: None,
                asset_key: asset_key.to_owned(),
                stable_asset_key: stable_asset_key(&product_key, asset_key).unwrap(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: match class {
                    PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => {
                        PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID
                    }
                    PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => {
                        PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID
                    }
                    PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => {
                        PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID
                    }
                }
                .to_owned(),
                scheme: AssetAccessScheme::PublicHttps,
                stable_href: Some(href.to_owned()),
                requires_credentials: false,
                credential_profile: None,
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(file_name.to_owned()),
            expected_size_bytes: Some(4096),
            checksum_algorithm: None,
            checksum_value: None,
        }
    }

    #[test]
    fn transient_sign_failures_are_retryable_but_terminal_sign_failures_are_not() {
        fn provider_error(code: &str, reason: &str) -> insar_core::ContractError {
            insar_core::ContractError::new(
                code,
                "fixture",
                serde_json::Map::from_iter([(
                    "reason".to_owned(),
                    serde_json::Value::String(reason.to_owned()),
                )]),
            )
            .unwrap()
        }

        for (code, reason) in [
            ("MPC_ASSET_SIGN_TRANSPORT", "transport"),
            ("MPC_ASSET_SIGN_RATE_LIMITED", "http_status_transient"),
            ("MPC_ASSET_SIGN_UNAVAILABLE", "http_status_transient"),
        ] {
            assert_eq!(
                map_provider_error(provider_error(code, reason)).kind(),
                super::super::ResolveErrorKind::Timeout,
                "{code}/{reason}"
            );
        }
        for (code, reason) in [
            ("MPC_ASSET_SIGN_UNAVAILABLE", "http_status_terminal"),
            ("MPC_ASSET_SIGN_REJECTED", "http_status_terminal"),
            ("MPC_ASSET_SIGN_TRANSPORT", "response_read"),
            ("MPC_ASSET_SIGN_RESPONSE_INVALID", "response_json"),
            ("MPC_ASSET_SIGN_RESPONSE_INVALID", "signed_identity"),
            ("MPC_ASSET_SIGN_RESPONSE_INVALID", "signed_permissions"),
        ] {
            assert_eq!(
                map_provider_error(provider_error(code, reason)).kind(),
                super::super::ResolveErrorKind::Failed,
                "{code}/{reason}"
            );
        }
    }

    #[test]
    fn frozen_identity_is_exact_for_both_reviewed_classes() {
        let s1 =
            PlanetaryComputerReviewedAssetResolver::sentinel1_grd(ProviderNetworkPolicy::direct())
                .unwrap();
        s1.validate_frozen(&frozen(
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            S1_ID,
            "vv",
            S1_HREF,
            "iw-vv.tiff",
        ))
        .unwrap();

        let dem =
            PlanetaryComputerReviewedAssetResolver::cop_dem_glo30(ProviderNetworkPolicy::direct())
                .unwrap();
        dem.validate_frozen(&frozen(
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
            DEM_ID,
            "data",
            DEM_HREF,
            &format!("{DEM_ID}.tif"),
        ))
        .unwrap();

        let sentinel2 = PlanetaryComputerReviewedAssetResolver::sentinel2_l2a_band(
            ProviderNetworkPolicy::direct(),
        )
        .unwrap();
        sentinel2
            .validate_frozen(&frozen(
                PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
                S2_ID,
                "B02",
                S2_HREF,
                "T48SYC_20260706T032511_B02_10m.tif",
            ))
            .unwrap();
    }

    #[test]
    fn forged_class_asset_key_size_and_product_identity_fail_closed() {
        let resolver =
            PlanetaryComputerReviewedAssetResolver::sentinel1_grd(ProviderNetworkPolicy::direct())
                .unwrap();
        let valid = frozen(
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            S1_ID,
            "vv",
            S1_HREF,
            "iw-vv.tiff",
        );

        let mut wrong_key = valid.clone();
        wrong_key.reference.asset_key = "data".to_owned();
        assert!(resolver.validate_frozen(&wrong_key).is_err());

        let mut no_size = valid.clone();
        no_size.expected_size_bytes = None;
        assert!(resolver.validate_frozen(&no_size).is_err());

        let mut wrong_provider = valid.clone();
        wrong_provider.reference.provider_id = "asf.catalog".to_owned();
        assert!(resolver.validate_frozen(&wrong_provider).is_err());

        let mut signed_stable = valid;
        signed_stable.descriptor.stable_href = Some(format!("{S1_HREF}?sig=forged"));
        assert!(resolver.validate_frozen(&signed_stable).is_err());
    }
}
