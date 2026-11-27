use std::fs;
use std::path::PathBuf;

use insar_provider::cdse::archive_filename::evaluate_archive_filename;
use serde::Deserialize;

#[derive(Deserialize)]
struct HeaderFixture {
    odata_name: String,
    headers: serde_json::Map<String, serde_json::Value>,
    outcome: String,
}

fn fixture(name: &str) -> HeaderFixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/catalog/cdse/sentinel2_l2a")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("fixture reads")).expect("fixture shape")
}

#[test]
fn header_evidence_is_required_and_only_the_profile_match_is_accepted() {
    let valid = fixture("odata_download_headers_filename_match.json");
    let header = valid.headers["content_disposition"].as_str();
    assert_eq!(valid.outcome, "accept");
    assert_eq!(
        evaluate_archive_filename(&valid.odata_name, header)
            .unwrap()
            .basename,
        "S2_FIXTURE_ONE.SAFE.zip"
    );

    for name in [
        "odata_download_headers_filename_missing.json",
        "odata_download_headers_filename_conflict.json",
        "odata_unsafe_content_disposition.json",
    ] {
        let fixture = fixture(name);
        assert!(
            evaluate_archive_filename(
                &fixture.odata_name,
                fixture
                    .headers
                    .get("content_disposition")
                    .and_then(serde_json::Value::as_str)
            )
            .is_err(),
            "{name}"
        );
    }

    let unsafe_product: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("tests/fixtures/catalog/cdse/sentinel2_l2a/odata_unsafe_product_name.json"),
        )
        .expect("unsafe product fixture reads"),
    )
    .expect("unsafe product fixture shape");
    assert!(evaluate_archive_filename(
        unsafe_product["value"][0]["Name"].as_str().unwrap(),
        Some("attachment; filename=\"S2_UNSAFE.SAFE.zip\""),
    )
    .is_err());
    assert!(evaluate_archive_filename(
        "S2_MISSING_SAFE_SUFFIX",
        Some("attachment; filename=\"S2_MISSING_SAFE_SUFFIX.zip\""),
    )
    .is_err());
    assert!(evaluate_archive_filename(
        "S2_DUPLICATE.SAFE.SAFE",
        Some("attachment; filename=\"S2_DUPLICATE.SAFE.SAFE.zip\""),
    )
    .is_err());

    for (odata_name, header) in [
        (
            " S2_SPACE.SAFE",
            "attachment; filename=\" S2_SPACE.SAFE.zip\"",
        ),
        (
            "S2_TRAILING.SAFE",
            "attachment; filename=\"S2_TRAILING.SAFE.zip \"",
        ),
        ("CON.SAFE", "attachment; filename=\"CON.SAFE.zip\""),
        (
            "S2_VARIANT.SAFE",
            "inline; filename=\"S2_VARIANT.SAFE.zip\"",
        ),
        (
            "S2_VARIANT.SAFE",
            "attachment; filename*=UTF-8''S2_VARIANT.SAFE.zip",
        ),
        (
            "S2_VARIANT.SAFE",
            "attachment; filename=\"S2_VARIANT.SAFE.zip\"; filename=\"S2_OTHER.SAFE.zip\"",
        ),
        (
            "S2_VARIANT.SAFE",
            "attachment; filename=S2_VARIANT.SAFE.zip",
        ),
    ] {
        assert!(
            evaluate_archive_filename(odata_name, Some(header)).is_err(),
            "{header}"
        );
    }

    assert_eq!(
        evaluate_archive_filename(
            "S2_普通.SAFE",
            Some("attachment; filename=\"S2_普通.SAFE.zip\""),
        )
        .unwrap()
        .basename,
        "S2_普通.SAFE.zip"
    );

    for invalid in ['<', '>', '"', '|', '?', '*'] {
        let name = format!("S2{invalid}BAD.SAFE");
        let header = format!("attachment; filename=\"{name}.zip\"");
        assert!(
            evaluate_archive_filename(&name, Some(&header)).is_err(),
            "{invalid}"
        );
    }
    assert!(evaluate_archive_filename(
        "S2_SAFE.SAFE",
        Some("attachment; filename=\"S2_SAFE.SAFE.zip\"; ignored=\"line\nfeed\"")
    )
    .is_err());
    assert!(evaluate_archive_filename(
        "NUL.SAFE",
        Some("attachment; filename=\"NUL.txt.SAFE.zip\"")
    )
    .is_err());
    let overlong = format!("{}.SAFE", "a".repeat(251));
    let header = format!("attachment; filename=\"{overlong}.zip\"");
    assert!(evaluate_archive_filename(&overlong, Some(&header)).is_err());
}
