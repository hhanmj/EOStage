use std::{fs, path::PathBuf};

use insar_core::{AssetAccessScheme, AssetRole};
use insar_provider::asf::{map_nisar_rslc_umm_granule, CmrGranuleEnvelope};
use insar_provider::catalog::{
    CollectionProfileRegistry, NISAR_P05023_PRODUCT_SPECIFICATION_VERSION,
    NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION, NISAR_PROFILE_ID,
    NISAR_RESOLVER_POLICY_ID_V2,
};
use insar_provider::preview::{resolve_preview_candidate, PreviewProductClass};
use serde_json::{json, Value};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-nisar-l-p1")
}

fn captured_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asf/nisar/provisional-rslc")
}

fn load(path: PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("fixture exists"))
        .expect("fixture is valid JSON")
}

fn named_fixture(name: &str) -> Value {
    load(fixture_root().join(name))
}

fn base_fixture() -> Value {
    let mut fixture = named_fixture("granule-provisional-multi-asset-valid.json");
    fixture["collection"]["cmr_provider_id"] = json!("ASF");
    fixture
}

fn captured_raw_umm() -> Value {
    load(captured_root().join("granule-partial.json"))["cmr_response"]["items"][0].clone()
}

fn set_raw_attribute_values(raw: &mut Value, name: &str, values: &[&str]) {
    let attributes = raw["umm"]["AdditionalAttributes"]
        .as_array_mut()
        .expect("raw additional attributes");
    let attribute = attributes
        .iter_mut()
        .find(|attribute| attribute["Name"].as_str() == Some(name))
        .expect("captured raw attribute exists");
    attribute["Values"] = json!(values);
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn raw_primary_archive_mut(raw: &mut Value) -> &mut Value {
    let producer = raw["umm"]["GranuleUR"]
        .as_str()
        .expect("raw producer identity")
        .to_owned();
    let expected_name = format!("{producer}.h5");
    raw["umm"]["DataGranule"]["ArchiveAndDistributionInformation"]
        .as_array_mut()
        .expect("raw archives")
        .iter_mut()
        .find(|archive| {
            archive["Name"]
                .as_str()
                .is_some_and(|name| name == expected_name.as_str())
        })
        .expect("raw primary archive")
}

fn remove_primary_related_format_evidence(raw: &mut Value) {
    let producer = raw["umm"]["GranuleUR"]
        .as_str()
        .expect("raw producer identity")
        .to_owned();
    let suffix = format!("{producer}.h5");
    for related in raw["umm"]["RelatedUrls"]
        .as_array_mut()
        .expect("raw related URLs")
    {
        if related["URL"]
            .as_str()
            .is_some_and(|url| url.ends_with(&suffix))
        {
            related
                .as_object_mut()
                .expect("related URL object")
                .remove("Format");
            related
                .as_object_mut()
                .expect("related URL object")
                .remove("MimeType");
        }
    }
}

fn append_raw_archive(raw: &mut Value, name: &str, format: Option<&str>, mime_type: Option<&str>) {
    let mut archive = json!({"Name": name, "SizeInBytes": 1});
    if let Some(format) = format {
        archive["Format"] = json!(format);
    }
    if let Some(mime_type) = mime_type {
        archive["MimeType"] = json!(mime_type);
    }
    raw["umm"]["DataGranule"]["ArchiveAndDistributionInformation"]
        .as_array_mut()
        .expect("raw archives")
        .push(archive);
}

fn first_archive_name_with_suffix(raw: &Value, suffix: &str) -> String {
    raw["umm"]["DataGranule"]["ArchiveAndDistributionInformation"]
        .as_array()
        .expect("raw archives")
        .iter()
        .filter_map(|archive| archive["Name"].as_str())
        .find(|name| name.ends_with(suffix))
        .expect("captured archive suffix")
        .to_owned()
}

fn append_related_case(raw: &mut Value, case: &Value) {
    let producer = raw["umm"]["GranuleUR"]
        .as_str()
        .expect("raw producer identity")
        .to_owned();
    let primary = format!("{producer}.h5");
    let path = case["path"].as_str().expect("matrix path");
    let path = match path {
        "__PRIMARY__" => primary.clone(),
        "__PRIMARY_PERCENT_ENCODED__" => primary.replace(".h5", "%2Eh5"),
        "__EXISTING_PNG__" => first_archive_name_with_suffix(raw, ".png"),
        "__EXISTING_XML__" => first_archive_name_with_suffix(raw, ".xml"),
        "__SECOND_PRIMARY__" => "second-primary.h5".to_owned(),
        value => value.to_owned(),
    };
    let host = case
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or("nisar.asf.earthdatacloud.nasa.gov");
    let locator = format!("https://{host}/synthetic/{path}");
    let mut related = json!({
        "URL": locator,
        "Type": case["relation_type"].clone()
    });
    if let Some(subtype) = case.get("subtype") {
        related["Subtype"] = subtype.clone();
    }
    if let Some(format) = case.get("format") {
        related["Format"] = format.clone();
    }
    if let Some(mime_type) = case.get("mime_type") {
        related["MimeType"] = mime_type.clone();
    }
    raw["umm"]["RelatedUrls"]
        .as_array_mut()
        .expect("raw related URLs")
        .push(related);
}

fn asset_records_mut(value: &mut Value) -> &mut Vec<Value> {
    value["granule"]["assets"]["records"]
        .as_array_mut()
        .expect("asset records")
}

fn apply_mutation(target: &mut Value, mutation: &Value) {
    if let Some(path) = mutation.get("path").and_then(Value::as_str) {
        *target.pointer_mut(path).expect("mutation path exists") = mutation["value"].clone();
        return;
    }
    match mutation["operation"].as_str().expect("known mutation") {
        "append_asset" => asset_records_mut(target).push(mutation["asset"].clone()),
        "remove_asset" => {
            let asset_id = mutation["asset_id"].as_str().expect("asset id");
            asset_records_mut(target).retain(|asset| asset["asset_id"].as_str() != Some(asset_id));
        }
        "remove_roles" => {
            let roles = mutation["roles"]
                .as_array()
                .expect("roles")
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            asset_records_mut(target).retain(|asset| {
                !asset["normalized_role"]
                    .as_str()
                    .is_some_and(|role| roles.contains(&role))
            });
        }
        operation => panic!("unsupported fixture mutation: {operation}"),
    }
}

fn mutated_fixture(name: &str) -> Value {
    let mutation = named_fixture(name);
    let mut target = named_fixture(
        mutation["base"]
            .as_str()
            .expect("mutation names a base fixture"),
    );
    target["collection"]["cmr_provider_id"] = json!("ASF");
    if let Some(mutations) = mutation["mutations"].as_array() {
        for item in mutations {
            apply_mutation(&mut target, item);
        }
    } else if mutation.get("mutation").is_some() {
        apply_mutation(&mut target, &mutation["mutation"]);
    }
    target
}

fn map(value: &Value) -> Result<insar_core::ProviderProduct, insar_core::ContractError> {
    let envelope = CmrGranuleEnvelope::try_from(value)?;
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    map_nisar_rslc_umm_granule(&envelope, profile)
}

#[test]
fn synthetic_provisional_rslc_maps_one_product_and_one_executable_hdf5() {
    let fixture = base_fixture();
    let product = map(&fixture).expect("valid synthetic RSLC maps");

    assert_eq!(product.provider_id, "asf.nasa.earthdata");
    assert_eq!(product.product_id, "G9900000001-ASF");
    assert_eq!(
        product.display_name,
        "NISAR_L1_PR_RSLC_SYNTHETIC_P05023_0001"
    );
    assert_eq!(product.properties["processing_level"], "RSLC");
    assert_eq!(product.properties["science_level"], "L1");
    assert_eq!(product.properties["quality_status"], "Provisional");
    assert_eq!(product.properties["production_configuration"], "PR");
    assert_eq!(product.properties["band"], "L");
    assert_eq!(product.properties["crid"], "P05023");
    assert_eq!(
        product.properties["product_specification_version"],
        NISAR_P05023_PRODUCT_SPECIFICATION_VERSION
    );
    assert_eq!(
        product.properties["product_specification_profile_registry_version"],
        NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
    );
    assert_eq!(
        product.properties["resolver_policy_id"],
        NISAR_RESOLVER_POLICY_ID_V2
    );
    assert_eq!(product.properties["frame_coverage"], "PARTIAL");
    assert_eq!(product.properties["frequency_a_polarization"], "HH+HV");
    assert_eq!(product.properties["frequency_b_polarization"], "HH");
    assert_eq!(product.properties["range_bandwidth"], "20+5");
    assert_eq!(product.properties["joint_observation"], true);
    assert_eq!(product.properties["track"], 37);
    assert_eq!(product.properties["relative_orbit"], 37);
    assert_eq!(product.properties["preview_available"], true);
    assert_eq!(
        product
            .assets
            .iter()
            .filter(|asset| asset.role == AssetRole::Preview)
            .count(),
        1
    );
    assert!(product.properties.get("s_band_asset").is_none());
    assert!(product.properties.get("bhoonidhi_locator").is_none());

    let primary = product
        .assets
        .iter()
        .filter(|asset| asset.role == AssetRole::Primary)
        .collect::<Vec<_>>();
    assert_eq!(primary.len(), 1);
    assert_eq!(primary[0].asset_key.as_deref(), Some("science-rslc-hdf5"));
    assert_eq!(
        primary[0].file_name.as_deref(),
        Some("NISAR_L1_PR_RSLC_SYNTHETIC_P05023_0001.h5")
    );
    assert_eq!(primary[0].size_bytes, Some(12_000_000_000));
    let access = primary[0]
        .access_descriptor
        .as_ref()
        .expect("primary HDF5 has frozen Earthdata access");
    assert_eq!(access.scheme, AssetAccessScheme::EarthdataHttps);
    assert!(access.requires_credentials);
    assert_eq!(
        access.credential_profile.as_deref(),
        Some("earthdata-default")
    );

    let qa = product
        .assets
        .iter()
        .find(|asset| {
            asset
                .file_name
                .as_deref()
                .is_some_and(|name| name.ends_with("_QA_STATS.h5"))
        })
        .expect("QA HDF5 is retained");
    assert_eq!(qa.role, AssetRole::Auxiliary);
    assert!(qa.access_descriptor.is_none());
    assert_eq!(
        product.properties["asset_integrity"]["science-rslc-hdf5"]["algorithm"],
        "SHA-256"
    );
    assert_eq!(
        product.properties["assets"]
            .as_array()
            .expect("bounded metadata asset graph")
            .len(),
        product.assets.len()
    );
    assert!(
        serde_json::to_vec(&product.properties["assets"])
            .unwrap()
            .len()
            <= 40_960
    );
    product
        .validate()
        .expect("mapped product contract is valid");
}

#[test]
fn captured_public_umm_maps_exact_primary_and_keeps_qa_hdf5_auxiliary() {
    let raw = captured_raw_umm();
    let envelope = CmrGranuleEnvelope::try_from(&raw).expect("captured UMM-G normalizes");
    assert_eq!(envelope.granule["assets"]["raw_resource_count"], 38);
    assert_eq!(
        envelope.granule["assets"]["records"]
            .as_array()
            .expect("normalized public asset records")
            .len(),
        13
    );
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    let product =
        map_nisar_rslc_umm_granule(&envelope, profile).expect("captured public UMM-G maps");

    assert_eq!(product.product_id, "G4260317833-ASF");
    assert_eq!(
        product.display_name,
        "NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_001"
    );
    assert_eq!(product.assets.len(), 13);
    assert_eq!(product.properties["frame_coverage"], "PARTIAL");
    assert_eq!(product.properties["frequency_a_polarization"], "HH");
    assert!(product.properties.get("frequency_b_polarization").is_none());
    assert_eq!(product.properties["range_bandwidth"], "5");
    assert_eq!(product.properties["joint_observation"], false);
    assert_eq!(product.properties["look_direction"], "LEFT");
    assert_eq!(product.properties["track"], 45);
    assert_eq!(product.properties["relative_orbit"], 45);
    assert_eq!(product.properties["crid"], "P05023");
    assert_eq!(product.properties["product_specification_version"], "1.5.0");
    assert_eq!(
        product.properties["product_specification_profile_registry_version"],
        NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
    );
    assert_eq!(product.properties["pge_version"], "R05.02.3");
    let thumbnail = product
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::Thumbnail)
        .expect("captured URL-only thumbnail is retained");
    assert!(thumbnail
        .file_name
        .as_deref()
        .is_some_and(|name| name.ends_with("_LATLON_thumbnail.png")));
    assert!(thumbnail.size_bytes.is_none());
    let previews = product
        .assets
        .iter()
        .filter(|asset| asset.role == AssetRole::Preview)
        .collect::<Vec<_>>();
    assert_eq!(previews.len(), 1);
    assert!(previews[0]
        .file_name
        .as_deref()
        .is_some_and(|name| name.ends_with("_LATLON.png")));
    assert!(product.assets.iter().any(|asset| {
        asset.role == AssetRole::Browse
            && asset
                .file_name
                .as_deref()
                .is_some_and(|name| name.contains("_NATIVE"))
    }));
    let resolved = resolve_preview_candidate(
        product.properties["provider_product_key"].as_str().unwrap(),
        PreviewProductClass::NisarCompleteRslc,
        &product.assets,
    )
    .expect("captured public NISAR graph resolves one classified Preview");
    assert_eq!(
        resolved.source_asset_key,
        previews[0].stable_asset_key.as_deref().unwrap()
    );

    let primary = product
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::Primary)
        .expect("captured science HDF5");
    assert_eq!(primary.size_bytes, Some(314_572_800));
    assert_eq!(
        product.properties["asset_integrity"][primary.asset_key.as_deref().unwrap()]["value"],
        "b1c87f32ef70a9cba31ae2cca8ac8055"
    );
    let qa = product
        .assets
        .iter()
        .find(|asset| {
            asset
                .file_name
                .as_deref()
                .is_some_and(|name| name.ends_with("_QA_STATS.h5"))
        })
        .expect("captured QA HDF5");
    assert_eq!(qa.role, AssetRole::Auxiliary);
    assert_eq!(qa.size_bytes, Some(543_066));
    assert_ne!(primary.stable_asset_key, qa.stable_asset_key);
}

