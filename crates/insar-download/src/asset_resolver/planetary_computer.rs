use std::sync::Arc;

use insar_core::{
    stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, FrozenAssetAccess,
    ProviderNetworkPolicy, StableAssetReference,
};
use insar_provider::catalog::{
    CollectionProfileRegistry, LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_PROVIDER_ID,
    MPC_LANDSAT_PROVIDER_ID,
};
use insar_provider::planetary_computer::{
    asset_definitions_for_sensor, validate_planetary_computer_landsat_asset,
    validate_planetary_computer_landsat_stable_href, PlanetaryComputerError,
    PlanetaryComputerErrorCode, PlanetaryComputerLandsatClient, PlanetaryComputerSignedAsset,
};
use reqwest::Url;

use super::types::ResolvedHostPolicy;
use super::{
    AssetResolver, ResolveContext, ResolveError, ResolvedAsset, ResolvedMethod, ShadowAccessIntent,
    ShadowResolveContext,
};

pub const PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID: &str =
    "planetary-computer.landsat-c2-l2.assets.v1";
pub const PLANETARY_COMPUTER_HLS_RESOLVER_ID: &str = "planetary-computer.hls.assets.v1";

pub fn is_valid_planetary_computer_hls_href(href: &str) -> bool {
    let Ok(url) = Url::parse(href) else {
        return false;
    };
    if url.scheme() != "https" || url.fragment().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    (host == "hls2euwest.blob.core.windows.net" || host == "hlseuwest.blob.core.windows.net")
        && (url.path().starts_with("/hls2/") || url.path().starts_with("/hls/"))
        && (url.path().ends_with(".tif") || url.path().ends_with(".tiff"))
}

