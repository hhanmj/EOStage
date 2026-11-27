fn escape_component(value: &str) -> String {
    value.trim().replace('%', "%25").replace("::", "%3A%3A")
}

pub fn provider_product_key(
    provider_id: &str,
    collection_id: &str,
    provider_product_id: &str,
) -> Result<String, String> {
    let provider_id = provider_id.trim();
    let collection_id = collection_id.trim();
    let provider_product_id = provider_product_id.trim();
    if provider_id.is_empty() || collection_id.is_empty() || provider_product_id.is_empty() {
        return Err("provider_id, collection_id, and provider_product_id are required".to_string());
    }
    Ok(format!(
        "PROVIDER::{}::COLLECTION::{}::ITEM::{}",
        escape_component(provider_id),
        escape_component(collection_id),
        escape_component(provider_product_id)
    ))
}

pub fn stable_asset_key(provider_product_key: &str, asset_key: &str) -> Result<String, String> {
    let provider_product_key = provider_product_key.trim();
    let asset_key = asset_key.trim();
    if provider_product_key.is_empty() || asset_key.is_empty() {
        return Err("provider_product_key and asset_key are required".to_string());
    }
    Ok(format!(
        "{provider_product_key}::ASSET::{}",
        escape_component(asset_key)
    ))
}