#[test]
fn raw_umm_excludes_known_private_and_credential_endpoints_without_hiding_the_count() {
    let mut raw = captured_raw_umm();
    let producer = raw["umm"]["GranuleUR"]
        .as_str()
        .expect("producer identity")
        .to_owned();
    let related = raw["umm"]["RelatedUrls"]
        .as_array_mut()
        .expect("related URLs");
    related.push(json!({
        "URL": format!("s3://nisar-jpl-private.invalid/restricted/{producer}.h5"),
        "Type": "GET DATA VIA DIRECT ACCESS",
        "Format": "HDF5"
    }));
    related.push(json!({
        "URL": "https://nisar.asf.earthdatacloud.nasa.gov/s3credentials",
        "Type": "VIEW RELATED INFORMATION",
        "Description": "Credential discovery endpoint"
    }));

    let envelope = CmrGranuleEnvelope::try_from(&raw)
        .expect("known non-public endpoints are excluded from the public graph");
    assert_eq!(envelope.granule["assets"]["raw_resource_count"], 40);
    let public_graph = serde_json::to_string(&envelope.granule["assets"]).unwrap();
    assert!(!public_graph.contains("nisar-jpl-private"));
    assert!(!public_graph.contains("s3credentials"));
    assert_eq!(
        envelope.granule["assets"]["records"]
            .as_array()
            .unwrap()
            .len(),
        13
    );
}

