use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use insar_core::{
    canonical_product_key, dns_name_matches, provider_product_key, stable_asset_key,
    DownloadOutcome, SnapshotItem,
};
use insar_download::{
    classify_frozen_execution, part_path, verify_file_checksum, AsfCollectionProfileRegistry,
    AssetExecutionPolicyRegistry, EligibleResolverAsset, ExpectedChecksum, FrozenExecution,
    HttpTransfer, ResolverExecutionRuntime, TransferOptions,
};
use serde_json::json;

const REQUIRED_INPUTS: [&str; 7] = [
    "INSAR_REAL_ASF_STABLE_HREF",
    "INSAR_REAL_ASF_PRODUCT_ID",
    "INSAR_REAL_ASF_FILE_NAME",
    "INSAR_REAL_ASF_EXPECTED_SIZE",
    "INSAR_REAL_ASF_CHECKSUM_ALGORITHM",
    "INSAR_REAL_ASF_CHECKSUM_VALUE",
    "INSAR_REAL_ASF_OUTPUT_DIR",
];

struct RealAsfAcceptanceInput {
    product_id: String,
    file_name: String,
    expected_size: u64,
    checksum: ExpectedChecksum,
    output_dir: PathBuf,
    resume_validation: bool,
    eligible: Box<EligibleResolverAsset>,
}

impl RealAsfAcceptanceInput {
    fn from_mapping(values: &BTreeMap<String, String>) -> Result<Self, &'static str> {
        let stable_href = required_mapping(values, "INSAR_REAL_ASF_STABLE_HREF")?;
        let product_id = required_mapping(values, "INSAR_REAL_ASF_PRODUCT_ID")?;
        let file_name = required_mapping(values, "INSAR_REAL_ASF_FILE_NAME")?;
        let expected_size = required_mapping(values, "INSAR_REAL_ASF_EXPECTED_SIZE")?
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or("invalid expected size")?;
        let checksum_algorithm = required_mapping(values, "INSAR_REAL_ASF_CHECKSUM_ALGORITHM")?;
        let checksum_value = required_mapping(values, "INSAR_REAL_ASF_CHECKSUM_VALUE")?;
        let checksum = ExpectedChecksum::new(&checksum_algorithm, &checksum_value)
            .map_err(|_| "invalid checksum declaration")?;
        let output_dir = PathBuf::from(required_mapping(values, "INSAR_REAL_ASF_OUTPUT_DIR")?);
        if !output_dir.is_dir() {
            return Err("output directory does not exist");
        }
        let resume_validation = match values
            .get("INSAR_REAL_ASF_RESUME_VALIDATION")
            .map(|value| value.trim())
        {
            None | Some("0") => false,
            Some("1") => true,
            Some(_) => return Err("invalid resume validation flag"),
        };
        if product_id.len() > 512
            || product_id.chars().any(char::is_control)
            || !safe_basename(&file_name)
        {
            return Err("invalid product or file identity");
        }

        let url = reqwest::Url::parse(&stable_href).map_err(|_| "invalid stable href")?;
        let approved_host = url.host_str().is_some_and(|host| {
            [
                "earthdata.nasa.gov",
                "earthdatacloud.nasa.gov",
                "asf.alaska.edu",
            ]
            .iter()
            .any(|allowed| dns_name_matches(host, allowed))
        });
        let url_file_name = url
            .path_segments()
            .and_then(|mut segments| segments.rfind(|part| !part.is_empty()));
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() != Some(443)
            || !approved_host
            || url_file_name != Some(file_name.as_str())
        {
            return Err("stable href is not an approved exact asset URL");
        }

