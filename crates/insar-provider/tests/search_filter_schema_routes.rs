//! Issue #3 R2: a search filter schema may only emit filter keys that every
//! Registry-declared route of its resource accepts. Ties Registry -> route
//! policy -> schema so a new filter or route cannot drift silently.
use insar_core::{ResourceDescriptor, ResourceRegistry, SearchFilterSchemaCatalog};
use insar_provider::catalog::{DatasetRoutePolicyRegistry, RouteUse};

#[test]
fn every_schema_query_key_is_accepted_by_every_declared_route() {
    let registry = ResourceRegistry::embedded().unwrap();
    let catalog = SearchFilterSchemaCatalog::embedded().unwrap();
    catalog.validate_against_registry(&registry).unwrap();
    let policies = DatasetRoutePolicyRegistry;
    let mut checked_routes = 0;
    for schema in &catalog.schemas {
        let Some(ResourceDescriptor::ScienceDataset { common, science }) =
            registry.resource(&schema.resource_id)
        else {
            panic!("schema resource must be a science dataset");
        };
        let logical_dataset = science
            .catalogue_logical_dataset
            .as_deref()
            .expect("schema-driven resources declare a catalogue logical dataset");
        for route in &common.provider_routes {
            let collection = route
                .native_collection_id
                .as_deref()
                .unwrap_or_else(|| panic!("route {} has no native collection", route.route_id));
            let policy = policies
                .resolve(
                    logical_dataset,
                    &route.provider_id,
                    collection,
                    RouteUse::OfflineFixture,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "Registry route {} for {} has no route policy: {error:?}",
                        route.route_id, schema.resource_id
                    )
                });
            for key in schema.query_keys() {
                assert!(
                    policy.supported_filter_keys.contains(&key),
                    "route {} ({}) does not accept filter key {key} from {}",
                    route.route_id,
                    policy.route_id,
                    schema.schema_id
                );
            }
            checked_routes += 1;
        }
    }
    // Sentinel-2 L2A: 4 Registry routes (CDSE archive/search, MPC, Earth Search); Sentinel-2 L1C: 2; Landsat C2 L2: 5; Sentinel-3: 2; HLS L30 and S30: 1 each.
    assert_eq!(checked_routes, 15);
}