#[test]
fn raw_umm_rejects_mixed_native_producer_identity_and_conflicting_url_format() {
    let mut mixed_identity = captured_raw_umm();
    let producer = mixed_identity["umm"]["DataGranule"]["Identifiers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|identifier| identifier["IdentifierType"] == "ProducerGranuleId")
        .expect("producer identifier");
    producer["Identifier"] = json!("NISAR_L1_PR_RSLC_DIFFERENT_GRANULE");
    assert_eq!(
        CmrGranuleEnvelope::try_from(&mixed_identity)
            .unwrap_err()
            .code,
        "ASF_ASSET_CLASSIFICATION_INVALID"
    );

    let mut conflicting_format = captured_raw_umm();
    conflicting_format["umm"]["RelatedUrls"][0]["Format"] = json!("PNG");
    assert_eq!(
        CmrGranuleEnvelope::try_from(&conflicting_format)
            .unwrap_err()
            .code,
        "ASF_ASSET_CLASSIFICATION_INVALID"
    );
}

#[test]
fn archive_format_remediation_matrix_is_explicitly_synthetic_and_redacted() {
    let matrix = load(
        repository_root()
            .join("docs/fixtures/asf-nisar-subset-p1-5/archive-format-remediation-matrix.json"),
    );
    assert_eq!(matrix["synthetic_offline_matrix"], true);
    assert_eq!(matrix["observed_remote_format_value"], "unknown");
    assert_eq!(matrix["not_raw_attempt_2_or_canary_response"], true);
    assert_eq!(matrix["safety"]["public_requests"], 0);
    assert_eq!(matrix["cases"].as_array().expect("matrix cases").len(), 13);
    assert!(!serde_json::to_string(&matrix)
        .unwrap()
        .contains("UNRETAINED_FORMAT_TOKEN"));
}

#[test]
fn related_url_role_matrix_is_synthetic_redacted_and_bounded() {
    let matrix = load(
        repository_root()
            .join("docs/fixtures/asf-nisar-subset-p1-5/related-url-role-remediation-matrix.json"),
    );
    assert_eq!(matrix["synthetic_offline_matrix"], true);
    assert_eq!(matrix["observed_r2_related_url_type"], "unknown");
    assert_eq!(matrix["observed_r2_related_url_subtype"], "unknown");
    assert_eq!(matrix["observed_r2_basename"], "not_retained");
    assert_eq!(matrix["safety"]["public_requests"], 0);
    assert_eq!(matrix["safety"]["credentials_reached"], false);
    assert_eq!(matrix["safety"]["locators_dereferenced"], false);
    assert!(matrix["cases"].as_array().expect("matrix cases").len() >= 20);
    let serialized = serde_json::to_string(&matrix).expect("matrix serialization");
    assert!(!serialized.contains("catalogue-canary-r2"));
    assert!(!serialized.contains("Authorization"));
}

#[test]
fn related_url_role_matrix_executes_against_offline_captured_fixture() {
    let matrix = load(
        repository_root()
            .join("docs/fixtures/asf-nisar-subset-p1-5/related-url-role-remediation-matrix.json"),
    );
    for case in matrix["cases"].as_array().expect("matrix cases") {
        let id = case["id"].as_str().expect("case id");
        let expectation = case["expectation"].as_str().expect("case expectation");
        let mut raw = captured_raw_umm();
        if id == "second_primary_data_route_fails" {
            let primary = raw_primary_archive_mut(&mut raw).clone();
            let mut second = primary;
            second["Name"] = json!("second-primary.h5");
            raw["umm"]["DataGranule"]["ArchiveAndDistributionInformation"]
                .as_array_mut()
                .expect("raw archives")
                .push(second);
        }
        append_related_case(&mut raw, case);
        let result = map(&raw);
        if expectation.starts_with("valid") {
            let product = result.unwrap_or_else(|error| panic!("{id}: {error:?}"));
            let product_json = serde_json::to_string(&product).expect("product serialization");
            if expectation.ends_with(":ignored") || expectation.ends_with(":filtered") {
                let path = case["path"].as_str().expect("matrix path");
                assert!(!product_json.contains(path), "{id}: locator leaked");
            }
        } else {
            let error = result.expect_err(id);
            if let Some(reason) = expectation.strip_prefix("fail:") {
                let debug = format!("{error:?}");
                assert!(
                    debug.contains(reason) || error.code == reason,
                    "{id}: expected {reason}, got {error:?}"
                );
            }
            let serialized = format!("{error:?}");
            assert!(!serialized.contains("DOWNLOAD SOMETHING"));
            assert!(!serialized.contains("GET DATA VIA DIRECT ACCESS EXTRA"));
            assert!(!serialized.contains("UNAPPROVED SUBTYPE"));
        }
    }
}

#[test]
fn related_url_exact_roles_and_binding_boundaries_remain_closed() {
    let mut substring = captured_raw_umm();
    append_related_case(
        &mut substring,
        &json!({
            "relation_type": "GET DATA VIA DIRECT ACCESS EXTRA",
            "path": "lookalike.h5",
            "format": "HDF5"
        }),
    );
    let error = CmrGranuleEnvelope::try_from(&substring).expect_err("substring is not data");
    let debug = format!("{error:?}");
    assert!(debug.contains("related_url_type_unrecognized"));
    assert!(!debug.contains("GET DATA VIA DIRECT ACCESS EXTRA"));

    let mut encoded = captured_raw_umm();
    let primary = format!(
        "{}.h5",
        encoded["umm"]["GranuleUR"].as_str().expect("producer")
    );
    encoded["umm"]["RelatedUrls"]
        .as_array_mut()
        .expect("related URLs")
        .push(json!({
            "URL": format!("https://nisar.asf.earthdatacloud.nasa.gov/synthetic/{}", primary.replace(".h5", "%2Eh5")),
            "Type": "GET DATA",
            "Format": "HDF5"
        }));
    CmrGranuleEnvelope::try_from(&encoded).expect("one safe percent decode binds exactly");

    for encoded_separator in ["%2F", "%5C"] {
        let mut invalid = captured_raw_umm();
        invalid["umm"]["RelatedUrls"]
            .as_array_mut()
            .expect("related URLs")
            .push(json!({
                "URL": format!("https://nisar.asf.earthdatacloud.nasa.gov/synthetic/bad{encoded_separator}name.h5"),
                "Type": "GET DATA",
                "Format": "HDF5"
            }));
        let error = CmrGranuleEnvelope::try_from(&invalid).expect_err("encoded separator rejected");
        assert!(format!("{error:?}").contains("related_url_basename"));
        assert!(!format!("{error:?}").contains(encoded_separator));
    }
}

#[test]
fn unknown_primary_archive_format_can_use_hdf5_mime_evidence() {
    let mut raw = captured_raw_umm();
    let primary = raw_primary_archive_mut(&mut raw);
    primary["Format"] = json!("UNRETAINED_FORMAT_TOKEN");
    primary["MimeType"] = json!("application/x-hdf5");

    let envelope = CmrGranuleEnvelope::try_from(&raw).expect("HDF5 MIME corroborates the primary");
    let product = map(&raw).expect("unknown primary format with HDF5 MIME maps");
    let primary = product
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::Primary)
        .expect("primary asset remains present");
    assert_eq!(primary.file_name.as_deref(), Some("NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_001.h5"));
    assert_eq!(
        envelope.granule["assets"]["records"]
            .as_array()
            .expect("normalized records")
            .iter()
            .find(|record| record["normalized_role"] == "primary")
            .expect("normalized primary")["media_type"],
        "application/x-hdf5"
    );
}

