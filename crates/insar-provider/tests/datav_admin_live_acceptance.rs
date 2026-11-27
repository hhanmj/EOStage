use insar_provider::boundary::{ChinaBoundaryProvider, DataVBoundaryProvider};

#[test]
#[ignore = "explicit live DataV diagnostic"]
fn hubei_province_boundary_is_accepted_by_the_production_provider() {
    let provider = DataVBoundaryProvider::production().expect("DataV provider initializes");
    let results = provider
        .search("湖北省", "全部", "全部", "", 8)
        .expect("湖北省 boundary remains bindable");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "湖北省");
}

#[test]
#[ignore = "explicit live DataV exact-adcode diagnostic"]
fn qinghai_province_resolves_by_exact_adcode_without_hierarchy_discovery() {
    let provider = DataVBoundaryProvider::production().expect("DataV provider initializes");
    let result = provider
        .resolve_exact_adcode("630000")
        .expect("Qinghai exact DataV boundary remains bindable");

    assert_eq!(result.provider_object_id, "630000");
    assert_eq!(result.name, "青海省");
    assert!(matches!(
        result.geometry_kind.as_str(),
        "polygon" | "multipolygon"
    ));
}

#[test]
#[ignore = "explicit live DataV diagnostic"]
fn national_boundary_is_accepted_as_full_authoritative_geometry() {
    let provider = DataVBoundaryProvider::production().expect("DataV provider initializes");
    let results = provider
        .search("全国", "全部", "全部", "", 1)
        .expect("national boundary remains bindable");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "中国");
    assert_eq!(results[0].admin_level.as_deref(), Some("country"));
    assert_eq!(results[0].provider_object_id, "100000");
}

#[test]
#[ignore = "explicit live DataV municipality-options diagnostic"]
fn beijing_options_expose_direct_district_identities() {
    let provider = DataVBoundaryProvider::production().expect("DataV provider initializes");
    let options = provider
        .options("北京市", "全部")
        .expect("Beijing district options remain discoverable");

    assert!(options.districts.iter().all(|option| option.name == "全部"));
    assert!(options.cities.iter().any(|option| {
        option.name == "海淀区" && option.provider_object_id.as_deref() == Some("110108")
    }));
    assert!(!options.cities.iter().any(|option| {
        option.name == "北京市"
            || matches!(
                option.provider_object_id.as_deref(),
                Some("110100" | "110200")
            )
    }));
}

#[test]
#[ignore = "explicit live DataV Hong Kong/Macao two-level diagnostic"]
fn hong_kong_and_macao_options_expose_direct_second_level_identities() {
    let provider = DataVBoundaryProvider::production().expect("DataV provider initializes");
    for (province, expected_name, expected_adcode) in [
        ("香港特别行政区", "湾仔区", "810002"),
        ("澳门特别行政区", "花地玛堂区", "820001"),
    ] {
        let options = provider
            .options(province, expected_name)
            .expect("special-region direct second-level options remain discoverable");
        assert!(options.districts.iter().all(|option| option.name == "全部"));
        assert!(options.cities.iter().any(|option| {
            option.name == expected_name
                && option.provider_object_id.as_deref() == Some(expected_adcode)
        }));
        assert!(!options.cities.iter().any(|option| option.name == province));

        let result = provider
            .resolve_exact_adcode(expected_adcode)
            .expect("special-region child exact geometry remains bindable");
        assert_eq!(result.provider_object_id, expected_adcode);
        assert_eq!(result.name, expected_name);
        assert!(matches!(
            result.geometry_kind.as_str(),
            "polygon" | "multipolygon"
        ));
    }
}

#[test]
#[ignore = "explicit live DataV Taiwan boundary acceptance"]
fn taiwan_whole_boundary_is_bindable_but_current_transport_has_no_second_level_batch() {
    let provider = DataVBoundaryProvider::production().expect("DataV provider initializes");
    let whole = provider
        .search("台湾省", "全部", "全部", "", 1)
        .expect("Taiwan whole boundary remains bindable");
    assert_eq!(whole.len(), 1);
    assert_eq!(whole[0].provider_object_id, "710000");
    assert!(matches!(
        whole[0].geometry_kind.as_str(),
        "polygon" | "multipolygon"
    ));

    let child_error = provider
        .options("台湾省", "全部")
        .expect_err("current DataV areas_v3 exposes childrenNum=0 and no 710000_full transport");
    assert_eq!(child_error.code, "BOUNDARY_PROVIDER_HTTP_ERROR");
}

#[test]
#[ignore = "explicit live DataV administrative-completeness diagnostic"]
fn every_bundled_province_level_selector_has_bindable_geometry() {
    const PROVINCES: &[&str] = &[
        "北京市",
        "天津市",
        "河北省",
        "山西省",
        "内蒙古自治区",
        "辽宁省",
        "吉林省",
        "黑龙江省",
        "上海市",
        "江苏省",
        "浙江省",
        "安徽省",
        "福建省",
        "江西省",
        "山东省",
        "河南省",
        "湖北省",
        "湖南省",
        "广东省",
        "广西壮族自治区",
        "海南省",
        "重庆市",
        "四川省",
        "贵州省",
        "云南省",
        "西藏自治区",
        "陕西省",
        "甘肃省",
        "青海省",
        "宁夏回族自治区",
        "新疆维吾尔自治区",
        "台湾省",
        "香港特别行政区",
        "澳门特别行政区",
    ];
    let provider = DataVBoundaryProvider::production().expect("DataV provider initializes");
    let mut failures = Vec::new();
    for province in PROVINCES {
        match provider.search(province, "全部", "全部", "", 8) {
            Ok(results) if results.len() == 1 => {}
            Ok(results) => failures.push(format!("{province}: {} results", results.len())),
            Err(error) => failures.push(format!("{province}: {error:?}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