trait AssetSigningPort: Send + Sync {
    fn sign_asset(
        &self,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerSignedAsset, PlanetaryComputerError>;

    fn sign_hls_asset(
        &self,
        collection_id: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerSignedAsset, PlanetaryComputerError>;

    fn invalidate_sas(&self) -> Result<(), PlanetaryComputerError>;

    fn invalidate_hls_sas(&self, collection_id: &str) -> Result<(), PlanetaryComputerError>;
}

impl AssetSigningPort for PlanetaryComputerLandsatClient {
    fn sign_asset(
        &self,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerSignedAsset, PlanetaryComputerError> {
        self.sign_asset(stable_href, cancelled)
    }

    fn sign_hls_asset(
        &self,
        collection_id: &str,
        stable_href: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<PlanetaryComputerSignedAsset, PlanetaryComputerError> {
        self.sign_hls_asset(collection_id, stable_href, cancelled)
    }

    fn invalidate_sas(&self) -> Result<(), PlanetaryComputerError> {
        self.invalidate_sas()
    }

    fn invalidate_hls_sas(&self, collection_id: &str) -> Result<(), PlanetaryComputerError> {
        self.invalidate_hls_sas(collection_id)
    }
}

pub struct PlanetaryComputerLandsatAssetResolver {
    resolver_id: &'static str,
    signer: Arc<dyn AssetSigningPort>,
}

impl PlanetaryComputerLandsatAssetResolver {
    pub fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, ResolveError> {
        let client = PlanetaryComputerLandsatClient::new(network_policy)
            .map_err(map_planetary_computer_error)?;
        Ok(Self {
            resolver_id: PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID,
            signer: Arc::new(client),
        })
    }

    pub fn hls(network_policy: ProviderNetworkPolicy) -> Result<Self, ResolveError> {
        let client = PlanetaryComputerLandsatClient::new(network_policy)
            .map_err(map_planetary_computer_error)?;
        Ok(Self {
            resolver_id: PLANETARY_COMPUTER_HLS_RESOLVER_ID,
            signer: Arc::new(client),
        })
    }

    #[doc(hidden)]
    pub fn with_client(client: PlanetaryComputerLandsatClient) -> Self {
        Self {
            resolver_id: PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID,
            signer: Arc::new(client),
        }
    }

    #[doc(hidden)]
    pub fn hls_with_client(client: PlanetaryComputerLandsatClient) -> Self {
        Self {
            resolver_id: PLANETARY_COMPUTER_HLS_RESOLVER_ID,
            signer: Arc::new(client),
        }
    }
}

impl AssetResolver for PlanetaryComputerLandsatAssetResolver {
    fn resolver_id(&self) -> &'static str {
        self.resolver_id
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::PublicHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        (descriptor.resolver_id == PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID
            || descriptor.resolver_id == PLANETARY_COMPUTER_HLS_RESOLVER_ID)
            && descriptor.scheme == AssetAccessScheme::PublicHttps
            && !descriptor.requires_credentials
            && descriptor.credential_profile.is_none()
            && descriptor.stable_href.as_deref().is_some_and(|href| {
                validate_planetary_computer_landsat_stable_href(href).is_ok()
                    || is_valid_planetary_computer_hls_href(href)
            })
    }

    fn validate_frozen(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        let href = frozen
            .descriptor
            .stable_href
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let (display_id, url_file_name) = stable_path_identity(href)?;
        let file_name = frozen
            .file_name
            .as_deref()
            .filter(|name| safe_file_name(name) && *name == url_file_name)
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let reference = &frozen.reference;
        let expected_stable_key =
            stable_asset_key(&reference.provider_product_key, &reference.asset_key)
                .map_err(|_| ResolveError::reference_invalid())?;
        let is_hls = is_hls_asset_access(reference, &frozen.descriptor, href);
        if is_hls {
            if !is_valid_planetary_computer_hls_href(href)
                || reference.provider_product_key.is_empty()
                || reference.stable_asset_key != expected_stable_key
            {
                return Err(ResolveError::descriptor_invalid());
            }
            return validate_native_source(reference, &display_id);
        }
        if !is_supported_landsat_source_collection(&reference.provider_id, &reference.collection_id)
            || !valid_identifier(&reference.provider_product_id)
            || reference.provider_product_key.is_empty()
            || !valid_identifier(&reference.asset_key)
            || reference.stable_asset_key != expected_stable_key
            || frozen.expected_size_bytes.is_none_or(|size| size == 0)
            || frozen.checksum_algorithm.is_some()
            || frozen.checksum_value.is_some()
        {
            return Err(ResolveError::descriptor_invalid());
        }
        validate_native_source(reference, &display_id)?;
        validate_planetary_computer_landsat_asset(
            href,
            &display_id,
            &reference.asset_key,
            file_name,
            &frozen.role,
        )
        .map_err(map_planetary_computer_error)
    }

    fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        context.check_cancelled()?;
        if !self.can_resolve(descriptor) {
            return Err(ResolveError::descriptor_invalid());
        }
        let href = descriptor
            .stable_href
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let is_hls = is_hls_asset_access(reference, descriptor, href);
        if is_hls && !is_valid_planetary_computer_hls_href(href) {
            return Err(ResolveError::descriptor_invalid());
        }
        if !is_hls
            && (!is_supported_landsat_source_collection(
                &reference.provider_id,
                &reference.collection_id,
            ) || !valid_identifier(&reference.provider_product_id)
                || !valid_identifier(&reference.asset_key))
        {
            return Err(ResolveError::descriptor_invalid());
        }
        let (display_id, file_name) = stable_path_identity(href)?;
        validate_native_source(reference, &display_id)?;
        if !is_hls {
            validate_landsat_asset_key_and_filename(&display_id, &reference.asset_key, &file_name)?;
        }
        let (url, expires_at) = if is_hls {
            let signed = self
                .signer
                .sign_hls_asset(&reference.collection_id, href, &|| {
                    context.check_cancelled().is_err()
                })
                .map_err(map_planetary_computer_error)?;
            signed.into_parts()
        } else {
            let signed = self
                .signer
                .sign_asset(href, &|| context.check_cancelled().is_err())
                .map_err(map_planetary_computer_error)?;
            signed.into_parts()
        };
        context.check_cancelled()?;
        let host_policy = if is_hls {
            ResolvedHostPolicy::PlanetaryComputerHlsV1
        } else {
            ResolvedHostPolicy::PlanetaryComputerLandsatV1
        };
        Ok(ResolvedAsset::new(url.as_str(), ResolvedMethod::Get)?
            .with_host_policy(host_policy)
            .with_expiry(expires_at)
            .with_suggested_filename(Some(file_name)))
    }

    fn prepare_auth_refresh(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        self.validate_frozen(frozen)?;
        let href = frozen
            .descriptor
            .stable_href
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let is_hls = is_hls_asset_access(&frozen.reference, &frozen.descriptor, href);
        if is_hls {
            self.signer
                .invalidate_hls_sas(&frozen.reference.collection_id)
                .map_err(map_planetary_computer_error)
        } else {
            self.signer
                .invalidate_sas()
                .map_err(map_planetary_computer_error)
        }
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::not_configured())
    }
}

fn is_hls_asset_access(
    reference: &StableAssetReference,
    descriptor: &AssetAccessDescriptor,
    href: &str,
) -> bool {
    is_valid_planetary_computer_hls_href(href)
        || descriptor.resolver_id == PLANETARY_COMPUTER_HLS_RESOLVER_ID
        || (reference.provider_id == insar_provider::catalog::MPC_HLS_PROVIDER_ID
            && (reference.collection_id == "hls2-l30"
                || reference.collection_id == "hls2-s30"
                || reference.provider_product_id.starts_with("HLS.")))
}

fn is_supported_landsat_source_collection(provider_id: &str, collection_id: &str) -> bool {
    CollectionProfileRegistry
        .for_source(LANDSAT_C2_L2_LOGICAL_DATASET, provider_id, collection_id)
        .is_some()
        || (provider_id == LANDSAT_C2_L2_PROVIDER_ID
            && matches!(collection_id, "landsat_tm_c2_l2" | "landsat_etm_c2_l2"))
}

fn validate_landsat_asset_key_and_filename(
    display_id: &str,
    asset_key: &str,
    file_name: &str,
) -> Result<(), ResolveError> {
    let parts = display_id.split('_').collect::<Vec<_>>();
    let [mission, processing_level, ..] = parts.as_slice() else {
        return Err(ResolveError::descriptor_invalid());
    };
    let definitions = asset_definitions_for_sensor(mission, processing_level)
        .map_err(map_planetary_computer_error)?;
    let definition = definitions
        .into_iter()
        .find(|def| def.key == asset_key)
        .ok_or_else(ResolveError::descriptor_invalid)?;
    let expected_file_name = format!("{display_id}{}", definition.suffix);
    if file_name != expected_file_name {
        return Err(ResolveError::descriptor_invalid());
    }
    Ok(())
}

fn validate_native_source(
    reference: &StableAssetReference,
    display_id: &str,
) -> Result<(), ResolveError> {
    if reference.provider_id == insar_provider::catalog::MPC_HLS_PROVIDER_ID
        && (reference.collection_id == "hls2-l30"
            || reference.collection_id == "hls2-s30"
            || reference.provider_product_id.starts_with("HLS."))
    {
        let expected = insar_core::provider_product_key(
            &reference.provider_id,
            &reference.collection_id,
            &reference.provider_product_id,
        )
        .map_err(|_| ResolveError::reference_invalid())?;
        if expected != reference.provider_product_key {
            return Err(ResolveError::reference_invalid());
        }
        return Ok(());
    }
    if reference.provider_id == MPC_LANDSAT_PROVIDER_ID {
        insar_provider::planetary_computer::validate_planetary_computer_landsat_source_id(
            &reference.provider_product_id,
            display_id,
        )
        .map_err(|_| ResolveError::reference_invalid())?;
        let expected = insar_core::provider_product_key(
            &reference.provider_id,
            &reference.collection_id,
            &reference.provider_product_id,
        )
        .map_err(|_| ResolveError::reference_invalid())?;
        if expected != reference.provider_product_key {
            return Err(ResolveError::reference_invalid());
        }
    }
    Ok(())
}

fn stable_path_identity(href: &str) -> Result<(String, String), ResolveError> {
    let url = Url::parse(href).map_err(|_| ResolveError::descriptor_invalid())?;
    if is_valid_planetary_computer_hls_href(href) {
        let segments = url
            .path_segments()
            .ok_or_else(ResolveError::descriptor_invalid)?
            .collect::<Vec<_>>();
        let [.., display_id, file_name] = segments.as_slice() else {
            return Err(ResolveError::descriptor_invalid());
        };
        return Ok(((*display_id).to_owned(), (*file_name).to_owned()));
    }
    validate_planetary_computer_landsat_stable_href(href).map_err(map_planetary_computer_error)?;
    let segments = url
        .path_segments()
        .ok_or_else(ResolveError::descriptor_invalid)?
        .collect::<Vec<_>>();
    let [.., display_id, file_name] = segments.as_slice() else {
        return Err(ResolveError::descriptor_invalid());
    };
    if !valid_identifier(display_id) || !safe_file_name(file_name) {
        return Err(ResolveError::descriptor_invalid());
    }
    Ok(((*display_id).to_owned(), (*file_name).to_owned()))
}

fn safe_file_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value == value.trim()
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('\0')
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value == value.trim()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn map_planetary_computer_error(error: PlanetaryComputerError) -> ResolveError {
    match error.code() {
        PlanetaryComputerErrorCode::Cancelled => ResolveError::cancelled(),
        PlanetaryComputerErrorCode::ConfigInvalid
        | PlanetaryComputerErrorCode::IdentityMismatch
        | PlanetaryComputerErrorCode::AssetSetInvalid
        | PlanetaryComputerErrorCode::SizeMissing => ResolveError::descriptor_invalid(),
        PlanetaryComputerErrorCode::TransportFailed
        | PlanetaryComputerErrorCode::HttpRejected
        | PlanetaryComputerErrorCode::ResponseInvalid
        | PlanetaryComputerErrorCode::ResponseTooLarge
        | PlanetaryComputerErrorCode::AssetUnavailable => ResolveError::failed(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use insar_core::{provider_product_key, AssetResolverParameters, AssetRole, ProxyPolicy};

    use super::*;
    use crate::{
        AssetResolverRegistry, Clock, CredentialMaterial, CredentialProvider,
        CredentialProviderError,
    };

    const ENTITY_ID: &str = "LC90130352023085LGN00";
    const FILE_NAME: &str = "LC09_L2SP_013035_20230326_20230328_02_T1_SR_B2.TIF";
    const STABLE_HREF: &str = "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/013/035/LC09_L2SP_013035_20230326_20230328_02_T1/LC09_L2SP_013035_20230326_20230328_02_T1_SR_B2.TIF";

    struct NoCredentials;

    impl CredentialProvider for NoCredentials {
        fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
            Err(CredentialProviderError::Missing)
        }
    }

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(1)
        }
    }

