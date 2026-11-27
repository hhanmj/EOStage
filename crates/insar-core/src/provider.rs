use crate::{
    CatalogCollectionDescriptor, CatalogSearchQuery, CatalogSearchResponse, ContractError,
    ProviderMetadata, ProviderPlan, ProviderProduct, ProviderQuery,
};

pub type ProviderResult<T> = Result<T, ContractError>;

pub trait DataProvider: Send + Sync {
    fn metadata(&self) -> ProviderMetadata;

    fn search(&self, query: &ProviderQuery) -> ProviderResult<Vec<ProviderProduct>>;

    fn plan(
        &self,
        query: &ProviderQuery,
        products: Option<&[ProviderProduct]>,
    ) -> ProviderResult<ProviderPlan>;
}

pub trait CatalogProvider: Send + Sync {
    fn collections(&self) -> ProviderResult<Vec<CatalogCollectionDescriptor>>;

    fn search_catalog(&self, query: &CatalogSearchQuery) -> ProviderResult<CatalogSearchResponse>;
}