#[test]
fn unknown_primary_without_hdf5_evidence_fails_with_safe_diagnostics() {
    let mut raw = captured_raw_umm();
    let primary = raw_primary_archive_mut(&mut raw);
    primary["Format"] = json!("UNRETAINED_FORMAT_TOKEN");
    primary
        .as_object_mut()
        .expect("primary archive object")
        .remove("MimeType");
    remove_primary_related_format_evidence(&mut raw);

    let error = CmrGranuleEnvelope::try_from(&raw).expect_err("extension alone is insufficient");
    let debug = format!("{error:?}");
    assert_eq!(error.code, "ASF_ASSET_CLASSIFICATION_INVALID");
    assert!(debug.contains("unknown_on_expected_primary"));
    assert!(debug.contains("unknown_on_hdf5_extension"));
    assert!(debug.contains("unrecognized_format_sha256"));
    assert!(!debug.contains("UNRETAINED_FORMAT_TOKEN"));
}

#[test]
fn format_type_is_native_evidence_but_not_primary_hdf5_proof() {
    let mut raw = captured_raw_umm();
    let primary = raw_primary_archive_mut(&mut raw);
    primary["Format"] = json!("UNRETAINED_FORMAT_TOKEN");
    primary["FormatType"] = json!("HDF5");
    primary
        .as_object_mut()
        .expect("primary archive object")
        .remove("MimeType");
    remove_primary_related_format_evidence(&mut raw);

    let error = CmrGranuleEnvelope::try_from(&raw).expect_err("FormatType alone is insufficient");
    assert_eq!(error.code, "ASF_ASSET_CLASSIFICATION_INVALID");
}

