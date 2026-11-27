#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsfItemModel {
    OneFilePerProduct,
    MultiAssetProduct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsfDeliveryModel {
    SinglePrimaryArchive,
    MultiAssetProduct,
    Unreviewed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsfCollectionProfile {
    pub profile_id: &'static str,
    pub rule_id: &'static str,
    pub dataset: &'static str,
    pub item_id_fields: &'static [&'static str],
    pub asset_id_fields: &'static [&'static str],
    pub display_alias_fields: &'static [&'static str],
    pub critical_identity_fields: &'static [&'static str],
    pub item_model: AsfItemModel,
    pub delivery_model: AsfDeliveryModel,
    pub approved_resolver_ids: &'static [&'static str],
    pub allow_scene_name_identity: bool,
}

const DISPLAY_FIELDS: &[&str] = &[
    "sceneName",
    "granuleName",
    "fileName",
    "productID",
    "fileID",
];
const CRITICAL_IDENTITY_FIELDS: &[&str] = &[
    "processingLevel",
    "processingType",
    "level",
    "pgeVersion",
    "processingVersion",
    "version",
];

const SENTINEL_1: AsfCollectionProfile = AsfCollectionProfile {
    profile_id: "asf.sentinel-1.standard.v1",
    rule_id: "asf.sentinel-1.standard.v1",
    dataset: "SENTINEL-1",
    item_id_fields: &[
        "productID",
        "granuleUR",
        "fileID",
        "sceneName",
        "granuleName",
    ],
    asset_id_fields: &["fileID", "fileName"],
    display_alias_fields: DISPLAY_FIELDS,
    critical_identity_fields: CRITICAL_IDENTITY_FIELDS,
    item_model: AsfItemModel::OneFilePerProduct,
    delivery_model: AsfDeliveryModel::SinglePrimaryArchive,
    approved_resolver_ids: &["earthdata_https"],
    allow_scene_name_identity: true,
};

const SENTINEL_1_BURST: AsfCollectionProfile = AsfCollectionProfile {
    profile_id: "asf.sentinel-1.burst.v1",
    rule_id: "asf.sentinel-1.burst.v1",
    dataset: "SLC-BURST",
    item_id_fields: &["productID", "granuleUR", "groupID"],
    asset_id_fields: &["fileID", "fileName"],
    display_alias_fields: DISPLAY_FIELDS,
    critical_identity_fields: CRITICAL_IDENTITY_FIELDS,
    item_model: AsfItemModel::MultiAssetProduct,
    delivery_model: AsfDeliveryModel::MultiAssetProduct,
    approved_resolver_ids: &["earthdata_https"],
    allow_scene_name_identity: false,
};

const ALOS_PALSAR: AsfCollectionProfile = AsfCollectionProfile {
    profile_id: "asf.alos-palsar.v2",
    rule_id: "asf.alos-palsar.v2",
    dataset: "ALOS PALSAR",
    item_id_fields: &["productID", "granuleUR", "fileID"],
    asset_id_fields: &["fileID", "fileName"],
    display_alias_fields: DISPLAY_FIELDS,
    critical_identity_fields: CRITICAL_IDENTITY_FIELDS,
    item_model: AsfItemModel::OneFilePerProduct,
    delivery_model: AsfDeliveryModel::Unreviewed,
    approved_resolver_ids: &[],
    allow_scene_name_identity: false,
};

const ERS: AsfCollectionProfile = AsfCollectionProfile {
    profile_id: "asf.ers.v1",
    rule_id: "asf.ers.v1",
    dataset: "ERS",
    item_id_fields: &[
        "productID",
        "granuleUR",
        "fileID",
        "granuleName",
        "sceneName",
    ],
    asset_id_fields: &["fileID", "fileName"],
    display_alias_fields: DISPLAY_FIELDS,
    critical_identity_fields: CRITICAL_IDENTITY_FIELDS,
    item_model: AsfItemModel::OneFilePerProduct,
    delivery_model: AsfDeliveryModel::Unreviewed,
    approved_resolver_ids: &[],
    allow_scene_name_identity: true,
};

const JERS_1: AsfCollectionProfile = AsfCollectionProfile {
    profile_id: "asf.jers-1.v1",
    rule_id: "asf.jers-1.v1",
    dataset: "JERS-1",
    item_id_fields: &[
        "productID",
        "granuleUR",
        "fileID",
        "granuleName",
        "sceneName",
    ],
    asset_id_fields: &["fileID", "fileName"],
    display_alias_fields: DISPLAY_FIELDS,
    critical_identity_fields: CRITICAL_IDENTITY_FIELDS,
    item_model: AsfItemModel::OneFilePerProduct,
    delivery_model: AsfDeliveryModel::Unreviewed,
    approved_resolver_ids: &[],
    allow_scene_name_identity: true,
};

const RADARSAT_1: AsfCollectionProfile = AsfCollectionProfile {
    profile_id: "asf.radarsat-1.v1",
    rule_id: "asf.radarsat-1.v1",
    dataset: "RADARSAT-1",
    item_id_fields: &[
        "productID",
        "granuleUR",
        "fileID",
        "granuleName",
        "sceneName",
    ],
    asset_id_fields: &["fileID", "fileName"],
    display_alias_fields: DISPLAY_FIELDS,
    critical_identity_fields: CRITICAL_IDENTITY_FIELDS,
    item_model: AsfItemModel::OneFilePerProduct,
    delivery_model: AsfDeliveryModel::Unreviewed,
    approved_resolver_ids: &[],
    allow_scene_name_identity: true,
};

const SEASAT: AsfCollectionProfile = AsfCollectionProfile {
    profile_id: "asf.seasat.v1",
    rule_id: "asf.seasat.v1",
    dataset: "SEASAT",
    item_id_fields: &[
        "productID",
        "granuleUR",
        "fileID",
        "granuleName",
        "sceneName",
    ],
    asset_id_fields: &["fileID", "fileName"],
    display_alias_fields: DISPLAY_FIELDS,
    critical_identity_fields: CRITICAL_IDENTITY_FIELDS,
    item_model: AsfItemModel::OneFilePerProduct,
    delivery_model: AsfDeliveryModel::Unreviewed,
    approved_resolver_ids: &[],
    allow_scene_name_identity: true,
};

pub fn profile_for_dataset(dataset: &str) -> Option<&'static AsfCollectionProfile> {
    // Resolves through the static profile table rather than the registry's
    // implemented-selector binding: the ASF-removed legacy missions
    // (Owner evidence 2026-10-31) are retired in the registry, but their
    // identity profiles stay addressable so the provider code path can be
    // restored if ASF re-exposes the collections.
    let trimmed = dataset.trim();
    [
        &SENTINEL_1,
        &SENTINEL_1_BURST,
        &ALOS_PALSAR,
        &ERS,
        &JERS_1,
        &RADARSAT_1,
        &SEASAT,
    ]
    .into_iter()
    .find(|profile| profile.dataset.eq_ignore_ascii_case(trimmed))
}

pub fn profile_for_id(profile_id: &str) -> Option<&'static AsfCollectionProfile> {
    match profile_id {
        "asf.sentinel-1.standard.v1" => Some(&SENTINEL_1),
        "asf.sentinel-1.burst.v1" => Some(&SENTINEL_1_BURST),
        "asf.alos-palsar.v2" => Some(&ALOS_PALSAR),
        "asf.ers.v1" => Some(&ERS),
        "asf.jers-1.v1" => Some(&JERS_1),
        "asf.radarsat-1.v1" => Some(&RADARSAT_1),
        "asf.seasat.v1" => Some(&SEASAT),
        _ => None,
    }
}