    struct FixtureSigner;

    impl AssetSigningPort for FixtureSigner {
        fn sign_asset(
            &self,
            stable_href: &str,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<PlanetaryComputerSignedAsset, PlanetaryComputerError> {
            validate_planetary_computer_landsat_stable_href(stable_href)?;
            let mut url = Url::parse(stable_href).unwrap();
            url.set_query(Some(
                "sp=r&se=2099-01-01T00%3A00%3A00Z&sv=1&sr=c&sig=fixture-secret",
            ));
            PlanetaryComputerSignedAsset::new(
                url,
                SystemTime::UNIX_EPOCH + Duration::from_secs(100),
            )
        }

        fn sign_hls_asset(
            &self,
            collection_id: &str,
            stable_href: &str,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<PlanetaryComputerSignedAsset, PlanetaryComputerError> {
            assert!(
                matches!(collection_id, "hls2-l30" | "hls2-s30"),
                "sign_hls_asset received non-HLS collection_id: {collection_id}"
            );
            let mut url = Url::parse(stable_href).unwrap();
            insar_provider::planetary_computer::validate_hls_asset_origin_and_path(&url)?;
            url.set_query(Some(
                "st=2026-11-09T00%3A00%3A00Z&se=2099-01-01T00%3A00%3A00Z&sp=rl&sv=2025-07-05&sr=c&sig=fixture-secret",
            ));
            PlanetaryComputerSignedAsset::new(
                url,
                SystemTime::UNIX_EPOCH + Duration::from_secs(100),
            )
        }

        fn invalidate_sas(&self) -> Result<(), PlanetaryComputerError> {
            Ok(())
        }

        fn invalidate_hls_sas(&self, _collection_id: &str) -> Result<(), PlanetaryComputerError> {
            Ok(())
        }
    }

    fn resolver() -> PlanetaryComputerLandsatAssetResolver {
        PlanetaryComputerLandsatAssetResolver {
            resolver_id: PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID,
            signer: Arc::new(FixtureSigner),
        }
    }

    fn frozen() -> FrozenAssetAccess {
        let product_key =
            provider_product_key("usgs.eros.m2m", "landsat_ot_c2_l2", ENTITY_ID).unwrap();
        FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: "usgs.eros.m2m".to_owned(),
                collection_id: "landsat_ot_c2_l2".to_owned(),
                provider_product_id: ENTITY_ID.to_owned(),
                provider_product_key: product_key.clone(),
                canonical_product_key: None,
                asset_key: "blue".to_owned(),
                stable_asset_key: stable_asset_key(&product_key, "blue").unwrap(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID.to_owned(),
                scheme: AssetAccessScheme::PublicHttps,
                stable_href: Some(STABLE_HREF.to_owned()),
                requires_credentials: false,
                credential_profile: None,
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(FILE_NAME.to_owned()),
            expected_size_bytes: Some(87_750_834),
            checksum_algorithm: None,
            checksum_value: None,
        }
    }

    #[test]
    fn exact_frozen_asset_resolves_without_credentials_and_redacts_sas() {
        let frozen = frozen();
        let resolver = resolver();
        resolver.validate_frozen(&frozen).unwrap();
        let mut registry = AssetResolverRegistry::new();
        registry.register(Arc::new(resolver)).unwrap();
        let resolved = registry
            .resolve_frozen(
                &frozen,
                &ResolveContext::new(&NoCredentials, &|| false, &FixedClock)
                    .with_proxy_policy(ProxyPolicy::System),
            )
            .unwrap();
        assert_eq!(resolved.expected_size(), Some(87_750_834));
        assert_eq!(resolved.suggested_filename(), Some(FILE_NAME));
        assert_eq!(resolved.asset_role(), Some(&AssetRole::Primary));
        let debug = format!("{resolved:?}");
        assert!(!debug.contains("fixture-secret"));
        assert!(!debug.contains("sig="));
    }

    #[test]
    fn frozen_scene_asset_and_role_drift_fail_closed() {
        let resolver = resolver();
        let mut wrong = frozen();
        wrong.reference.asset_key = "red".to_owned();
        wrong.reference.stable_asset_key =
            stable_asset_key(&wrong.reference.provider_product_key, "red").unwrap();
        assert!(resolver.validate_frozen(&wrong).is_err());

        let mut wrong = frozen();
        wrong.role = AssetRole::Metadata;
        assert!(resolver.validate_frozen(&wrong).is_err());

        let mut wrong = frozen();
        wrong.file_name = Some(format!("../{FILE_NAME}"));
        assert!(resolver.validate_frozen(&wrong).is_err());

        let mut wrong = frozen();
        wrong.descriptor.stable_href = Some(STABLE_HREF.replace("/013/035/", "/014/035/"));
        assert!(resolver.validate_frozen(&wrong).is_err());
    }

    #[test]
    fn native_microsoft_source_requires_exact_native_id_and_computed_product_key() {
        let mut native = frozen();
        let reference = &mut native.reference;
        reference.provider_id = MPC_LANDSAT_PROVIDER_ID.to_owned();
        reference.collection_id = insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID.to_owned();
        reference.provider_product_id = "LC09_L2SP_013035_20230326_02_T1".to_owned();
        reference.provider_product_key = provider_product_key(
            &reference.provider_id,
            &reference.collection_id,
            &reference.provider_product_id,
        )
        .unwrap();
        reference.stable_asset_key =
            stable_asset_key(&reference.provider_product_key, &reference.asset_key).unwrap();
        let resolver = resolver();
        resolver.validate_frozen(&native).unwrap();
        let mut registry = AssetResolverRegistry::new();
        registry.register(Arc::new(resolver)).unwrap();
        let resolved = registry
            .resolve_frozen(
                &native,
                &ResolveContext::new(&NoCredentials, &|| false, &FixedClock)
                    .with_proxy_policy(ProxyPolicy::System),
            )
            .unwrap();
        assert_eq!(resolved.expected_size(), native.expected_size_bytes);
        assert!(!format!("{resolved:?}").contains("fixture-secret"));
        for mutation in 0..3 {
            let mut invalid = native.clone();
            match mutation {
                0 => invalid.reference.provider_product_id = ENTITY_ID.to_owned(),
                1 => invalid.reference.collection_id = "landsat_ot_c2_l2".to_owned(),
                _ => {
                    invalid.reference.provider_product_key =
                        "PROVIDER::other::ITEM::forged".to_owned();
                    invalid.reference.stable_asset_key =
                        stable_asset_key(&invalid.reference.provider_product_key, "blue").unwrap();
                }
            }
            assert!(registry
                .resolve_frozen(
                    &invalid,
                    &ResolveContext::new(&NoCredentials, &|| false, &FixedClock)
                )
                .is_err());
        }
    }

    #[test]
    fn descriptor_is_exact_anonymous_public_https() {
        let resolver = resolver();
        let mut descriptor = frozen().descriptor;
        assert!(resolver.can_resolve(&descriptor));
        descriptor.requires_credentials = true;
        descriptor.credential_profile = Some("unexpected".to_owned());
        assert!(!resolver.can_resolve(&descriptor));
    }

    #[test]
    fn hls_cog_asset_resolves_with_sas_query() {
        let resolver = resolver();
        let hls_href = "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2023/04/30/HLS.L30.T50TMK.2023120T024545.v2.0/HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif";
        let descriptor = AssetAccessDescriptor {
            resolver_id: PLANETARY_COMPUTER_HLS_RESOLVER_ID.to_owned(),
            scheme: AssetAccessScheme::PublicHttps,
            requires_credentials: false,
            credential_profile: None,
            stable_href: Some(hls_href.to_owned()),
            resolver_parameters: AssetResolverParameters {},
        };
        assert!(resolver.can_resolve(&descriptor));
        let pkey = provider_product_key(
            insar_provider::catalog::MPC_HLS_PROVIDER_ID,
            "hls2-l30",
            "HLS.L30.T50TMK.2023120T024545.v2.0",
        )
        .unwrap();
        let reference = StableAssetReference {
            provider_id: insar_provider::catalog::MPC_HLS_PROVIDER_ID.to_owned(),
            collection_id: "hls2-l30".to_owned(),
            provider_product_id: "HLS.L30.T50TMK.2023120T024545.v2.0".to_owned(),
            provider_product_key: pkey.clone(),
            asset_key: "B02".to_owned(),
            stable_asset_key: stable_asset_key(&pkey, "B02").unwrap(),
            canonical_product_key: None,
        };
        let file_name = "HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif";
        let frozen = FrozenAssetAccess {
            descriptor: descriptor.clone(),
            reference: reference.clone(),
            role: AssetRole::Primary,
            file_name: Some(file_name.to_owned()),
            expected_size_bytes: Some(15_000_000),
            checksum_algorithm: None,
            checksum_value: None,
        };
        resolver.validate_frozen(&frozen).unwrap();
        let resolved = resolver
            .resolve(
                &reference,
                &descriptor,
                &ResolveContext::new(&NoCredentials, &|| false, &FixedClock),
            )
            .unwrap();
        assert_eq!(resolved.suggested_filename(), Some(file_name));
        assert!(resolved
            .effective_url()
            .as_str()
            .contains("hls2euwest.blob.core.windows.net"));
        assert!(resolved.effective_url().as_str().contains("sig="));
        assert!(resolved.effective_url().as_str().contains("sp=rl"));
    }

    #[test]
    fn lt04_lt05_and_le07_assets_validate_manifest_and_resolve_signed_downloads() {
        use crate::execution_policy_registry::build_planetary_computer_landsat_asset_access;
        use insar_core::SnapshotItem;
        use insar_provider::planetary_computer::{
            PlanetaryComputerLandsatAsset, PlanetaryComputerLandsatManifest,
        };
        use serde_json::{json, Map};

        let resolver = resolver();
        let mut registry = AssetResolverRegistry::new();
        registry.register(Arc::new(resolver)).unwrap();

        let scenes = [
            (
                "LT04_L2SP_013045_19931118_02_T2",
                "LT04_L2SP_013045_19931118_20200913_02_T2",
                "tm",
                "1993/013/045",
                "l2sp",
                "landsat-4",
            ),
            (
                "LT05_L2SP_038037_20120505_02_T1",
                "LT05_L2SP_038037_20120505_20200820_02_T1",
                "tm",
                "2012/038/037",
                "l2sp",
                "landsat-5",
            ),
            (
                "LE07_L2SP_135047_20240119_02_T1",
                "LE07_L2SP_135047_20240119_20240213_02_T1",
                "etm",
                "2024/135/047",
                "l2sp",
                "landsat-7",
            ),
            (
                "LE07_L2SR_071086_20240118_02_T2",
                "LE07_L2SR_071086_20240118_20240227_02_T2",
                "etm",
                "2024/071/086",
                "l2sr",
                "landsat-7",
            ),
        ];

        for (stac_id, display_id, folder, path_row_year, level_lower, platform) in scenes {
            let defs =
                asset_definitions_for_sensor(platform, &level_lower.to_ascii_uppercase()).unwrap();
            let mut assets = Vec::new();
            let mut total_size = 0_u64;
            for (idx, def) in defs.iter().enumerate() {
                let sz = 1_000_000 + (idx as u64) * 1024;
                total_size += sz;
                let file_name = format!("{display_id}{}", def.suffix);
                let stable_href = format!(
                    "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/{folder}/{path_row_year}/{display_id}/{file_name}"
                );
                assets.push(PlanetaryComputerLandsatAsset {
                    asset_key: def.key.to_owned(),
                    file_name,
                    role: def.role.clone(),
                    stable_href,
                    expected_size_bytes: sz,
                });
            }

            let manifest = PlanetaryComputerLandsatManifest {
                entity_id: stac_id.to_owned(),
                display_id: display_id.to_owned(),
                stac_item_id: stac_id.to_owned(),
                processing_level: level_lower.to_owned(),
                assets,
                total_size_bytes: total_size,
            };

            let product_key = provider_product_key(
                MPC_LANDSAT_PROVIDER_ID,
                insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID,
                stac_id,
            )
            .unwrap();
            let mut metadata = Map::new();
            metadata.insert(
                "logical_dataset".to_owned(),
                json!(LANDSAT_C2_L2_LOGICAL_DATASET),
            );
            metadata.insert(
                "native_collection_id".to_owned(),
                json!(insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID),
            );
            metadata.insert(
                "collection_profile_id".to_owned(),
                json!(insar_provider::catalog::MPC_LANDSAT_SCENE_PROFILE_ID),
            );
            metadata.insert(
                "identity_rule_id".to_owned(),
                json!(insar_provider::catalog::MPC_LANDSAT_IDENTITY_RULE_ID),
            );
            metadata.insert("delivery_model".to_owned(), json!("archive_first"));
            metadata.insert("provider_product_id".to_owned(), json!(stac_id));
            metadata.insert("provider_product_key".to_owned(), json!(product_key));
            metadata.insert(
                "properties".to_owned(),
                json!({
                    "entity_id": stac_id,
                    "landsat_product_id": display_id,
                    "dataset_alias": insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID,
                    "processing_level": level_lower,
                }),
            );
            let snapshot_item = SnapshotItem {
                item_id: product_key,
                provider_id: MPC_LANDSAT_PROVIDER_ID.to_owned(),
                display_name: display_id.to_owned(),
                acquisition_datetime: Some("2012-05-05T18:00:00Z".to_owned()),
                footprint_geojson: None,
                metadata,
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            };

            // Verify band selection and full manifest resolution for TM/ETM+ keys
            for target_key in [
                "blue",
                "red",
                "nir08",
                "atmos_opacity",
                "cloud_qa",
                "mtl.txt",
            ] {
                let target_asset = manifest
                    .assets
                    .iter()
                    .find(|a| a.asset_key == target_key)
                    .expect("expected TM/ETM+ asset key");
                let frozen_access = build_planetary_computer_landsat_asset_access(
                    &snapshot_item,
                    &manifest,
                    target_asset,
                )
                .expect("manifest + asset access must build cleanly for LT04/LT05/LE07");
                let resolved = registry
                    .resolve_frozen(
                        &frozen_access,
                        &ResolveContext::new(&NoCredentials, &|| false, &FixedClock)
                            .with_proxy_policy(ProxyPolicy::System),
                    )
                    .expect("LT04/LT05/LE07 asset must resolve signed download URL");
                assert_eq!(
                    resolved.suggested_filename(),
                    Some(target_asset.file_name.as_str())
                );
                assert_eq!(
                    resolved.expected_size(),
                    Some(target_asset.expected_size_bytes)
                );
            }
            if level_lower == "l2sp" {
                let lwir_asset = manifest
                    .assets
                    .iter()
                    .find(|a| a.asset_key == "lwir")
                    .expect("lwir (_ST_B6.TIF) must exist on L2SP TM/ETM+");
                let frozen_lwir = build_planetary_computer_landsat_asset_access(
                    &snapshot_item,
                    &manifest,
                    lwir_asset,
                )
                .expect("lwir asset access must build cleanly");
                let resolved_lwir = registry
                    .resolve_frozen(
                        &frozen_lwir,
                        &ResolveContext::new(&NoCredentials, &|| false, &FixedClock),
                    )
                    .expect("lwir must resolve");
                assert!(resolved_lwir
                    .suggested_filename()
                    .unwrap()
                    .ends_with("_ST_B6.TIF"));
            }
        }
    }
}