#[test]
fn primary_integrity_and_filename_gates_remain_strict() {
    for missing_field in ["SizeInBytes", "Checksum"] {
        let mut raw = captured_raw_umm();
        raw_primary_archive_mut(&mut raw)
            .as_object_mut()
            .expect("primary archive object")
            .remove(missing_field);
        let error = map(&raw).expect_err("incomplete primary integrity is rejected");
        assert_eq!(error.code, "ASF_PRIMARY_ASSET_MISSING", "{missing_field}");
    }

    let mut wrong_algorithm = captured_raw_umm();
    wrong_algorithm["umm"]["DataGranule"]["ArchiveAndDistributionInformation"][0]["Checksum"]
        ["Algorithm"] = json!("SHA-1");
    assert_eq!(
        CmrGranuleEnvelope::try_from(&wrong_algorithm)
            .unwrap_err()
            .code,
        "ASF_ASSET_CLASSIFICATION_INVALID"
    );

    let mut mismatched_filename = captured_raw_umm();
    raw_primary_archive_mut(&mut mismatched_filename)["Name"] = json!("not-the-producer.h5");
    assert_eq!(
        CmrGranuleEnvelope::try_from(&mismatched_filename)
            .unwrap_err()
            .code,
        "ASF_ASSET_CLASSIFICATION_INVALID"
    );
}

