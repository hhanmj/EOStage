use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use insar_core::{AssetRole, FrozenAssetAccess};
use insar_download::{
    AssetResolverRegistry, Clock, LegacyTransferIntent, ShadowAssetClass, ShadowAuthKind,
    ShadowCredentialShapeProvider, ShadowDestinationPolicy, ShadowHostClass, ShadowMismatchCode,
    ShadowPathClass, ShadowProviderClass, ShadowResolveContext, TransferIntentFingerprint,
};
use serde_json::Value;

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct FixtureShapes;

impl ShadowCredentialShapeProvider for FixtureShapes {
    fn auth_kind(&self, profile: &str) -> Result<ShadowAuthKind, insar_download::ResolveError> {
        match profile {
            "earthdata-default" => Ok(ShadowAuthKind::EarthdataBasic),
            "earthdata-burst-session" => Ok(ShadowAuthKind::EarthdataSessionCookie),
            _ => Err(insar_download::ResolveError::credentials_missing()),
        }
    }
}

struct FixtureCase {
    asset_class: ShadowAssetClass,
    frozen: FrozenAssetAccess,
    legacy: LegacyTransferIntent,
    policy: ShadowDestinationPolicy,
}

fn fixture(name: &str) -> FixtureCase {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asset_resolver_shadow")
        .join(name);
    let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let legacy = &value["legacy"];
    let policy = &value["policy"];
    FixtureCase {
        asset_class: asset_class(value["asset_class"].as_str().unwrap()),
        frozen: serde_json::from_value(value["frozen"].clone()).unwrap(),
        legacy: LegacyTransferIntent {
            provider_id: legacy["provider_id"].as_str().unwrap().to_string(),
            url: legacy["url"].as_str().unwrap().to_string(),
            file_name: legacy["file_name"].as_str().unwrap().to_string(),
            auth_kind: auth_kind(legacy["auth_kind"].as_str().unwrap()),
            expected_size: legacy["expected_size"].as_u64(),
            checksum_algorithm: legacy["checksum_algorithm"].as_str().map(str::to_string),
            checksum_value: legacy["checksum_value"].as_str().map(str::to_string),
            destination: PathBuf::from(legacy["destination"].as_str().unwrap()),
            destination_layout: serde_json::from_value(legacy["destination_layout"].clone())
                .unwrap(),
            asset_role: serde_json::from_value(legacy["asset_role"].clone()).unwrap(),
        },
        policy: ShadowDestinationPolicy::new(
            PathBuf::from(policy["output_root"].as_str().unwrap()),
            serde_json::from_value(policy["layout"].clone()).unwrap(),
            policy["dataset"].as_str().unwrap(),
            policy["processing_level"].as_str().unwrap(),
        ),
    }
}

fn asset_class(value: &str) -> ShadowAssetClass {
    match value {
        "sentinel1_standard" => ShadowAssetClass::Sentinel1Standard,
        "burst_tiff" => ShadowAssetClass::BurstTiff,
        "burst_xml" => ShadowAssetClass::BurstXml,
        "historical_asf" => ShadowAssetClass::HistoricalAsf,
        "orbit_direct_eof" => ShadowAssetClass::OrbitDirectEof,
        _ => panic!("unknown fixture asset class"),
    }
}

fn auth_kind(value: &str) -> ShadowAuthKind {
    match value {
        "none" => ShadowAuthKind::None,
        "earthdata_bearer" => ShadowAuthKind::EarthdataBearer,
        "earthdata_basic" => ShadowAuthKind::EarthdataBasic,
        "earthdata_session_cookie" => ShadowAuthKind::EarthdataSessionCookie,
        _ => panic!("unknown fixture auth kind"),
    }
}

fn fingerprints(case: &FixtureCase) -> (TransferIntentFingerprint, TransferIntentFingerprint) {
    let shapes = FixtureShapes;
    let cancelled = || false;
    let clock = FixedClock;
    let context =
        ShadowResolveContext::new(&shapes, &cancelled, &clock, Duration::from_millis(25)).unwrap();
    let intent = AssetResolverRegistry::with_standard_resolvers()
        .unwrap()
        .plan_shadow(&case.frozen, &context)
        .unwrap();
    let destination = case
        .policy
        .destination_for(intent.file_name().unwrap())
        .unwrap();
    (
        TransferIntentFingerprint::from_legacy(&case.legacy).unwrap(),
        TransferIntentFingerprint::from_shadow(&intent, destination, case.policy.layout().clone())
            .unwrap(),
    )
}

#[test]
fn all_five_offline_asset_fixtures_have_shadow_parity() {
    for (name, expected_class) in [
        (
            "sentinel1_standard.json",
            ShadowAssetClass::Sentinel1Standard,
        ),
        ("burst_tiff.json", ShadowAssetClass::BurstTiff),
        ("burst_xml.json", ShadowAssetClass::BurstXml),
        ("historical_asf.json", ShadowAssetClass::HistoricalAsf),
        ("orbit_direct_eof.json", ShadowAssetClass::OrbitDirectEof),
    ] {
        let case = fixture(name);
        assert_eq!(case.asset_class, expected_class);
        let (legacy, shadow) = fingerprints(&case);
        assert_eq!(
            legacy.compare(&shadow),
            Vec::<ShadowMismatchCode>::new(),
            "{name}"
        );
        assert!(!format!("{legacy:?}").contains("fixture-output"));
        assert!(!format!("{shadow:?}").contains("fixture-sha256"));
    }
}

#[test]
fn one_changed_semantic_field_produces_only_its_bounded_code() {
    let case = fixture("sentinel1_standard.json");
    let (legacy, shadow) = fingerprints(&case);
    let mutations = [
        (
            shadow
                .clone()
                .with_provider_class(ShadowProviderClass::Other),
            ShadowMismatchCode::Provider,
        ),
        (
            shadow
                .clone()
                .with_host_class(ShadowHostClass::PublicHttpsOther),
            ShadowMismatchCode::HostClass,
        ),
        (
            shadow.clone().with_path_class(ShadowPathClass::NativeFile),
            ShadowMismatchCode::PathClass,
        ),
        (
            shadow
                .clone()
                .with_file_name(Some("different.zip".to_string())),
            ShadowMismatchCode::Filename,
        ),
        (
            shadow
                .clone()
                .with_auth_kind(ShadowAuthKind::EarthdataBearer),
            ShadowMismatchCode::AuthKind,
        ),
        (
            shadow.clone().with_expected_size(Some(1)),
            ShadowMismatchCode::ExpectedSize,
        ),
        (
            shadow.clone().with_checksum(
                Some("md5".to_string()),
                shadow.checksum_value().map(str::to_string),
            ),
            ShadowMismatchCode::Checksum,
        ),
        (
            shadow.clone().with_checksum(
                shadow.checksum_algorithm().map(str::to_string),
                Some("different".to_string()),
            ),
            ShadowMismatchCode::Checksum,
        ),
        (
            shadow
                .clone()
                .with_destination(PathBuf::from("other/output.zip")),
            ShadowMismatchCode::Destination,
        ),
        (
            shadow.clone().with_asset_role(AssetRole::Metadata),
            ShadowMismatchCode::AssetRole,
        ),
    ];
    for (mutated, expected) in mutations {
        assert_eq!(legacy.compare(&mutated), vec![expected]);
    }
}
