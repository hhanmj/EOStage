use insar_core::{
    dns_name_matches, AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters,
};
use reqwest::Url;

const APPROVED_ASF_ASSET_HOSTS: &[&str] = &[
    "asf.alaska.edu",
    "earthdata.nasa.gov",
    "earthdatacloud.nasa.gov",
];

pub(super) fn earthdata_https_descriptor(stable_href: &str) -> Option<AssetAccessDescriptor> {
    let url = Url::parse(stable_href).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
        || !url.host_str().is_some_and(is_approved_asf_asset_host)
    {
        return None;
    }
    let descriptor = AssetAccessDescriptor {
        resolver_id: "earthdata_https".to_string(),
        scheme: AssetAccessScheme::EarthdataHttps,
        stable_href: Some(url.to_string()),
        requires_credentials: true,
        credential_profile: Some("earthdata-default".to_string()),
        resolver_parameters: AssetResolverParameters {},
    };
    descriptor.validate().ok().map(|()| descriptor)
}

fn is_approved_asf_asset_host(host: &str) -> bool {
    APPROVED_ASF_ASSET_HOSTS
        .iter()
        .any(|allowed| dns_name_matches(host, allowed))
}
