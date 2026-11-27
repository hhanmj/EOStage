fn escape_component(value: &str) -> String {
    value.trim().replace('%', "%25").replace("::", "%3A%3A")
}

pub fn canonical_product_key(
    provider: &str,
    dataset: &str,
    product_id: &str,
) -> Result<String, String> {
    let provider = provider.trim().to_ascii_uppercase();
    let dataset = dataset.trim().to_ascii_uppercase();
    let product_id = product_id.trim();
    if provider.is_empty() || dataset.is_empty() || product_id.is_empty() {
        return Err("provider, dataset, and product_id are required".to_string());
    }
    Ok(format!(
        "{provider}::{dataset}::{}",
        escape_component(product_id)
    ))
}