#[test]
fn raw_unknown_format_bounds_and_controls_are_rejected_without_leaking_text() {
    let mut control = captured_raw_umm();
    raw_primary_archive_mut(&mut control)["Format"] = json!("bad\nformat");
    let error = CmrGranuleEnvelope::try_from(&control).expect_err("control characters are invalid");
    let debug = format!("{error:?}");
    assert!(!debug.contains("bad\\nformat"));

    let mut oversized = captured_raw_umm();
    raw_primary_archive_mut(&mut oversized)["Format"] = json!("f".repeat(513));
    assert_eq!(
        CmrGranuleEnvelope::try_from(&oversized).unwrap_err().code,
        "ASF_ASSET_GRAPH_LIMIT_EXCEEDED"
    );
}

#[test]
fn unknown_non_hdf5_archives_are_non_executable_other_auxiliaries() {
    for extension in ["json", "txt", "zip", "bin"] {
        let mut raw = captured_raw_umm();
        let expected_name = format!("unknown-auxiliary.{extension}");
        append_raw_archive(
            &mut raw,
            &expected_name,
            Some("UNRETAINED_FORMAT_TOKEN"),
            None,
        );
        let product = map(&raw).expect("unknown non-HDF5 auxiliary maps safely");
        let asset = product
            .assets
            .iter()
            .find(|asset| asset.file_name.as_deref() == Some(expected_name.as_str()))
            .expect("unknown auxiliary retained");
        assert_eq!(asset.role, AssetRole::Auxiliary, "{extension}");
        assert!(asset.access_descriptor.is_none(), "{extension}");
        assert_eq!(
            asset.media_type.as_deref(),
            Some("application/octet-stream")
        );
    }
}