        let provider_key = provider_product_key("asf.catalog", "SENTINEL-1", &product_id)
            .map_err(|_| "invalid product identity")?;
        let canonical_key = canonical_product_key("ASF", "SENTINEL-1", &product_id)
            .map_err(|_| "invalid canonical identity")?;
        let frozen_asset_key =
            stable_asset_key(&provider_key, &file_name).map_err(|_| "invalid asset identity")?;
        let item: SnapshotItem = serde_json::from_value(json!({
            "item_id": provider_key,
            "provider_id": "asf.catalog",
            "display_name": file_name,
            "metadata": {
                "dataset": "SENTINEL-1",
                "product_family": "SLC",
                "processing_level": "SLC",
                "identity_rule_id": "asf.sentinel-1.standard.v1",
                "provider_product_id": product_id,
                "provider_product_key": provider_key,
                "canonical_product_key": canonical_key
            },
            "payload": {},
            "asset_access": {
                "reference": {
                    "provider_id": "asf.catalog",
                    "collection_id": "SENTINEL-1",
                    "provider_product_id": product_id,
                    "provider_product_key": provider_key,
                    "canonical_product_key": canonical_key,
                    "asset_key": file_name,
                    "stable_asset_key": frozen_asset_key
                },
                "descriptor": {
                    "resolver_id": "earthdata_https",
                    "scheme": "earthdata_https",
                    "stable_href": stable_href,
                    "requires_credentials": true,
                    "credential_profile": "earthdata-default",
                    "resolver_parameters": {}
                },
                "role": "primary",
                "file_name": file_name,
                "expected_size_bytes": expected_size,
                "checksum_algorithm": checksum.algorithm().name(),
                "checksum_value": checksum.value()
            },
            "execution_policy": {
                "authority": "resolver_authoritative",
                "policy_id": "asf.sentinel-1.standard.resolver-cutover.v1",
                "collection_profile_id": "asf.sentinel-1.standard.v1"
            }
        }))
        .map_err(|_| "invalid frozen acceptance item")?;
        let FrozenExecution::ResolverAuthoritative(eligible) = classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ) else {
            return Err("acceptance item is not Resolver eligible");
        };

        Ok(Self {
            product_id,
            file_name,
            expected_size,
            checksum,
            output_dir,
            resume_validation,
            eligible,
        })
    }

    fn from_environment() -> Result<Self, &'static str> {
        let mut values = REQUIRED_INPUTS
            .into_iter()
            .map(|name| (name.to_string(), required_env(name)))
            .collect::<BTreeMap<_, _>>();
        if let Ok(value) = std::env::var("INSAR_REAL_ASF_RESUME_VALIDATION") {
            values.insert("INSAR_REAL_ASF_RESUME_VALIDATION".to_string(), value);
        }
        Self::from_mapping(&values)
    }
}

fn required_mapping(
    values: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<String, &'static str> {
    values
        .get(name)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or("missing required acceptance input")
}

fn required_env(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("missing required acceptance input: {name}"))
}

