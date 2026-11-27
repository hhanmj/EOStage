use super::*;
use insar_provider::catalog::{
    MPC_LANDSAT_ASSET_POLICY_ID, MPC_LANDSAT_COLLECTION_ID, MPC_LANDSAT_PROVIDER_ID,
    MPC_LANDSAT_SCENE_PROFILE_ID,
};

const NATIVE_ID: &str = "LC09_L2SP_123045_20260101_02_T1";

fn native_asset(index: usize) -> SnapshotItem {
    let mut item = executable_planetary_computer_asset(index);
    let profile = CollectionProfileRegistry
        .get(MPC_LANDSAT_SCENE_PROFILE_ID)
        .unwrap();
    let key =
        provider_product_key(profile.provider_id, profile.native_collection_id, NATIVE_ID).unwrap();
    item.provider_id = profile.provider_id.to_owned();
    for (name, value) in [
        ("native_collection_id", profile.native_collection_id),
        ("collection_profile_id", profile.profile_id),
        ("identity_rule_id", profile.identity_rule_id),
        ("filename_rule_id", profile.filename_rule_id),
        ("provider_product_id", NATIVE_ID),
        ("provider_product_key", &key),
        ("source_item_id", &key),
    ] {
        item.metadata.insert(name.to_owned(), json!(value));
    }
    item.metadata["properties"]["entity_id"] = json!(NATIVE_ID);
    item.metadata["properties"]["dataset_alias"] = json!(profile.native_collection_id);
    let reference = &mut item.asset_access.as_mut().unwrap().reference;
    reference.provider_id = profile.provider_id.to_owned();
    reference.collection_id = profile.native_collection_id.to_owned();
    reference.provider_product_id = NATIVE_ID.to_owned();
    reference.provider_product_key = key;
    reference.stable_asset_key =
        stable_asset_key(&reference.provider_product_key, &reference.asset_key).unwrap();
    item.item_id = reference.stable_asset_key.clone();
    item.execution_policy.as_mut().unwrap().policy_id = MPC_LANDSAT_ASSET_POLICY_ID.to_owned();
    item.execution_policy
        .as_mut()
        .unwrap()
        .collection_profile_id = Some(profile.profile_id.to_owned());
    item
}

#[test]
fn native_and_historical_asset_policies_remain_separately_executable() {
    for index in 0..22 {
        let legacy = executable_planetary_computer_asset(index);
        let native = native_asset(index);
        for item in [&legacy, &native] {
            assert!(matches!(
                classify_frozen_execution(
                    item,
                    &AsfCollectionProfileRegistry,
                    &AssetExecutionPolicyRegistry
                ),
                FrozenExecution::ResolverAuthoritative(_)
            ));
            assert!(
                !item
                    .asset_access
                    .as_ref()
                    .unwrap()
                    .descriptor
                    .requires_credentials
            );
        }
        assert_eq!(native.display_name, legacy.display_name);
        assert_eq!(
            native.asset_access.as_ref().unwrap().expected_size_bytes,
            legacy.asset_access.as_ref().unwrap().expected_size_bytes
        );
        assert_eq!(
            native.asset_access.as_ref().unwrap().descriptor,
            legacy.asset_access.as_ref().unwrap().descriptor
        );
        assert_ne!(native.item_id, legacy.item_id);
        assert_eq!(
            native.execution_policy.as_ref().unwrap().policy_id,
            MPC_LANDSAT_ASSET_POLICY_ID
        );
        assert_eq!(
            legacy.execution_policy.as_ref().unwrap().policy_id,
            PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID
        );
    }
}

#[test]
fn native_asset_rejects_mixed_source_policy_and_coherently_forged_keys() {
    let mutations: &[fn(&mut SnapshotItem)] = &[
        |x| x.provider_id = "usgs.eros.m2m".to_owned(),
        |x| x.metadata["native_collection_id"] = json!("landsat_ot_c2_l2"),
        |x| x.metadata["identity_rule_id"] = json!("usgs.eros.m2m.landsat.c2.l2.identity.v1"),
        |x| {
            x.execution_policy.as_mut().unwrap().policy_id =
                PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID.to_owned()
        },
        |x| {
            x.asset_access.as_mut().unwrap().reference.collection_id = "landsat_ot_c2_l2".to_owned()
        },
        |x| {
            x.asset_access
                .as_mut()
                .unwrap()
                .reference
                .provider_product_id = "LC91230452026001LGN00".to_owned()
        },
        |x| {
            x.asset_access
                .as_mut()
                .unwrap()
                .descriptor
                .stable_href
                .as_mut()
                .unwrap()
                .push_str("?sig=fixture-only")
        },
        |x| {
            let key = provider_product_key(
                MPC_LANDSAT_PROVIDER_ID,
                MPC_LANDSAT_COLLECTION_ID,
                "another-product",
            )
            .unwrap();
            x.metadata["provider_product_key"] = json!(key);
            x.metadata["source_item_id"] = json!(key);
            let reference = &mut x.asset_access.as_mut().unwrap().reference;
            reference.provider_product_key = key;
            reference.stable_asset_key =
                stable_asset_key(&reference.provider_product_key, &reference.asset_key).unwrap();
            x.item_id = reference.stable_asset_key.clone();
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut item = native_asset(1);
        mutate(&mut item);
        assert!(
            matches!(
                classify_frozen_execution(
                    &item,
                    &AsfCollectionProfileRegistry,
                    &AssetExecutionPolicyRegistry
                ),
                FrozenExecution::Invalid(_)
            ),
            "mutation {index}"
        );
    }
}