#[test]
fn unknown_hdf5_like_and_second_non_qa_hdf5_archives_fail_closed() {
    let mut unknown_h5 = captured_raw_umm();
    append_raw_archive(
        &mut unknown_h5,
        "unknown-auxiliary.h5",
        Some("UNRETAINED_FORMAT_TOKEN"),
        None,
    );
    assert_eq!(
        CmrGranuleEnvelope::try_from(&unknown_h5).unwrap_err().code,
        "ASF_ASSET_CLASSIFICATION_INVALID"
    );

    let mut second_hdf5 = captured_raw_umm();
    append_raw_archive(&mut second_hdf5, "second-science.h5", Some("HDF5"), None);
    assert_eq!(
        CmrGranuleEnvelope::try_from(&second_hdf5).unwrap_err().code,
        "ASF_ASSET_CLASSIFICATION_INVALID"
    );
}

#[test]
fn archive_mime_conflict_is_not_silently_reinterpreted() {
    let mut raw = captured_raw_umm();
    let primary = raw_primary_archive_mut(&mut raw);
    primary["MimeType"] = json!("image/png");
    let error = CmrGranuleEnvelope::try_from(&raw).expect_err("HDF5 and PNG conflict");
    assert_eq!(error.code, "ASF_ASSET_CLASSIFICATION_INVALID");
}

#[test]
fn raw_umm_asset_graph_overflows_use_the_dedicated_limit_error() {
    let assert_limit = |raw: &Value| {
        assert_eq!(
            CmrGranuleEnvelope::try_from(raw).unwrap_err().code,
            "ASF_ASSET_GRAPH_LIMIT_EXCEEDED"
        );
    };

    let mut archives = captured_raw_umm();
    let archive_values = archives["umm"]["DataGranule"]["ArchiveAndDistributionInformation"]
        .as_array_mut()
        .expect("archives");
    let template = archive_values[0].clone();
    while archive_values.len() <= 32 {
        archive_values.push(template.clone());
    }
    assert_limit(&archives);

    let mut related = captured_raw_umm();
    let related_values = related["umm"]["RelatedUrls"]
        .as_array_mut()
        .expect("related URLs");
    let template = related_values[0].clone();
    while related_values.len() <= 128 {
        related_values.push(template.clone());
    }
    assert_limit(&related);

    let mut file_name = captured_raw_umm();
    file_name["umm"]["DataGranule"]["ArchiveAndDistributionInformation"][0]["Name"] =
        json!("f".repeat(513));
    assert_limit(&file_name);

    let mut locator = captured_raw_umm();
    let basename = locator["umm"]["DataGranule"]["ArchiveAndDistributionInformation"][0]["Name"]
        .as_str()
        .expect("archive name")
        .to_owned();
    let prefix = "https://nisar.asf.earthdatacloud.nasa.gov/";
    let suffix = format!("/{basename}");
    let padding = 4_097 - prefix.len() - suffix.len();
    locator["umm"]["RelatedUrls"][0]["URL"] =
        json!(format!("{prefix}{}{suffix}", "x".repeat(padding)));
    assert_limit(&locator);
}

#[test]
fn frequency_a_only_b_only_and_dual_l_band_products_remain_distinct_and_valid() {
    for (fixture_name, expected_a, expected_b, expected_bandwidth) in [
        ("frequency-a-only-valid.json", Some("HH"), None, "5"),
        ("frequency-b-only-valid.json", None, Some("VV"), "77"),
        (
            "dual-frequency-valid.json",
            Some("HH+HV"),
            Some("HH+HV"),
            "40+5",
        ),
    ] {
        let product = map(&mutated_fixture(fixture_name)).expect(fixture_name);
        assert_eq!(
            product
                .properties
                .get("frequency_a_polarization")
                .and_then(Value::as_str),
            expected_a,
            "{fixture_name}"
        );
        assert_eq!(
            product
                .properties
                .get("frequency_b_polarization")
                .and_then(Value::as_str),
            expected_b,
            "{fixture_name}"
        );
        assert_eq!(
            product.properties["range_bandwidth"], expected_bandwidth,
            "{fixture_name}"
        );
        assert_eq!(product.properties["band"], "L", "{fixture_name}");
        assert!(product.assets.iter().all(|asset| !asset
            .native_roles
            .iter()
            .any(|role| role.contains("S-BAND"))));
    }
}

#[test]
fn captured_public_quad_and_vv_vh_frequency_values_remain_in_provisional_rslc_scope() {
    for value in ["HH+HV+VH+VV", "VV+VH"] {
        let mut raw = captured_raw_umm();
        set_raw_attribute_values(&mut raw, "FREQUENCY_A_POLARIZATION_CONCAT", &[value]);
        let product = map(&raw).unwrap_or_else(|error| panic!("Frequency A {value}: {error:?}"));
        assert_eq!(product.properties["frequency_a_polarization"], value);
    }

    let mut raw = captured_raw_umm();
    set_raw_attribute_values(&mut raw, "FREQUENCIES", &["A", "B"]);
    raw["umm"]["AdditionalAttributes"]
        .as_array_mut()
        .expect("raw additional attributes")
        .push(json!({
            "Name": "FREQUENCY_B_POLARIZATION_CONCAT",
            "Values": ["VV+VH"]
        }));
    let product = map(&raw).expect("captured public Frequency B VV+VH maps");
    assert_eq!(product.properties["frequency_b_polarization"], "VV+VH");
}