fn safe_basename(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\0')
        && !value.contains('/')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && value != "."
        && value != ".."
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResumeKind {
    Range,
    ControlledRestart,
}

struct RealAsfAcceptanceReport {
    cancelled_part_bytes: u64,
    resume_kind: ResumeKind,
    final_size: u64,
    expected_size: u64,
    frozen_checksum_algorithm: String,
    frozen_checksum_value_matched: bool,
    full_checksum_verified: bool,
}

fn run_real_asf_resolver_acceptance(
    input: RealAsfAcceptanceInput,
) -> Result<RealAsfAcceptanceReport, &'static str> {
    let runtime =
        ResolverExecutionRuntime::production().map_err(|_| "Resolver runtime unavailable")?;
    let options = TransferOptions {
        chunk_size: 64 * 1024,
        ..TransferOptions::default()
    };
    let transfer = HttpTransfer::new(options).map_err(|_| "transfer runtime unavailable")?;
    let destination = input.output_dir.join(&input.file_name);
    let partial = part_path(&destination);
    if destination.exists() {
        return Err("acceptance final destination already exists");
    }

    let cancelled_part_bytes = if input.resume_validation {
        fs::metadata(&partial)
            .map(|metadata| metadata.len())
            .map_err(|_| "resume validation requires an existing partial file")?
    } else {
        if partial.exists() {
            return Err("acceptance destination is not empty");
        }
        let cancel_after = (512 * 1024_u64).min(input.expected_size.saturating_sub(1));
        if cancel_after < 128 * 1024 {
            return Err("acceptance product is too small for cancellation evidence");
        }
        let cancelled = AtomicBool::new(false);
        let first = runtime.execute_resolver_authoritative(
            &transfer,
            &input.eligible,
            "real-asf-resolver-acceptance",
            &input.product_id,
            destination.clone(),
            &|| cancelled.load(Ordering::SeqCst),
            &|| false,
            &mut |update| {
                if let insar_download::TransferUpdate::Progress(progress) = update {
                    if progress.bytes_completed >= cancel_after {
                        cancelled.store(true, Ordering::SeqCst);
                    }
                }
            },
        );
        match first {
            Ok(result)
                if matches!(
                    result.outcome,
                    DownloadOutcome::Interrupted | DownloadOutcome::Cancelled
                ) => {}
            Err(error) if error.code() == "DL001" => {}
            _ => return Err("bounded cancellation did not preserve a partial transfer"),
        }
        fs::metadata(&partial)
            .map(|metadata| metadata.len())
            .map_err(|_| "cancelled partial file is missing")?
    };
    if cancelled_part_bytes == 0 || destination.exists() {
        return Err("cancelled transfer did not leave only a non-empty partial file");
    }

    let mut first_resume_progress = None;
    let resumed = runtime
        .execute_resolver_authoritative(
            &transfer,
            &input.eligible,
            "real-asf-resolver-acceptance",
            &input.product_id,
            destination.clone(),
            &|| false,
            &|| false,
            &mut |update| {
                if let insar_download::TransferUpdate::Progress(progress) = update {
                    first_resume_progress.get_or_insert(progress.bytes_completed);
                }
            },
        )
        .map_err(|_| "resumed Resolver transfer failed")?;
    if resumed.outcome != DownloadOutcome::Completed {
        return Err("resumed Resolver transfer did not complete");
    }
    let first_resume_progress = first_resume_progress.ok_or("resume progress is missing")?;
    let resume_kind = if first_resume_progress > cancelled_part_bytes {
        ResumeKind::Range
    } else {
        ResumeKind::ControlledRestart
    };
    let final_size = fs::metadata(&destination)
        .map(|metadata| metadata.len())
        .map_err(|_| "final acceptance file is missing")?;
    verify_file_checksum(&destination, &input.checksum)
        .map_err(|_| "final full-file checksum verification failed")?;
    if partial.exists() {
        return Err("partial file remains after verified promotion");
    }
    let frozen_checksum_value_matched =
        input.eligible.frozen_access.checksum_value.as_deref() == Some(input.checksum.value());

    Ok(RealAsfAcceptanceReport {
        cancelled_part_bytes,
        resume_kind,
        final_size,
        expected_size: input.expected_size,
        frozen_checksum_algorithm: input.checksum.algorithm().name().to_string(),
        frozen_checksum_value_matched,
        full_checksum_verified: true,
    })
}

fn acceptance_mapping(algorithm: &str, checksum: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "INSAR_REAL_ASF_STABLE_HREF".to_string(),
            "https://datapool.asf.alaska.edu/SLC/fixture.zip".to_string(),
        ),
        (
            "INSAR_REAL_ASF_PRODUCT_ID".to_string(),
            "S1A_FIXTURE_SLC".to_string(),
        ),
        (
            "INSAR_REAL_ASF_FILE_NAME".to_string(),
            "fixture.zip".to_string(),
        ),
        (
            "INSAR_REAL_ASF_EXPECTED_SIZE".to_string(),
            "1048576".to_string(),
        ),
        (
            "INSAR_REAL_ASF_CHECKSUM_ALGORITHM".to_string(),
            algorithm.to_string(),
        ),
        (
            "INSAR_REAL_ASF_CHECKSUM_VALUE".to_string(),
            checksum.to_string(),
        ),
        (
            "INSAR_REAL_ASF_OUTPUT_DIR".to_string(),
            std::env::temp_dir().to_string_lossy().into_owned(),
        ),
    ])
}

