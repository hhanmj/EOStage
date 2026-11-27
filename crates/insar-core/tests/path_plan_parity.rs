use std::fs;
use std::path::{Path, PathBuf};

use insar_core::{
    plan_desktop_orbit_target, plan_desktop_target, plan_sar_product_target, DesktopProductFamily,
    DesktopProductLayout, OrbitProductType, SarProduct,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    valid: Vec<ValidCase>,
    invalid_file_names: Vec<String>,
}

#[derive(Deserialize)]
struct ValidCase {
    selected_root: String,
    file_name: String,
    family: DesktopProductFamily,
    layout: DesktopProductLayout,
    expected: String,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_pure_logic/path_plan.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn wire_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn desktop_targets_match_python_fixture_without_legacy_sar_data() {
    for case in fixture().valid {
        let target = plan_desktop_target(
            Path::new(&case.selected_root),
            &case.file_name,
            case.family,
            case.layout,
        )
        .unwrap();

        assert_eq!(wire_path(&target), case.expected);
        assert!(!wire_path(&target).contains("SAR_Data"));
    }
}

#[test]
fn desktop_targets_reject_unsafe_file_names() {
    for file_name in fixture().invalid_file_names {
        for family in [DesktopProductFamily::Sentinel1, DesktopProductFamily::Orbit] {
            let error = plan_desktop_target(
                Path::new("D:/selected"),
                &file_name,
                family,
                DesktopProductLayout::Flat,
            )
            .unwrap_err();

            assert_eq!(error.code, "V3_INVALID_TARGET_PATH");
            assert_eq!(error.user_message, "下载目标路径无效");
            if !file_name.is_empty() {
                assert!(!error.to_string().contains(&file_name));
            }
        }
    }
}

#[test]
fn generic_sar_targets_use_native_filename_and_optional_dataset_level_layout() {
    let product: SarProduct = serde_json::from_value(serde_json::json!({
        "product_key": "ASF::ALOS PALSAR::ALPSRP123",
        "product_id": "ALPSRP123",
        "scene_id": "ALPSRP123",
        "provider": "ASF",
        "dataset": "ALOS PALSAR",
        "platform": "ALOS",
        "instrument": "PALSAR",
        "processing_level": "L1.1",
        "product_family": "SLC",
        "download_url": "https://example.org/native-product.tar.gz?token=redacted"
    }))
    .unwrap();

    assert_eq!(
        plan_sar_product_target(Path::new("E:/downloads"), &product, false).unwrap(),
        Path::new("E:/downloads/native-product.tar.gz")
    );
    assert_eq!(
        plan_sar_product_target(Path::new("E:/downloads"), &product, true).unwrap(),
        Path::new("E:/downloads/ALOS_PALSAR/L1_1/native-product.tar.gz")
    );
}

#[test]
fn orbit_subdirectory_layout_isolates_resorb_from_poeorb() {
    let poeorb = "S1A_OPER_AUX_POEORB_OPOD_20260401T070707_V20260311T225942_20260313T005942.EOF";
    let resorb = "S1A_OPER_AUX_RESORB_OPOD_20260312T140000_V20260312T095942_20260312T131712.EOF";
    let root = Path::new("E:/downloads");

    assert_eq!(
        wire_path(
            &plan_desktop_target(
                root,
                poeorb,
                DesktopProductFamily::Orbit,
                DesktopProductLayout::ProductSubdirectory,
            )
            .unwrap()
        ),
        format!("E:/downloads/Sentinel_Orbit/AUX_POEORB/{poeorb}")
    );
    assert_eq!(
        wire_path(
            &plan_desktop_target(
                root,
                resorb,
                DesktopProductFamily::Orbit,
                DesktopProductLayout::ProductSubdirectory,
            )
            .unwrap()
        ),
        format!("E:/downloads/Sentinel_Orbit/AUX_RESORB/{resorb}")
    );
    assert_eq!(
        wire_path(
            &plan_desktop_orbit_target(
                root,
                resorb,
                OrbitProductType::RESORB,
                DesktopProductLayout::ProductSubdirectory,
            )
            .unwrap()
        ),
        format!("E:/downloads/Sentinel_Orbit/AUX_RESORB/{resorb}")
    );
}
