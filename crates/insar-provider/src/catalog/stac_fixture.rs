//! Program-owned, non-serializable admission for an offline STAC experiment.
//! No Registry mutation, environment flag, request field, or persisted grant.

use super::*;
use crate::catalog::{CollectionProfileRegistry, DatasetRoutePolicyRegistry, RouteUse};

#[derive(Clone, Debug)]
pub struct StacOfflineFixtureAdmission {
    route: StacRouteProfile,
    binding: StacCollectionBinding,
}

impl StacOfflineFixtureAdmission {
    /// Only hidden fixture runtime constructors accept this value. It cannot
    /// shadow a registered dataset/profile or carry production admission.
    pub fn new_for_test(
        route: StacRouteProfile,
        binding: StacCollectionBinding,
    ) -> ProviderResult<Self> {
        binding.validate(&route)?;
        if binding.production_enabled
            || !binding.provider_id.starts_with("fixture.")
            || !binding.logical_dataset.contains(".fixture.")
            || DatasetRoutePolicyRegistry
                .for_logical_dataset(&binding.logical_dataset)
                .is_some()
            || CollectionProfileRegistry
                .get(&binding.collection_profile_id)
                .is_some()
        {
            return Err(binding_error("offline_fixture_namespace"));
        }
        Ok(Self { route, binding })
    }

    pub fn targets(&self, query: &CatalogSearchQuery) -> bool {
        query.logical_dataset == self.binding.logical_dataset
    }

    pub fn binding(&self) -> &StacCollectionBinding {
        &self.binding
    }

    pub fn validate_query(
        &self,
        query: &CatalogSearchQuery,
        route_use: RouteUse,
    ) -> ProviderResult<()> {
        if route_use != RouteUse::OfflineFixture || self.binding.production_enabled {
            return Err(query_error("offline_fixture_only"));
        }
        let query = &insar_core::clamp_catalogue_search_query(query);
        if query.cursor.is_some() {
            return Err(query_error("offline_fixture_query_bounds"));
        }
        // Reuse the adapter's scope, date, exact AOI and queryable validation.
        build_stac_search_request(&self.route, &self.binding, query)?;
        Ok(())
    }

