use std::net::IpAddr;

use insar_core::{dns_name_matches, normalize_dns_name, FrozenAssetAccess};
use reqwest::Url;

use super::{
    ResolveError, ShadowAccessIntent, ShadowAuthKind, ShadowHostClass, ShadowPathClass,
    ShadowResolveContext,
};

pub(crate) fn plan_public_https(
    frozen: &FrozenAssetAccess,
    context: &ShadowResolveContext<'_>,
) -> Result<ShadowAccessIntent, ResolveError> {
    plan_https(frozen, context, ShadowAuthKind::None, HttpsPolicy::Public)
}

pub(crate) fn plan_earthdata_https(
    frozen: &FrozenAssetAccess,
    context: &ShadowResolveContext<'_>,
    auth_kind: ShadowAuthKind,
) -> Result<ShadowAccessIntent, ResolveError> {
    plan_https(frozen, context, auth_kind, HttpsPolicy::Earthdata)
}

pub(crate) fn plan_local_file(
    frozen: &FrozenAssetAccess,
    context: &ShadowResolveContext<'_>,
) -> Result<ShadowAccessIntent, ResolveError> {
    frozen
        .validate()
        .map_err(|_| ResolveError::descriptor_invalid())?;
    context.checkpoint()?;
    let href = frozen
        .descriptor
        .stable_href
        .as_deref()
        .ok_or_else(ResolveError::reference_invalid)?;
    let url = Url::parse(href).map_err(|_| ResolveError::reference_invalid())?;
    if url.scheme() != "file"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ResolveError::reference_invalid());
    }
    context.checkpoint()?;
    let file_name = frozen
        .file_name
        .clone()
        .or_else(|| suggested_filename(&url));
    context.checkpoint()?;
    intent_from_frozen(
        frozen,
        ShadowHostClass::LocalFile,
        ShadowPathClass::NativeFile,
        file_name,
        ShadowAuthKind::None,
    )
}

fn plan_https(
    frozen: &FrozenAssetAccess,
    context: &ShadowResolveContext<'_>,
    auth_kind: ShadowAuthKind,
    policy: HttpsPolicy,
) -> Result<ShadowAccessIntent, ResolveError> {
    frozen
        .validate()
        .map_err(|_| ResolveError::descriptor_invalid())?;
    context.checkpoint()?;
    let href = frozen
        .descriptor
        .stable_href
        .as_deref()
        .ok_or_else(ResolveError::access_not_durable)?;
    let url = Url::parse(href).map_err(|_| ResolveError::access_not_durable())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err(ResolveError::access_not_durable());
    }
    let host = url
        .host_str()
        .ok_or_else(ResolveError::access_not_durable)?;
    let host_is_allowed = match policy {
        HttpsPolicy::Public => is_public_host(host),
        HttpsPolicy::Earthdata => is_approved_earthdata_host(host),
    };
    if !host_is_allowed {
        return Err(ResolveError::access_not_durable());
    }
    let host_class = classify_host(host, url.path());
    context.checkpoint()?;
    let file_name = frozen
        .file_name
        .clone()
        .or_else(|| suggested_filename(&url));
    let path_class = classify_path(host_class, file_name.as_deref(), url.path());
    context.checkpoint()?;
    intent_from_frozen(frozen, host_class, path_class, file_name, auth_kind)
}

fn intent_from_frozen(
    frozen: &FrozenAssetAccess,
    host_class: ShadowHostClass,
    path_class: ShadowPathClass,
    file_name: Option<String>,
    auth_kind: ShadowAuthKind,
) -> Result<ShadowAccessIntent, ResolveError> {
    ShadowAccessIntent::from_parts(
        frozen.reference.provider_id.clone(),
        frozen.descriptor.scheme.clone(),
        host_class,
        path_class,
        file_name,
        auth_kind,
        frozen.expected_size_bytes,
        frozen.checksum_algorithm.clone(),
        frozen.checksum_value.clone(),
        frozen.role.clone(),
    )
}

pub(crate) fn classify_host(host: &str, path: &str) -> ShadowHostClass {
    if dns_name_matches(host, "sentinel1-burst.asf.alaska.edu") {
        ShadowHostClass::AsfBurst
    } else if dns_name_matches(host, "earthdatacloud.nasa.gov")
        || dns_name_matches(host, "earthdata.nasa.gov")
    {
        ShadowHostClass::EarthdataCloud
    } else if dns_name_matches(host, "asf.alaska.edu")
        && path.to_ascii_lowercase().ends_with(".eof")
    {
        ShadowHostClass::AsfOrbit
    } else if dns_name_matches(host, "asf.alaska.edu") {
        ShadowHostClass::AsfDatapool
    } else {
        ShadowHostClass::PublicHttpsOther
    }
}

#[derive(Clone, Copy)]
enum HttpsPolicy {
    Public,
    Earthdata,
}

fn is_approved_earthdata_host(host: &str) -> bool {
    [
        "earthdata.nasa.gov",
        "earthdatacloud.nasa.gov",
        "asf.alaska.edu",
    ]
    .iter()
    .any(|allowed| dns_name_matches(host, allowed))
}

fn is_public_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if normalized.is_empty()
        || normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || normalized.ends_with(".internal")
    {
        return false;
    }
    match normalized.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_multicast())
        }
        Ok(IpAddr::V6(address)) => {
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast())
        }
        Err(_) => normalize_dns_name(&normalized).is_ok_and(|name| name.contains('.')),
    }
}

pub(crate) fn classify_path(
    host_class: ShadowHostClass,
    file_name: Option<&str>,
    path: &str,
) -> ShadowPathClass {
    let candidate = file_name.unwrap_or(path).to_ascii_lowercase();
    if host_class == ShadowHostClass::AsfBurst && candidate.ends_with(".tiff") {
        ShadowPathClass::BurstTiff
    } else if host_class == ShadowHostClass::AsfBurst && candidate.ends_with(".xml") {
        ShadowPathClass::BurstXml
    } else if candidate.ends_with(".eof") {
        ShadowPathClass::OrbitEof
    } else if candidate.ends_with(".zip") {
        ShadowPathClass::Archive
    } else {
        ShadowPathClass::NativeFile
    }
}

fn suggested_filename(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.rfind(|part| !part.is_empty()))
        .filter(|name| name != &"." && name != &"..")
        .map(str::to_string)
}