#[test]
fn beta_collection_scope_and_identity_mismatches_fail_with_stable_codes() {
    for (fixture_name, expected) in [
        ("beta-rejected.json", "NISAR_QUALITY_STATUS_MISMATCH"),
        (
            "collection-mismatch.json",
            "ASF_COLLECTION_IDENTITY_MISMATCH",
        ),
        ("wrong-product-type.json", "ASF_PRODUCT_SCOPE_MISMATCH"),
        (
            "wrong-production-configuration.json",
            "ASF_PRODUCT_SCOPE_MISMATCH",
        ),
    ] {
        assert_eq!(
            map(&mutated_fixture(fixture_name)).unwrap_err().code,
            expected,
            "{fixture_name}"
        );
    }

    let mut invalid = base_fixture();
    for (path, value, expected) in [
        (
            "/granule/concept_id",
            Value::Null,
            "ASF_PRODUCT_IDENTITY_MISSING",
        ),
        (
            "/granule/quality_status",
            json!("Validated"),
            "NISAR_QUALITY_STATUS_MISMATCH",
        ),
        ("/granule/band", json!("S"), "ASF_PRODUCT_SCOPE_MISMATCH"),
        (
            "/granule/frame_coverage",
            json!("AOI_SUBSET"),
            "ASF_PRODUCT_SCOPE_MISMATCH",
        ),
    ] {
        let old = invalid.pointer(path).cloned().expect("path exists");
        *invalid.pointer_mut(path).expect("path exists") = value;
        assert_eq!(map(&invalid).unwrap_err().code, expected, "{path}");
        *invalid.pointer_mut(path).expect("path exists") = old;
    }
    invalid["granule"]["frequency_a_polarization"] = Value::Null;
    invalid["granule"]["frequency_b_polarization"] = Value::Null;
    assert_eq!(
        map(&invalid).unwrap_err().code,
        "ASF_PRODUCT_SCOPE_MISMATCH"
    );
    invalid["granule"]["frequency_a_polarization"] = json!("HH");
    invalid["granule"]["relative_orbit"] = json!(38);
    assert_eq!(
        map(&invalid).unwrap_err().code,
        "ASF_PRODUCT_SCOPE_MISMATCH"
    );
}

#[test]
fn missing_or_unknown_crid_and_missing_exact_asf_provider_fail_closed() {
    let mut missing_crid = base_fixture();
    missing_crid["granule"]
        .as_object_mut()
        .unwrap()
        .remove("crid");
    assert_eq!(
        map(&missing_crid).unwrap_err().code,
        "ASF_PRODUCT_SCOPE_MISMATCH"
    );

    let mut unknown_crid = base_fixture();
    unknown_crid["granule"]["crid"] = json!("P99999");
    assert_eq!(
        map(&unknown_crid).unwrap_err().code,
        "ASF_PRODUCT_SCOPE_MISMATCH"
    );

    let mut missing_provider = base_fixture();
    missing_provider["collection"]
        .as_object_mut()
        .unwrap()
        .remove("cmr_provider_id");
    assert_eq!(
        map(&missing_provider).unwrap_err().code,
        "ASF_COLLECTION_IDENTITY_MISMATCH"
    );
}

#[test]
fn browse_absence_is_honest_and_does_not_invalidate_the_complete_product() {
    let product = map(&mutated_fixture("browse-absent-valid.json"))
        .expect("missing Browse does not invalidate the primary HDF5");
    assert_eq!(product.properties["preview_available"], false);
    assert!(product.assets.iter().all(|asset| {
        !matches!(
            asset.role,
            AssetRole::Preview | AssetRole::Browse | AssetRole::Thumbnail
        )
    }));
    assert_eq!(
        product
            .assets
            .iter()
            .filter(|asset| asset.role == AssetRole::Primary)
            .count(),
        1
    );

    let mut thumbnail_only = base_fixture();
    asset_records_mut(&mut thumbnail_only).retain(|asset| asset["normalized_role"] != "preview");
    let product = map(&thumbnail_only).expect("thumbnail-only product remains valid");
    assert_eq!(product.properties["preview_available"], false);
    assert!(product
        .assets
        .iter()
        .any(|asset| asset.role == AssetRole::Thumbnail));
}