    pub fn validate_response(
        &self,
        query: &CatalogSearchQuery,
        response: &CatalogSearchResponse,
        route_use: RouteUse,
    ) -> ProviderResult<()> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        self.validate_query(query, route_use)?;
        response
            .validate()
            .map_err(|_| response_error("fixture_response"))?;
        if response.collection != self.binding.collection_descriptor() {
            return Err(identity_error("fixture_collection"));
        }
        let stats = &response.stats;
        let count = response.products.len() as u64;
        let complete = query.result_limit.map_or(stats.remote_exhausted, |limit| {
            count <= limit && (count == limit || stats.remote_exhausted)
        });
        if !complete
            || !stats.working_set_satisfied
            || stats.truncated
            || response.next_cursor.is_some()
            || !response.item_errors.is_empty()
            || stats.requested_limit != query.result_limit
            || stats.returned_count != count
            || stats.request_count != 1
            || stats.transport_part_count != 1
            || stats.fetched_record_count > self.route.page_cap
            || stats.fetched_record_count != stats.mapped_record_count
            || stats.profile_filtered_record_count != 0
            || stats.exact_duplicate_record_count != 0
            || stats.mapped_record_count != stats.unique_provider_product_count
            || stats.exact_matched_count != stats.unique_provider_product_count
            || count > stats.exact_matched_count
            || stats
                .matched_count
                .is_some_and(|total| total < stats.unique_provider_product_count)
        {
            return Err(response_error("fixture_completeness"));
        }
        let mut ids = BTreeSet::new();
        for product in &response.products {
            self.validate_product(product, route_use)?;
            if !ids.insert(&product.product_id) {
                return Err(identity_error("fixture_duplicate_product"));
            }
        }
        Ok(())
    }

    /// #22 B slice 3(b) review fix 7: the stats-only counterpart of
    /// [`Self::validate_response`] for a streamed search. The products were
    /// validated one-by-one at mapping time (including duplicate identity
    /// across pages by the transport); this re-runs the same stats checks
    /// against the observed candidate count instead of a products array.
    pub fn validate_response_stats(
        &self,
        query: &CatalogSearchQuery,
        collection: &CatalogCollectionDescriptor,
        stats: &CatalogSearchStats,
        candidate_count: u64,
        route_use: RouteUse,
    ) -> ProviderResult<()> {
        let query = &insar_core::clamp_catalogue_search_query(query);
        self.validate_query(query, route_use)?;
        stats
            .validate()
            .map_err(|_| response_error("fixture_response"))?;
        if *collection != self.binding.collection_descriptor() {
            return Err(identity_error("fixture_collection"));
        }
        // The same completeness and stats checks as validate_response, with
        // the streamed candidate count taking the products length's place.
        let complete = query.result_limit.map_or(stats.remote_exhausted, |limit| {
            candidate_count <= limit && (candidate_count == limit || stats.remote_exhausted)
        });
        if !complete
            || !stats.working_set_satisfied
            || stats.truncated
            || stats.requested_limit != query.result_limit
            || stats.returned_count != candidate_count
            || stats.request_count != 1
            || stats.transport_part_count != 1
            || stats.fetched_record_count > self.route.page_cap
            || stats.fetched_record_count != stats.mapped_record_count
            || stats.profile_filtered_record_count != 0
            || stats.exact_duplicate_record_count != 0
            || stats.mapped_record_count != stats.unique_provider_product_count
            || stats.exact_matched_count != stats.unique_provider_product_count
            || candidate_count > stats.exact_matched_count
            || stats
                .matched_count
                .is_some_and(|total| total < stats.unique_provider_product_count)
        {
            return Err(response_error("fixture_completeness"));
        }
        Ok(())
    }

    /// Recheck normalized products at the application boundary rather than
    /// trusting a Provider port merely because it was injected by a test.
    pub fn validate_product(
        &self,
        product: &ProviderProduct,
        route_use: RouteUse,
    ) -> ProviderResult<()> {
        if route_use != RouteUse::OfflineFixture {
            return Err(query_error("offline_fixture_only"));
        }
        product
            .validate()
            .map_err(|_| response_error("fixture_product"))?;
        if product.provider_id != self.binding.provider_id
            || product.source_kind != self.binding.source_kind
            || product.product_kind != self.binding.product_kind
            || product.footprint_bbox.is_some()
            || product
                .footprint_geojson
                .as_ref()
                .and_then(insar_core::validated_authoritative_polygonal_geojson)
                .is_none()
        {
            return Err(identity_error("fixture_product_scope"));
        }
        let properties = &product.properties;
        let item_id = properties
            .get("stac_item_id")
            .and_then(Value::as_str)
            .and_then(valid_item_text)
            .ok_or_else(|| identity_error("fixture_item_id"))?;
        let stac_properties = properties
            .get("stac_properties")
            .and_then(Value::as_object)
            .ok_or_else(|| response_error("fixture_stac_properties"))?;
        let expected_id = identity_value(
            stac_properties,
            self.binding.product_id_property.as_deref(),
            item_id,
        )?;
        let expected_name = identity_value(
            stac_properties,
            self.binding.display_name_property.as_deref(),
            item_id,
        )?;
        let key = provider_product_key(
            &self.binding.provider_id,
            &self.binding.native_collection_id,
            &expected_id,
        )
        .map_err(|_| identity_error("fixture_product_key"))?;
        let datetime = stac_properties
            .get("datetime")
            .and_then(Value::as_str)
            .and_then(|value| normalized_utc(value).ok())
            .ok_or_else(|| response_error("fixture_datetime"))?;
        if product.product_id != expected_id
            || product.display_name != expected_name
            || product.acquisition_datetime.as_deref() != Some(datetime.as_str())
            || properties
                .get("provider_product_key")
                .and_then(Value::as_str)
                != Some(key.as_str())
            || properties
                .get("provider_product_id")
                .and_then(Value::as_str)
                != Some(expected_id.as_str())
            || properties
                .get("collection_profile_id")
                .and_then(Value::as_str)
                != Some(self.binding.collection_profile_id.as_str())
            || properties.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "provider_product_key"
                        | "provider_product_id"
                        | "collection_profile_id"
                        | "stac_item_id"
                        | "stac_properties"
                        | "title"
                )
            })
        {
            return Err(identity_error("fixture_identity_facts"));
        }
        if product.payload.len() != 1
            || product
                .payload
                .get("stac")
                .and_then(|v| v.get("version"))
                .and_then(Value::as_str)
                .is_none_or(|version| !self.route.accepts_version(version))
        {
            return Err(response_error("fixture_stac_payload"));
        }
        // Reuse asset mapping validation, including unsigned URL/host rules.
        // Equality rejects forged stable keys, roles, filenames and access grants.
        let mut raw_assets = Map::new();
        for asset in &product.assets {
            let asset_key = asset
                .asset_key
                .as_ref()
                .ok_or_else(|| asset_error("fixture_asset_key"))?;
            let mut raw = Map::from_iter([
                ("href".to_owned(), json!(asset.url)),
                ("roles".to_owned(), json!(asset.native_roles)),
            ]);
            if let Some(media) = &asset.media_type {
                raw.insert("type".to_owned(), json!(media));
            }
            if let Some(size) = asset.size_bytes {
                raw.insert("file:size".to_owned(), json!(size));
            }
            if raw_assets
                .insert(asset_key.clone(), Value::Object(raw))
                .is_some()
            {
                return Err(asset_error("fixture_duplicate_asset"));
            }
        }
        let mapped = map_assets(&self.binding, &key, item_id, &raw_assets)?;
        if mapped != product.assets {
            return Err(asset_error("fixture_asset_authority"));
        }
        Ok(())
    }
}
