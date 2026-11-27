use insar_application_service::{ApplicationService, ApplicationServiceConfig};
use insar_core::{
    AccountId, SourceAvailability, SourceFamily, SourceNetworkDefault, SourceSearchMode,
};

#[test]
fn all_reviewed_source_families_keep_registry_identity_and_route_requirements() {
    let sources = ApplicationService::new(ApplicationServiceConfig::liveness_only())
        .list_sources()
        .expect("embedded source projection");
    assert!(sources
        .windows(2)
        .all(|pair| pair[0].resource_id < pair[1].resource_id));
    let l = sources
        .iter()
        .find(|s| s.resource_id == "science.nisar.l.rslc.provisional.v1")
        .unwrap();
    let s = sources
        .iter()
        .find(|s| s.resource_id == "science.nisar.s.ssar.rslc.v1")
        .unwrap();
    assert_eq!(l.group_id, "nisar");
    assert_eq!(s.group_id, l.group_id);
    assert_eq!(l.network_default, SourceNetworkDefault::Direct);
    assert_eq!(s.network_default, SourceNetworkDefault::SystemProxy);
    assert_eq!(s.accounts, vec![AccountId::Bhoonidhi]);
    assert_eq!(l.search_mode, SourceSearchMode::Sar);
    assert_eq!(l.result_cap, 250);
    let optical = sources
        .iter()
        .find(|s| s.resource_id == "science.sentinel-2.l2a.v1")
        .unwrap();
    assert_eq!(optical.family, SourceFamily::Optical);
    assert_eq!(optical.search_mode, SourceSearchMode::CatalogueIntent);
    // Owner decision 2026-11-13: optical defaults to 250 like SAR.
    assert_eq!(optical.result_cap, 250);
    assert_eq!(
        optical.filter_schema_id.as_deref(),
        Some("search.optical.sentinel-2.l2a.v1")
    );
    assert_eq!(optical.filter_schema_revision, Some(1));
    assert!(optical.accounts.contains(&AccountId::Cdse));
    let dem = sources
        .iter()
        .find(|s| s.resource_id == "dem.cop30.v1")
        .unwrap();
    assert_eq!(dem.family, SourceFamily::Dem);
    assert_eq!(dem.search_mode, SourceSearchMode::Auxiliary);
    assert!(dem.accounts.contains(&AccountId::Opentopography));
    assert!(sources.iter().any(|s| s.family == SourceFamily::Orbit));
    assert!(sources
        .iter()
        .any(|s| s.family == SourceFamily::Atmospheric && s.accounts.contains(&AccountId::Gacos)));
    let unavailable = sources
        .iter()
        .find(|s| s.resource_id == "science.envisat.reserved.v1")
        .unwrap();
    assert_eq!(unavailable.availability, SourceAvailability::Unavailable);
    assert!(unavailable.availability_reason_zh.is_some());

    // ASF removed the legacy missions from its searchable interfaces
    // (Owner evidence 2026-10-31): the three retired resources stay listed
    // for identity but are unavailable with the typed ASF removal reason,
    // and the dropdown disables them like ENVISAT.
    for resource_id in [
        "science.ers-1-2.sar.v1",
        "science.jers-1.sar.v1",
        "science.radarsat-1.sar.v1",
    ] {
        let retired = sources
            .iter()
            .find(|s| s.resource_id == resource_id)
            .unwrap();
        assert_eq!(retired.availability, SourceAvailability::Unavailable);
        assert_eq!(
            retired.availability_reason_zh.as_deref(),
            Some("ASF 已从检索接口下线该数据集")
        );
    }
}