#[test]
fn checksum_input_validation() {
    for (algorithm, length) in [("md5", 32), ("sha1", 40), ("sha256", 64), ("sha512", 128)] {
        let mapping = acceptance_mapping(algorithm, &"a".repeat(length));
        let input = RealAsfAcceptanceInput::from_mapping(&mapping).unwrap();
        assert_eq!(input.checksum.algorithm().name(), algorithm);
    }

    for (algorithm, checksum) in [
        ("crc32", "a".repeat(8)),
        ("sha256", "not-hex".to_string()),
        ("sha256", "a".repeat(63)),
    ] {
        assert!(
            RealAsfAcceptanceInput::from_mapping(&acceptance_mapping(algorithm, &checksum,))
                .is_err()
        );
    }

    for href in [
        "http://datapool.asf.alaska.edu/SLC/fixture.zip",
        "https://user@example.com/fixture.zip",
        "https://datapool.asf.alaska.edu/SLC/fixture.zip?signed=fixture",
        "https://datapool.asf.alaska.edu/SLC/fixture.zip#fragment",
        "https://example.com/SLC/fixture.zip",
    ] {
        let mut mapping = acceptance_mapping("sha256", &"a".repeat(64));
        mapping.insert("INSAR_REAL_ASF_STABLE_HREF".to_string(), href.to_string());
        assert!(RealAsfAcceptanceInput::from_mapping(&mapping).is_err());
    }

    let mut unsafe_name = acceptance_mapping("sha256", &"a".repeat(64));
    unsafe_name.insert(
        "INSAR_REAL_ASF_FILE_NAME".to_string(),
        "../fixture.zip".to_string(),
    );
    assert!(RealAsfAcceptanceInput::from_mapping(&unsafe_name).is_err());

    let mut invalid_resume = acceptance_mapping("sha256", &"a".repeat(64));
    invalid_resume.insert(
        "INSAR_REAL_ASF_RESUME_VALIDATION".to_string(),
        "true".to_string(),
    );
    assert!(RealAsfAcceptanceInput::from_mapping(&invalid_resume).is_err());
}

#[test]
#[ignore = "requires explicit Earthdata credentials and approved real product"]
fn real_asf_resolver_download_cancels_resumes_and_verifies() {
    assert_eq!(
        required_env("INSAR_RESOLVER_CUTOVER_ASF_SENTINEL1_STANDARD"),
        "1"
    );
    let input = RealAsfAcceptanceInput::from_environment().unwrap();
    let public_product_id = input.product_id.clone();
    let report = run_real_asf_resolver_acceptance(input).unwrap();
    assert!(report.cancelled_part_bytes > 0);
    assert!(matches!(
        report.resume_kind,
        ResumeKind::Range | ResumeKind::ControlledRestart
    ));
    assert_eq!(report.final_size, report.expected_size);
    assert_eq!(
        report.frozen_checksum_algorithm,
        required_env("INSAR_REAL_ASF_CHECKSUM_ALGORITHM").to_ascii_lowercase(),
    );
    assert!(report.frozen_checksum_value_matched);
    assert!(report.full_checksum_verified);
    eprintln!(
        "product_id={public_product_id} cancelled_bytes={} final_bytes={} resume={:?} algorithm={} checksum_verified={}",
        report.cancelled_part_bytes,
        report.final_size,
        report.resume_kind,
        report.frozen_checksum_algorithm,
        report.full_checksum_verified,
    );
}
