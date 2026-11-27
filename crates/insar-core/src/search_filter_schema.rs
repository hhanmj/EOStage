//! R2 search filter schemas (Issue #3).
//!
//! A reviewed, EOStage-owned description of the discovery predicates a
//! science dataset exposes and how each maps onto the existing route-neutral
//! `CatalogSearchQuery.filters` vocabulary. The Resource Registry stays the
//! product authority: a schema is only reachable through a resource whose
//! `search_schema_id` names it. A schema is not a provider binding, not runtime
//! readiness and not a delivery decision; it never carries endpoints, native
//! collection ids, native field names, credentials or executable expressions.
//!
//! STAC queryables are an alignment reference only (Issue #7 audit, P2): their
//! coverage differs per endpoint and non-STAC routes have none.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::{ResourceDescriptor, ResourceRegistry, StaticCapabilityLifecycle};

pub const EMBEDDED_SEARCH_FILTER_SCHEMAS_JSON: &str =
    include_str!("../../../contracts/v3/search-filter-schemas.v1.json");
pub const SEARCH_FILTER_SCHEMA_CATALOG_ID: &str = "eostage.search-filter-schemas.v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchFilterSchemaCatalog {
    pub schema_version: u32,
    pub catalog_id: String,
    pub review_version: String,
    pub schemas: Vec<SearchFilterSchema>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchFilterSchema {
    pub schema_id: String,
    pub schema_revision: u32,
    pub resource_id: String,
    pub dataset_label: String,
    pub description: String,
    /// Where the Workbench lists this product (Issue #3 R2 B5). Absent means
    /// the schema is not offered by any Workbench panel yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workbench: Option<SearchFilterWorkbench>,
    pub filters: Vec<SearchFilter>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchFilterWorkbenchPanel {
    Optical,
}

/// Presentation placement only: it grants no route, credential or delivery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchFilterWorkbench {
    pub panel: SearchFilterWorkbenchPanel,
    /// Stable UI key, unique across the catalog (lowercase, digits, `_`).
    pub dataset_key: String,
    /// Order within the panel, unique per panel.
    pub order: u32,
    /// Must equal the Registry resource's `catalogue_logical_dataset`; the
    /// UI uses it to recognise frozen candidates of this product.
    pub logical_dataset: String,
    pub search_action_label: String,
    pub result_hint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchFilterGroup {
    /// Product facets rendered together (platform, instrument, level, type).
    Facet,
    Time,
    Quality,
}

/// What an empty multi-select means. `Omit` = no constraint, the key is not
/// sent. `RestoreAll` = an empty selection is never sent; the UI restores all
/// options and a builder rejects an empty value defensively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmptySelection {
    Omit,
    RestoreAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberOperator {
    Lte,
    Gte,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchFilterOption {
    pub value: String,
    pub label: String,
    pub description: String,
    /// When present, must name a platform the Registry declares for the
    /// resource. Registry platforms may be broader (history); the schema lists
    /// only the reviewed searchable subset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_platform: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case", deny_unknown_fields)]
pub enum SearchFilter {
    MultiEnum {
        filter_id: String,
        label: String,
        group: SearchFilterGroup,
        query_key: String,
        options: Vec<SearchFilterOption>,
        default: Vec<String>,
        empty_selection: EmptySelection,
        invalid_message: String,
    },
    /// A single reviewed value shown as a fact. With a `query_key` it is also
    /// sent as a constant predicate on every query.
    Fixed {
        filter_id: String,
        label: String,
        group: SearchFilterGroup,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query_key: Option<String>,
        options: Vec<SearchFilterOption>,
    },
    /// UTC interval; both ends or neither (neither = all time).
    DateRange {
        filter_id: String,
        label: String,
        group: SearchFilterGroup,
        query_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        help: Option<String>,
    },
    /// Non-negative decimal input; empty = no constraint.
    Number {
        filter_id: String,
        label: String,
        group: SearchFilterGroup,
        query_key: String,
        operator: NumberOperator,
        minimum: f64,
        maximum: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default)]
        default: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        invalid_message: String,
    },
}

impl SearchFilter {
    pub fn filter_id(&self) -> &str {
        match self {
            Self::MultiEnum { filter_id, .. }
            | Self::Fixed { filter_id, .. }
            | Self::DateRange { filter_id, .. }
            | Self::Number { filter_id, .. } => filter_id,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::MultiEnum { label, .. }
            | Self::Fixed { label, .. }
            | Self::DateRange { label, .. }
            | Self::Number { label, .. } => label,
        }
    }

    pub fn group(&self) -> SearchFilterGroup {
        match self {
            Self::MultiEnum { group, .. }
            | Self::Fixed { group, .. }
            | Self::DateRange { group, .. }
            | Self::Number { group, .. } => *group,
        }
    }

    /// The route-neutral `CatalogSearchQuery.filters` key this filter emits,
    /// if any. Display-only fixed facts emit nothing.
    pub fn query_key(&self) -> Option<&str> {
        match self {
            Self::MultiEnum { query_key, .. }
            | Self::DateRange { query_key, .. }
            | Self::Number { query_key, .. } => Some(query_key),
            Self::Fixed { query_key, .. } => query_key.as_deref(),
        }
    }

    pub fn options(&self) -> &[SearchFilterOption] {
        match self {
            Self::MultiEnum { options, .. } | Self::Fixed { options, .. } => options,
            Self::DateRange { .. } | Self::Number { .. } => &[],
        }
    }
}

impl SearchFilterSchema {
    /// Every filter key this schema can emit, in schema order.
    pub fn query_keys(&self) -> Vec<&str> {
        self.filters
            .iter()
            .filter_map(SearchFilter::query_key)
            .collect()
    }

    /// Check route-neutral query values against this reviewed schema (Issue #3
    /// R2 review M1), mirroring the UI builder: every key is declared; constant
    /// facts are present and exact; multi-selects are non-empty unique option
    /// values (an empty "omit" selection is never sent); date ranges carry both
    /// RFC 3339 ends with start before end; numbers are finite and in range.
    pub fn validate_query_filters(
        &self,
        filters: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        use serde_json::Value;
        let id = &self.schema_id;
        let declared = self.query_keys();
        for key in filters.keys() {
            if !declared.contains(&key.as_str()) {
                return Err(format!(
                    "filter {key} is not declared by search schema {id}"
                ));
            }
        }
        for filter in &self.filters {
            match filter {
                SearchFilter::Fixed {
                    query_key: Some(key),
                    options,
                    ..
                } => {
                    let expected = options.first().map(|option| option.value.as_str());
                    if filters.get(key).and_then(Value::as_str) != expected {
                        return Err(format!(
                            "constant filter {key} must be {} for search schema {id}",
                            expected.unwrap_or_default()
                        ));
                    }
                }
                SearchFilter::Fixed {
                    query_key: None, ..
                } => {}
                SearchFilter::MultiEnum {
                    query_key,
                    options,
                    empty_selection,
                    ..
                } => {
                    let Some(value) = filters.get(query_key) else {
                        // "Restore all" selections are always sent (the UI
                        // never sends them empty or absent).
                        if *empty_selection == EmptySelection::RestoreAll {
                            return Err(format!(
                                "filter {query_key} is required by search schema {id}"
                            ));
                        }
                        continue;
                    };
                    let values = value
                        .as_array()
                        .filter(|values| !values.is_empty())
                        .ok_or_else(|| {
                            format!("filter {query_key} must be a non-empty list for search schema {id}")
                        })?;
                    let mut seen = BTreeSet::new();
                    for item in values {
                        let item = item
                            .as_str()
                            .filter(|item| options.iter().any(|option| option.value == *item));
                        match item {
                            Some(item) if seen.insert(item) => {}
                            _ => {
                                return Err(format!(
                                    "filter {query_key} has a value outside search schema {id}"
                                ))
                            }
                        }
                    }
                }
                SearchFilter::DateRange { query_key, .. } => {
                    let Some(value) = filters.get(query_key) else {
                        continue;
                    };
                    let range = value
                        .as_object()
                        .filter(|range| range.len() == 2)
                        .ok_or_else(|| {
                            format!("filter {query_key} must carry start and end for search schema {id}")
                        })?;
                    // UTC only, as the UI sends ("…Z").
                    let instant = |end: &str| {
                        range
                            .get(end)
                            .and_then(Value::as_str)
                            .filter(|text| text.ends_with('Z'))
                            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
                    };
                    match (instant("start"), instant("end")) {
                        (Some(start), Some(end)) if start < end => {}
                        _ => {
                            return Err(format!(
                                "filter {query_key} needs RFC 3339 start before end for search schema {id}"
                            ))
                        }
                    }
                }
                SearchFilter::Number {
                    query_key,
                    minimum,
                    maximum,
                    ..
                } => {
                    let Some(value) = filters.get(query_key) else {
                        continue;
                    };
                    match value.as_f64() {
                        Some(number)
                            if number.is_finite() && (*minimum..=*maximum).contains(&number) => {}
                        _ => {
                            return Err(format!(
                                "filter {query_key} must be a number from {minimum} to {maximum} for search schema {id}"
                            ))
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl SearchFilterSchemaCatalog {
    /// Parse and structurally validate the embedded catalog.
    pub fn embedded() -> Result<Self, String> {
        Self::from_json(EMBEDDED_SEARCH_FILTER_SCHEMAS_JSON)
    }

    /// The embedded catalog parsed once per process; structural validation
    /// and the Registry cross-check both run before it is published.
    pub fn embedded_ref() -> Result<&'static Self, String> {
        static CATALOG: OnceLock<Result<SearchFilterSchemaCatalog, String>> = OnceLock::new();
        match CATALOG.get_or_init(|| {
            let catalog = Self::embedded()?;
            catalog.validate_against_registry(crate::ResourceRegistry::embedded_ref()?)?;
            Ok(catalog)
        }) {
            Ok(catalog) => Ok(catalog),
            Err(error) => Err(error.clone()),
        }
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let catalog: Self = serde_json::from_str(json)
            .map_err(|error| format!("search filter schema catalog does not parse: {error}"))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn schema(&self, schema_id: &str) -> Option<&SearchFilterSchema> {
        self.schemas
            .iter()
            .find(|schema| schema.schema_id == schema_id)
    }

    pub fn for_resource(&self, resource_id: &str) -> Option<&SearchFilterSchema> {
        self.schemas
            .iter()
            .find(|schema| schema.resource_id == resource_id)
    }

    /// Structural rules that do not need the Registry.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported search filter schema_version {}",
                self.schema_version
            ));
        }
        if self.catalog_id != SEARCH_FILTER_SCHEMA_CATALOG_ID {
            return Err(format!("unexpected catalog_id {}", self.catalog_id));
        }
        if !is_review_date(&self.review_version) {
            return Err(format!(
                "review_version must be YYYY-MM-DD: {}",
                self.review_version
            ));
        }
        if self.schemas.is_empty() {
            return Err("search filter schema catalog is empty".to_owned());
        }
        let mut schema_ids = BTreeSet::new();
        let mut resource_ids = BTreeSet::new();
        let mut dataset_keys = BTreeSet::new();
        let mut panel_orders = BTreeSet::new();
        for schema in &self.schemas {
            validate_schema(schema)?;
            if !schema_ids.insert(schema.schema_id.as_str()) {
                return Err(format!("duplicate schema_id {}", schema.schema_id));
            }
            if !resource_ids.insert(schema.resource_id.as_str()) {
                return Err(format!(
                    "resource {} has more than one search filter schema",
                    schema.resource_id
                ));
            }
            if let Some(workbench) = &schema.workbench {
                validate_workbench(&schema.schema_id, workbench)?;
                if !dataset_keys.insert(workbench.dataset_key.as_str()) {
                    return Err(format!(
                        "duplicate workbench dataset_key {}",
                        workbench.dataset_key
                    ));
                }
                if !panel_orders.insert((workbench.panel as u8, workbench.order)) {
                    return Err(format!(
                        "schema {} reuses workbench order {}",
                        schema.schema_id, workbench.order
                    ));
                }
            }
        }
        Ok(())
    }

    /// Cross-checks with the Registry: every schema belongs to an active
    /// science resource that names it, and every `registry_platform` is a
    /// platform the Registry declares for that resource.
    pub fn validate_against_registry(&self, registry: &ResourceRegistry) -> Result<(), String> {
        for schema in &self.schemas {
            let resource = registry.resource(&schema.resource_id).ok_or_else(|| {
                format!(
                    "schema {} names unknown resource {}",
                    schema.schema_id, schema.resource_id
                )
            })?;
            let ResourceDescriptor::ScienceDataset { common, science } = resource else {
                return Err(format!(
                    "schema {} names non-science resource {}",
                    schema.schema_id, schema.resource_id
                ));
            };
            if matches!(
                common.lifecycle,
                StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
            ) {
                return Err(format!(
                    "schema {} names inactive resource {}",
                    schema.schema_id, schema.resource_id
                ));
            }
            if science.search_schema_id.as_deref() != Some(schema.schema_id.as_str()) {
                return Err(format!(
                    "resource {} does not declare search_schema_id {}",
                    schema.resource_id, schema.schema_id
                ));
            }
            let Some(logical_dataset) = science.catalogue_logical_dataset.as_deref() else {
                return Err(format!(
                    "resource {} has a search filter schema but no catalogue_logical_dataset",
                    schema.resource_id
                ));
            };
            if let Some(workbench) = &schema.workbench {
                if workbench.logical_dataset != logical_dataset {
                    return Err(format!(
                        "schema {} workbench logical_dataset {} differs from the Registry ({logical_dataset})",
                        schema.schema_id, workbench.logical_dataset
                    ));
                }
            }
            for filter in &schema.filters {
                for option in filter.options() {
                    if let Some(platform) = option.registry_platform.as_deref() {
                        if !science.platforms.iter().any(|known| known == platform) {
                            return Err(format!(
                                "schema {} filter {} option {} names platform {} that resource {} does not declare",
                                schema.schema_id,
                                filter.filter_id(),
                                option.value,
                                platform,
                                schema.resource_id
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn validate_workbench(schema_id: &str, workbench: &SearchFilterWorkbench) -> Result<(), String> {
    let key = &workbench.dataset_key;
    if key.is_empty()
        || key.len() > 48
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(format!(
            "schema {schema_id} has an invalid workbench dataset_key {key}"
        ));
    }
    for (field, value) in [
        ("logical_dataset", &workbench.logical_dataset),
        ("search_action_label", &workbench.search_action_label),
        ("result_hint", &workbench.result_hint),
    ] {
        if value.trim().is_empty() {
            return Err(format!("schema {schema_id} has empty workbench {field}"));
        }
    }
    Ok(())
}

fn validate_schema(schema: &SearchFilterSchema) -> Result<(), String> {
    let id = &schema.schema_id;
    if !is_schema_id(id) {
        return Err(format!("invalid schema_id {id}"));
    }
    if schema.schema_revision == 0 {
        return Err(format!("schema {id} revision must be at least 1"));
    }
    for (field, value) in [
        ("resource_id", &schema.resource_id),
        ("dataset_label", &schema.dataset_label),
        ("description", &schema.description),
    ] {
        if value.trim().is_empty() {
            return Err(format!("schema {id} has empty {field}"));
        }
    }
    if schema.filters.is_empty() {
        return Err(format!("schema {id} has no filters"));
    }
    let mut filter_ids = BTreeSet::new();
    let mut query_keys: BTreeMap<&str, &str> = BTreeMap::new();
    for filter in &schema.filters {
        let filter_id = filter.filter_id();
        if !is_key(filter_id) {
            return Err(format!("schema {id} has invalid filter_id {filter_id}"));
        }
        if !filter_ids.insert(filter_id) {
            return Err(format!("schema {id} repeats filter_id {filter_id}"));
        }
        if filter.label().trim().is_empty() {
            return Err(format!("schema {id} filter {filter_id} has an empty label"));
        }
        if let Some(query_key) = filter.query_key() {
            if !is_key(query_key) {
                return Err(format!(
                    "schema {id} filter {filter_id} has invalid query_key {query_key}"
                ));
            }
            // One binding per key: ranges are one filter, not two bindings.
            if let Some(previous) = query_keys.insert(query_key, filter_id) {
                return Err(format!(
                    "schema {id} binds query_key {query_key} twice ({previous}, {filter_id})"
                ));
            }
        }
        validate_options(id, filter_id, filter.options())?;
        // The Workbench renders facets as menus or fixed facts and conditions
        // as date/number fields; a control in the other group has no renderer.
        let is_facet_control = matches!(
            filter,
            SearchFilter::MultiEnum { .. } | SearchFilter::Fixed { .. }
        );
        if is_facet_control != (filter.group() == SearchFilterGroup::Facet) {
            return Err(format!(
                "schema {id} filter {filter_id}: multi-selects and fixed facts belong to the facet group, dates and numbers outside it"
            ));
        }
        match filter {
            SearchFilter::MultiEnum {
                options,
                default,
                invalid_message,
                ..
            } => {
                if options.is_empty() {
                    return Err(format!("schema {id} filter {filter_id} has no options"));
                }
                let known: BTreeSet<&str> =
                    options.iter().map(|option| option.value.as_str()).collect();
                let mut seen = BTreeSet::new();
                for value in default {
                    if !known.contains(value.as_str()) || !seen.insert(value.as_str()) {
                        return Err(format!(
                            "schema {id} filter {filter_id} default {value} is unknown or repeated"
                        ));
                    }
                }
                if invalid_message.trim().is_empty() {
                    return Err(format!(
                        "schema {id} filter {filter_id} needs an invalid_message"
                    ));
                }
            }
            SearchFilter::Fixed { options, .. } => {
                if options.len() != 1 {
                    return Err(format!(
                        "schema {id} fixed filter {filter_id} must have exactly one option"
                    ));
                }
            }
            SearchFilter::DateRange { .. } => {}
            SearchFilter::Number {
                minimum,
                maximum,
                default,
                invalid_message,
                ..
            } => {
                if !minimum.is_finite()
                    || !maximum.is_finite()
                    || *minimum < 0.0
                    || minimum > maximum
                {
                    return Err(format!(
                        "schema {id} number filter {filter_id} needs finite 0 <= minimum <= maximum"
                    ));
                }
                if let Some(value) = default {
                    if !value.is_finite() || value < minimum || value > maximum {
                        return Err(format!(
                            "schema {id} number filter {filter_id} default is out of range"
                        ));
                    }
                }
                if invalid_message.trim().is_empty() {
                    return Err(format!(
                        "schema {id} filter {filter_id} needs an invalid_message"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_options(
    schema_id: &str,
    filter_id: &str,
    options: &[SearchFilterOption],
) -> Result<(), String> {
    let mut values = BTreeSet::new();
    for option in options {
        if !is_token(&option.value) || !values.insert(option.value.as_str()) {
            return Err(format!(
                "schema {schema_id} filter {filter_id} option {} is invalid or repeated",
                option.value
            ));
        }
        if option.label.trim().is_empty() || option.description.trim().is_empty() {
            return Err(format!(
                "schema {schema_id} filter {filter_id} option {} needs a label and description",
                option.value
            ));
        }
        if matches!(option.registry_platform.as_deref(), Some(platform) if platform.trim().is_empty())
        {
            return Err(format!(
                "schema {schema_id} filter {filter_id} option {} has an empty registry_platform",
                option.value
            ));
        }
    }
    Ok(())
}

fn is_key(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn is_token(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
}

fn is_schema_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("search.") else {
        return false;
    };
    let Some((body, version)) = rest.rsplit_once(".v") else {
        return false;
    };
    !body.is_empty()
        && body
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '-'))
        && !version.is_empty()
        && version.chars().all(|ch| ch.is_ascii_digit())
}

fn is_review_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}
