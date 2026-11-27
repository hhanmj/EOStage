use std::fs;
use std::path::{Path, PathBuf};

use insar_core::{
    CatalogCollectionDescriptor, CatalogProvider, CatalogSearchQuery, CatalogSearchResponse,
    ContractError, ProviderResult,
};
use serde_json::{Map, Value};

pub struct FixtureCatalogProvider {
    fixture_root: PathBuf,
}

impl FixtureCatalogProvider {
    pub fn from_fixture_root(fixture_root: PathBuf) -> Self {
        Self { fixture_root }
    }

    fn error(code: &str, user_message: &str) -> ContractError {
        ContractError {
            code: code.to_string(),
            user_message: user_message.to_string(),
            debug_context: Map::new(),
        }
    }

    fn read_json(&self, name: &str) -> ProviderResult<Value> {
        let path = self.fixture_root.join(name);
        let text = fs::read_to_string(&path)
            .map_err(|_| Self::error("CATALOG_FIXTURE_UNAVAILABLE", "固定目录 Fixture 不可读取"))?;
        serde_json::from_str(&text)
            .map_err(|_| Self::error("CATALOG_FIXTURE_MALFORMED", "固定目录 Fixture 格式无效"))
    }

    fn validates_collection_identity(query: &CatalogSearchQuery) -> bool {
        query.logical_dataset == "optical.sentinel-2.l2a"
            && query.provider_id == "fixture.catalog"
            && query.native_collection_id == "FIXTURE-S2-L2A"
    }

    fn page_name(cursor: Option<&str>) -> Result<&'static str, ContractError> {
        match cursor {
            None => Ok("items_page_1.json"),
            Some("fixture-page-2") => Ok("items_page_2.json"),
            Some("fixture-empty") => Ok("empty.json"),
            Some("fixture-malformed") => Ok("malformed_item.json"),
            Some("fixture-duplicate-product") => Ok("duplicate_product.json"),
            Some("fixture-cross-provider") => Ok("cross_provider_product.json"),
            Some("fixture-duplicate-asset-key") => Ok("duplicate_asset_key.json"),
            Some("fixture-invalid-native-role") => Ok("invalid_native_role.json"),
            Some("fixture-deduplicated") => Ok("deduplicated_page.json"),
            Some(_) => Err(Self::error(
                "CATALOG_CURSOR_UNTRUSTED",
                "目录续页标识未受信任",
            )),
        }
    }

    fn parse_page(&self, path: &Path) -> ProviderResult<CatalogSearchResponse> {
        let text = fs::read_to_string(path)
            .map_err(|_| Self::error("CATALOG_FIXTURE_UNAVAILABLE", "固定目录 Fixture 不可读取"))?;
        serde_json::from_str::<CatalogSearchResponse>(&text)
            .map_err(|_| Self::error("CATALOG_ITEM_MALFORMED", "固定目录记录无法映射"))
    }
}

impl CatalogProvider for FixtureCatalogProvider {
    fn collections(&self) -> ProviderResult<Vec<CatalogCollectionDescriptor>> {
        let value = self.read_json("collection.json")?;
        let collection = serde_json::from_value(value)
            .map_err(|_| Self::error("CATALOG_FIXTURE_MALFORMED", "固定目录集合定义无效"))?;
        Ok(vec![collection])
    }

    fn search_catalog(&self, query: &CatalogSearchQuery) -> ProviderResult<CatalogSearchResponse> {
        query
            .validate()
            .map_err(|_| Self::error("CATALOG_QUERY_INVALID", "固定目录检索范围无效"))?;
        if !Self::validates_collection_identity(query) {
            return Err(Self::error(
                "CATALOG_ROUTE_MISMATCH",
                "目录请求与固定集合不匹配",
            ));
        }
        let name = Self::page_name(query.cursor.as_deref())?;
        let response = self.parse_page(&self.fixture_root.join(name))?;
        response
            .validate()
            .map_err(|_| Self::error("CATALOG_ITEM_MALFORMED", "固定目录记录无法映射"))?;
        Ok(response)
    }
}
