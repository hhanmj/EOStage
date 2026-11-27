use std::io::{Read, Result as IoResult};
use std::time::Duration;

use insar_core::{sanitize_external_error_message, ProviderNetworkPolicy};
use reqwest::blocking::{Body, Client, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde::Serialize;
use serde_json::{Map, Value};
use zeroize::{Zeroize, Zeroizing};

use super::error::{UsgsM2mEndpointClass, UsgsM2mError, UsgsM2mErrorCode};
use super::session::{UsgsM2mApiKey, UsgsM2mLoginMaterial};

pub const USGS_M2M_STABLE_ORIGIN: &str = "https://m2m.cr.usgs.gov/api/api/json/stable/";
pub const USGS_M2M_AUTH_HEADER: &str = "X-Auth-Token";

const MAX_CONTROL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LOGIN_RESPONSE_BYTES: u64 = 64 * 1024;
const MAX_LOGOUT_RESPONSE_BYTES: u64 = 64 * 1024;
const READ_CHUNK_BYTES: usize = 8 * 1024;
const MAX_RETRY_AFTER_SECONDS: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UsgsM2mEndpoint {
    LoginToken,
    Permissions,
    DatasetDiscovery,
    DatasetFilters,
    SceneSearch,
    DownloadOptions,
    DownloadRequest,
    DownloadRetrieve,
    Logout,
}

impl UsgsM2mEndpoint {
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::LoginToken => "login-token",
            Self::Permissions => "permissions",
            Self::DatasetDiscovery => "dataset-search",
            Self::DatasetFilters => "dataset-filters",
            Self::SceneSearch => "scene-search",
            Self::DownloadOptions => "download-options",
            Self::DownloadRequest => "download-request",
            Self::DownloadRetrieve => "download-retrieve",
            Self::Logout => "logout",
        }
    }

    pub(crate) const fn class(self) -> UsgsM2mEndpointClass {
        match self {
            Self::LoginToken => UsgsM2mEndpointClass::LoginToken,
            Self::Permissions => UsgsM2mEndpointClass::Permissions,
            Self::DatasetDiscovery => UsgsM2mEndpointClass::DatasetDiscovery,
            Self::DatasetFilters => UsgsM2mEndpointClass::DatasetFilters,
            Self::SceneSearch => UsgsM2mEndpointClass::SceneSearch,
            Self::DownloadOptions => UsgsM2mEndpointClass::DownloadOptions,
            Self::DownloadRequest => UsgsM2mEndpointClass::DownloadRequest,
            Self::DownloadRetrieve => UsgsM2mEndpointClass::DownloadRetrieve,
            Self::Logout => UsgsM2mEndpointClass::Logout,
        }
    }

    const fn requires_auth(self) -> bool {
        !matches!(self, Self::LoginToken)
    }

    const fn response_limit(self) -> u64 {
        match self {
            Self::LoginToken => MAX_LOGIN_RESPONSE_BYTES,
            Self::Permissions => 64 * 1024,
            Self::DatasetDiscovery => 2 * 1024 * 1024,
            Self::DatasetFilters => 4 * 1024 * 1024,
            Self::SceneSearch => 16 * 1024 * 1024,
            Self::DownloadOptions => 8 * 1024 * 1024,
            Self::DownloadRequest | Self::DownloadRetrieve => 4 * 1024 * 1024,
            Self::Logout => MAX_LOGOUT_RESPONSE_BYTES,
        }
    }
}

pub(crate) trait UsgsM2mSessionEndpoint: Send + Sync {
    fn network_policy(&self) -> ProviderNetworkPolicy;

    fn login_token(
        &self,
        material: &UsgsM2mLoginMaterial,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mApiKey, UsgsM2mError>;

    fn logout(
        &self,
        api_key: &UsgsM2mApiKey,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<(), UsgsM2mError>;

    fn permissions(
        &self,
        _api_key: &UsgsM2mApiKey,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            UsgsM2mEndpointClass::Permissions,
        ))
    }

    fn dataset_search(
        &self,
        api_key: &UsgsM2mApiKey,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError>;

    fn dataset_filters(
        &self,
        _api_key: &UsgsM2mApiKey,
        _dataset_name: &str,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            UsgsM2mEndpointClass::DatasetFilters,
        ))
    }

    fn scene_search(
        &self,
        _api_key: &UsgsM2mApiKey,
        _payload: &Value,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            UsgsM2mEndpointClass::SceneSearch,
        ))
    }

    fn download_options(
        &self,
        _api_key: &UsgsM2mApiKey,
        _payload: &Value,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            UsgsM2mEndpointClass::DownloadOptions,
        ))
    }

    fn download_request(
        &self,
        _api_key: &UsgsM2mApiKey,
        _payload: &Value,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            UsgsM2mEndpointClass::DownloadRequest,
        ))
    }

    fn download_retrieve(
        &self,
        _api_key: &UsgsM2mApiKey,
        _payload: &Value,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            UsgsM2mEndpointClass::DownloadRetrieve,
        ))
    }
}

pub(crate) struct ReqwestUsgsM2mEndpoint {
    client: Client,
    origin: Url,
    network_policy: ProviderNetworkPolicy,
    origin_policy: OriginPolicy,
}

#[derive(Clone, Copy)]
enum OriginPolicy {
    Production,
    #[cfg(test)]
    Loopback,
}

impl ReqwestUsgsM2mEndpoint {
    pub(crate) fn new(
        network_policy: ProviderNetworkPolicy,
        timeout: Duration,
    ) -> Result<Self, UsgsM2mError> {
        if timeout.is_zero() || timeout > MAX_CONTROL_TIMEOUT {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ConfigInvalid,
                UsgsM2mEndpointClass::RuntimeSession,
            ));
        }
        let origin = Url::parse(USGS_M2M_STABLE_ORIGIN).map_err(|_| {
            UsgsM2mError::new(
                UsgsM2mErrorCode::ConfigInvalid,
                UsgsM2mEndpointClass::RuntimeSession,
            )
        })?;
        validate_production_origin(&origin)?;
        let client = Client::builder()
            .connect_timeout(timeout.min(Duration::from_secs(15)))
            .timeout(timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false);
        let client =
            crate::network_proxy::blocking_builder_with_proxy(client, network_policy.route())
                .build()
                .map_err(|_| {
                    UsgsM2mError::new(
                        UsgsM2mErrorCode::ConfigInvalid,
                        UsgsM2mEndpointClass::RuntimeSession,
                    )
                })?;
        Ok(Self {
            client,
            origin,
            network_policy,
            origin_policy: OriginPolicy::Production,
        })
    }

    #[cfg(test)]
    fn new_for_loopback(origin: Url, timeout: Duration) -> Result<Self, UsgsM2mError> {
        if timeout.is_zero() || timeout > MAX_CONTROL_TIMEOUT {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ConfigInvalid,
                UsgsM2mEndpointClass::RuntimeSession,
            ));
        }
        validate_loopback_origin(&origin)?;
        let client = Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .tls_built_in_root_certs(false)
            .no_proxy()
            .build()
            .map_err(|_| {
                UsgsM2mError::new(
                    UsgsM2mErrorCode::ConfigInvalid,
                    UsgsM2mEndpointClass::RuntimeSession,
                )
            })?;
        Ok(Self {
            client,
            origin,
            network_policy: ProviderNetworkPolicy::direct(),
            origin_policy: OriginPolicy::Loopback,
        })
    }

    fn endpoint_url(&self, endpoint: UsgsM2mEndpoint) -> Result<Url, UsgsM2mError> {
        let url = self
            .origin
            .join(endpoint.path())
            .map_err(|_| UsgsM2mError::new(UsgsM2mErrorCode::ConfigInvalid, endpoint.class()))?;
        match self.origin_policy {
            OriginPolicy::Production => validate_endpoint_url(&url, endpoint)?,
            #[cfg(test)]
            OriginPolicy::Loopback => validate_loopback_endpoint(&url, endpoint)?,
        }
        Ok(url)
    }

    fn post_json<T: Serialize>(
        &self,
        endpoint: UsgsM2mEndpoint,
        payload: &T,
        api_key: Option<&UsgsM2mApiKey>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        if cancelled() {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::Cancelled,
                endpoint.class(),
            ));
        }
        if endpoint.requires_auth() != api_key.is_some() {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ConfigInvalid,
                endpoint.class(),
            ));
        }

        let mut bytes = Zeroizing::new(Vec::new());
        serde_json::to_writer(&mut *bytes, payload)
            .map_err(|_| UsgsM2mError::new(UsgsM2mErrorCode::ConfigInvalid, endpoint.class()))?;
        let body_length = bytes.len() as u64;
        let body = Body::sized(ZeroizingReader::new(bytes), body_length);
        let mut request = self
            .client
            .post(self.endpoint_url(endpoint)?)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(api_key) = api_key {
            request = request.header(USGS_M2M_AUTH_HEADER, api_key.expose());
        }
        let response = request.send().map_err(|error| {
            let (message, redacted) = sanitize_external_error_message(&error.to_string());
            UsgsM2mError::new(
                if error.is_timeout() {
                    UsgsM2mErrorCode::Timeout
                } else {
                    UsgsM2mErrorCode::TransportFailed
                },
                endpoint.class(),
            )
            .with_transport_error(message, redacted)
        })?;
        self.parse_response(endpoint, response, cancelled)
    }

    fn parse_response(
        &self,
        endpoint: UsgsM2mEndpoint,
        response: Response,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        let status = response.status();
        let retry_after = retry_after_seconds(&response);
        if status.is_redirection() {
            return Err(
                UsgsM2mError::new(UsgsM2mErrorCode::RedirectRejected, endpoint.class())
                    .with_http_status(status.as_u16())
                    .with_retry_after(retry_after),
            );
        }

        let limit = endpoint.response_limit();
        if status != StatusCode::OK {
            let fallback = status_error(endpoint, status, retry_after);
            if !has_json_content_type(&response)
                || response
                    .content_length()
                    .is_some_and(|length| length > limit)
            {
                return Err(fallback);
            }
            let body = match read_bounded_body(response, limit, endpoint.class(), cancelled) {
                Ok(body) => body,
                Err(error) => {
                    return Err(error
                        .with_http_status(status.as_u16())
                        .with_retry_after(retry_after))
                }
            };
            if let Some(error) = parse_http_error_envelope(&body, endpoint, status, retry_after) {
                return Err(error);
            }
            return Err(fallback);
        }

        if !has_json_content_type(&response) {
            return Err(
                UsgsM2mError::new(UsgsM2mErrorCode::ResponseInvalid, endpoint.class())
                    .with_http_status(status.as_u16()),
            );
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(
                UsgsM2mError::new(UsgsM2mErrorCode::ResponseTooLarge, endpoint.class())
                    .with_http_status(status.as_u16()),
            );
        }
        let body = read_bounded_body(response, limit, endpoint.class(), cancelled)?;
        parse_envelope(&body, endpoint.class())
    }
}

impl UsgsM2mSessionEndpoint for ReqwestUsgsM2mEndpoint {
    fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    fn login_token(
        &self,
        material: &UsgsM2mLoginMaterial,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mApiKey, UsgsM2mError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct LoginTokenPayload<'a> {
            username: &'a str,
            token: &'a str,
        }

        let payload = LoginTokenPayload {
            username: material.username(),
            token: material.application_token(),
        };
        let mut data = self.post_json(UsgsM2mEndpoint::LoginToken, &payload, None, cancelled)?;
        match data.take() {
            Value::String(api_key) => UsgsM2mApiKey::from_wire(api_key),
            mut invalid => {
                zeroize_json(&mut invalid);
                Err(UsgsM2mError::new(
                    UsgsM2mErrorCode::ResponseInvalid,
                    UsgsM2mEndpointClass::LoginToken,
                ))
            }
        }
    }

    fn logout(
        &self,
        api_key: &UsgsM2mApiKey,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<(), UsgsM2mError> {
        let mut data = self.post_json(
            UsgsM2mEndpoint::Logout,
            &Option::<()>::None,
            Some(api_key),
            cancelled,
        )?;
        match data.take() {
            Value::Null => Ok(()),
            mut invalid => {
                zeroize_json(&mut invalid);
                Err(UsgsM2mError::new(
                    UsgsM2mErrorCode::ResponseInvalid,
                    UsgsM2mEndpointClass::Logout,
                ))
            }
        }
    }

    fn permissions(
        &self,
        api_key: &UsgsM2mApiKey,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        self.post_json(
            UsgsM2mEndpoint::Permissions,
            &Option::<()>::None,
            Some(api_key),
            cancelled,
        )
    }

    fn dataset_search(
        &self,
        api_key: &UsgsM2mApiKey,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct DatasetSearchPayload {
            dataset_name: &'static str,
        }

        self.post_json(
            UsgsM2mEndpoint::DatasetDiscovery,
            &DatasetSearchPayload {
                dataset_name: "Landsat 8-9",
            },
            Some(api_key),
            cancelled,
        )
    }

    fn dataset_filters(
        &self,
        api_key: &UsgsM2mApiKey,
        dataset_name: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct DatasetFiltersPayload<'a> {
            dataset_name: &'a str,
        }

        self.post_json(
            UsgsM2mEndpoint::DatasetFilters,
            &DatasetFiltersPayload { dataset_name },
            Some(api_key),
            cancelled,
        )
    }

    fn scene_search(
        &self,
        api_key: &UsgsM2mApiKey,
        payload: &Value,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        self.post_json(
            UsgsM2mEndpoint::SceneSearch,
            payload,
            Some(api_key),
            cancelled,
        )
    }

    fn download_options(
        &self,
        api_key: &UsgsM2mApiKey,
        payload: &Value,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        self.post_json(
            UsgsM2mEndpoint::DownloadOptions,
            payload,
            Some(api_key),
            cancelled,
        )
    }

    fn download_request(
        &self,
        api_key: &UsgsM2mApiKey,
        payload: &Value,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        self.post_json(
            UsgsM2mEndpoint::DownloadRequest,
            payload,
            Some(api_key),
            cancelled,
        )
    }

    fn download_retrieve(
        &self,
        api_key: &UsgsM2mApiKey,
        payload: &Value,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SensitiveJson, UsgsM2mError> {
        self.post_json(
            UsgsM2mEndpoint::DownloadRetrieve,
            payload,
            Some(api_key),
            cancelled,
        )
    }
}

fn validate_production_origin(origin: &Url) -> Result<(), UsgsM2mError> {
    if origin.scheme() != "https"
        || origin.host_str() != Some("m2m.cr.usgs.gov")
        || origin.port_or_known_default() != Some(443)
        || origin.path() != "/api/api/json/stable/"
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            UsgsM2mEndpointClass::RuntimeSession,
        ));
    }
    Ok(())
}

fn validate_endpoint_url(url: &Url, endpoint: UsgsM2mEndpoint) -> Result<(), UsgsM2mError> {
    let expected_path = format!("/api/api/json/stable/{}", endpoint.path());
    if url.scheme() != "https"
        || url.host_str() != Some("m2m.cr.usgs.gov")
        || url.port_or_known_default() != Some(443)
        || url.path() != expected_path
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            endpoint.class(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn validate_loopback_origin(origin: &Url) -> Result<(), UsgsM2mError> {
    if origin.scheme() != "http"
        || !matches!(origin.host_str(), Some("127.0.0.1") | Some("::1"))
        || origin.port().is_none()
        || origin.path() != "/api/api/json/stable/"
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            UsgsM2mEndpointClass::RuntimeSession,
        ));
    }
    Ok(())
}

#[cfg(test)]
fn validate_loopback_endpoint(url: &Url, endpoint: UsgsM2mEndpoint) -> Result<(), UsgsM2mError> {
    let expected_path = format!("/api/api/json/stable/{}", endpoint.path());
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1") | Some("::1"))
        || url.port().is_none()
        || url.path() != expected_path
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(UsgsM2mError::new(
            UsgsM2mErrorCode::ConfigInvalid,
            endpoint.class(),
        ));
    }
    Ok(())
}

fn retry_after_seconds(response: &Response) -> Option<u64> {
    (response.status() == StatusCode::TOO_MANY_REQUESTS)
        .then(|| {
            response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|seconds| *seconds <= MAX_RETRY_AFTER_SECONDS)
        })
        .flatten()
}

fn status_error(
    endpoint: UsgsM2mEndpoint,
    status: StatusCode,
    retry_after: Option<u64>,
) -> UsgsM2mError {
    let code = match status {
        StatusCode::UNAUTHORIZED if endpoint == UsgsM2mEndpoint::LoginToken => {
            UsgsM2mErrorCode::AuthInvalid
        }
        StatusCode::UNAUTHORIZED => UsgsM2mErrorCode::AuthUnauthorized,
        StatusCode::FORBIDDEN => UsgsM2mErrorCode::AuthUnauthorized,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => UsgsM2mErrorCode::Timeout,
        StatusCode::TOO_MANY_REQUESTS => UsgsM2mErrorCode::RateLimited,
        _ => UsgsM2mErrorCode::RemoteError,
    };
    UsgsM2mError::new(code, endpoint.class())
        .with_http_status(status.as_u16())
        .with_retry_after(retry_after)
}

fn parse_http_error_envelope(
    body: &[u8],
    endpoint: UsgsM2mEndpoint,
    status: StatusCode,
    retry_after: Option<u64>,
) -> Option<UsgsM2mError> {
    match parse_envelope(body, endpoint.class()) {
        Err(error) if error.remote_code().is_some() => Some(
            error
                .with_http_status(status.as_u16())
                .with_retry_after(retry_after),
        ),
        _ => None,
    }
}

fn has_json_content_type(response: &Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"))
}

fn read_bounded_body(
    mut response: Response,
    limit: u64,
    endpoint: UsgsM2mEndpointClass,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Zeroizing<Vec<u8>>, UsgsM2mError> {
    let mut body = Zeroizing::new(Vec::new());
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        if cancelled() {
            return Err(UsgsM2mError::new(UsgsM2mErrorCode::Cancelled, endpoint));
        }
        let read = response.read(&mut chunk).map_err(|error| {
            let (message, redacted) = sanitize_external_error_message(&error.to_string());
            UsgsM2mError::new(UsgsM2mErrorCode::TransportFailed, endpoint)
                .with_transport_error(message, redacted)
        })?;
        if read == 0 {
            break;
        }
        if body.len().saturating_add(read) as u64 > limit {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ResponseTooLarge,
                endpoint,
            ));
        }
        body.extend_from_slice(&chunk[..read]);
        chunk[..read].zeroize();
        if cancelled() {
            return Err(UsgsM2mError::new(UsgsM2mErrorCode::Cancelled, endpoint));
        }
    }
    Ok(body)
}

pub(crate) struct SensitiveJson(Value);

impl SensitiveJson {
    pub(crate) fn new(value: Value) -> Self {
        Self(value)
    }

    pub(crate) fn take(&mut self) -> Value {
        std::mem::replace(&mut self.0, Value::Null)
    }

    pub(crate) fn value(&self) -> &Value {
        &self.0
    }
}

impl Drop for SensitiveJson {
    fn drop(&mut self) {
        zeroize_json(&mut self.0);
    }
}

fn parse_envelope(
    body: &[u8],
    endpoint: UsgsM2mEndpointClass,
) -> Result<SensitiveJson, UsgsM2mError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| UsgsM2mError::new(UsgsM2mErrorCode::ResponseInvalid, endpoint))?;
    let mut protected = SensitiveJson::new(value);
    let map = protected
        .0
        .as_object_mut()
        .ok_or_else(|| UsgsM2mError::new(UsgsM2mErrorCode::ResponseInvalid, endpoint))?;

    validate_request_id(take_required(map, "requestId", endpoint)?, endpoint)?;
    validate_version(take_required(map, "version", endpoint)?, endpoint)?;
    let (remote_message, remote_message_redacted) =
        take_remote_message(take_required(map, "errorMessage", endpoint)?, endpoint)?;
    let error_code = take_required(map, "errorCode", endpoint)?;
    match error_code {
        Value::Null => {}
        Value::String(mut remote_code) => {
            if !valid_bounded_text(&remote_code, 128, true) {
                remote_code.zeroize();
                return Err(UsgsM2mError::new(
                    UsgsM2mErrorCode::ResponseInvalid,
                    endpoint,
                ));
            }
            let error = UsgsM2mError::from_remote_code(endpoint, &remote_code).with_remote_error(
                remote_code.clone(),
                remote_message,
                remote_message_redacted,
            );
            remote_code.zeroize();
            return Err(error);
        }
        mut invalid => {
            zeroize_json(&mut invalid);
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ResponseInvalid,
                endpoint,
            ));
        }
    }
    let data = take_required(map, "data", endpoint)?;
    Ok(SensitiveJson::new(data))
}

fn take_required(
    map: &mut Map<String, Value>,
    key: &str,
    endpoint: UsgsM2mEndpointClass,
) -> Result<Value, UsgsM2mError> {
    map.remove(key)
        .ok_or_else(|| UsgsM2mError::new(UsgsM2mErrorCode::ResponseInvalid, endpoint))
}

fn validate_request_id(value: Value, endpoint: UsgsM2mEndpointClass) -> Result<(), UsgsM2mError> {
    match value {
        Value::Number(number) if number.as_u64().is_some() => Ok(()),
        Value::String(mut request_id) if valid_bounded_text(&request_id, 256, false) => {
            request_id.zeroize();
            Ok(())
        }
        mut invalid => {
            zeroize_json(&mut invalid);
            Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ResponseInvalid,
                endpoint,
            ))
        }
    }
}

fn validate_version(value: Value, endpoint: UsgsM2mEndpointClass) -> Result<(), UsgsM2mError> {
    match value {
        Value::String(mut version) if valid_bounded_text(&version, 64, true) => {
            version.zeroize();
            Ok(())
        }
        mut invalid => {
            zeroize_json(&mut invalid);
            Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ResponseInvalid,
                endpoint,
            ))
        }
    }
}

fn take_remote_message(
    value: Value,
    endpoint: UsgsM2mEndpointClass,
) -> Result<(Option<String>, bool), UsgsM2mError> {
    match value {
        Value::Null => Ok((None, false)),
        Value::String(mut message) if message.len() <= 16 * 1024 => {
            let (safe, redacted) = sanitize_external_error_message(&message);
            message.zeroize();
            Ok(((!safe.is_empty()).then_some(safe), redacted))
        }
        mut invalid => {
            zeroize_json(&mut invalid);
            Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ResponseInvalid,
                endpoint,
            ))
        }
    }
}

fn valid_bounded_text(value: &str, max_bytes: usize, ascii: bool) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && (!ascii || value.is_ascii())
        && !value.chars().any(char::is_control)
}

pub(crate) fn zeroize_json(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(zeroize_json),
        Value::Object(values) => {
            for (mut key, mut value) in std::mem::take(values) {
                key.zeroize();
                zeroize_json(&mut value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    *value = Value::Null;
}

struct ZeroizingReader {
    bytes: Zeroizing<Vec<u8>>,
    offset: usize,
}

impl ZeroizingReader {
    fn new(bytes: Zeroizing<Vec<u8>>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl Read for ZeroizingReader {
    fn read(&mut self, output: &mut [u8]) -> IoResult<usize> {
        let remaining = &self.bytes[self.offset..];
        let count = remaining.len().min(output.len());
        output[..count].copy_from_slice(&remaining[..count]);
        self.offset += count;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver};
    use std::thread::{self, JoinHandle};

    use serde_json::json;

    use super::*;

    struct LoopbackExchange {
        origin: Url,
        request: Receiver<Vec<u8>>,
        server: JoinHandle<()>,
    }

    fn response(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn redirect_response(location: &Url) -> Vec<u8> {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    }

    fn serve_once(response: Vec<u8>) -> LoopbackExchange {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let origin = Url::parse(&format!("http://{address}/api/api/json/stable/")).unwrap();
        let (sender, request) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let captured = read_http_request(&mut stream);
            sender.send(captured).unwrap();
            stream.write_all(&response).unwrap();
        });
        LoopbackExchange {
            origin,
            request,
            server,
        }
    }

    fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        let header_end = loop {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "loopback request closed before headers");
            request.extend_from_slice(&chunk[..read]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
            assert!(
                request.len() <= 32 * 1024,
                "loopback request headers too large"
            );
        };
        let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "loopback request closed before body");
            request.extend_from_slice(&chunk[..read]);
        }
        request
    }

    fn login_material() -> UsgsM2mLoginMaterial {
        UsgsM2mLoginMaterial::new(
            "fixture-ers-user",
            super::super::session::UsgsM2mApplicationToken::new("fixture-application-token")
                .unwrap(),
        )
        .unwrap()
    }

    fn success_envelope(data: &str) -> Vec<u8> {
        format!(
            r#"{{"requestId":17,"version":"stable","data":{data},"errorCode":null,"errorMessage":null}}"#
        )
        .into_bytes()
    }

    fn finish(exchange: LoopbackExchange) -> Vec<u8> {
        let request = exchange
            .request
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        exchange.server.join().unwrap();
        request
    }

    #[test]
    fn production_origin_and_endpoint_set_are_exact() {
        let origin = Url::parse(USGS_M2M_STABLE_ORIGIN).unwrap();
        validate_production_origin(&origin).unwrap();
        assert_eq!(UsgsM2mEndpoint::LoginToken.path(), "login-token");
        assert_eq!(UsgsM2mEndpoint::Permissions.path(), "permissions");
        assert_eq!(UsgsM2mEndpoint::DatasetDiscovery.path(), "dataset-search");
        assert_eq!(UsgsM2mEndpoint::DatasetFilters.path(), "dataset-filters");
        assert_eq!(UsgsM2mEndpoint::SceneSearch.path(), "scene-search");
        assert_eq!(UsgsM2mEndpoint::DownloadOptions.path(), "download-options");
        assert_eq!(UsgsM2mEndpoint::DownloadRequest.path(), "download-request");
        assert_eq!(
            UsgsM2mEndpoint::DownloadRetrieve.path(),
            "download-retrieve"
        );
        assert_eq!(UsgsM2mEndpoint::Logout.path(), "logout");

        for invalid in [
            "http://m2m.cr.usgs.gov/api/api/json/stable/",
            "https://m2m.cr.usgs.gov:444/api/api/json/stable/",
            "https://m2m.cr.usgs.gov.evil.example/api/api/json/stable/",
            "https://user@m2m.cr.usgs.gov/api/api/json/stable/",
            "https://m2m.cr.usgs.gov/api/api/json/stable/?endpoint=login-token",
        ] {
            assert!(validate_production_origin(&Url::parse(invalid).unwrap()).is_err());
        }
    }

    #[test]
    #[ignore = "requires public USGS M2M network; uses deliberately invalid fixture credentials"]
    fn production_login_token_tls_reaches_usgs_with_invalid_fixture_credentials() {
        let client =
            ReqwestUsgsM2mEndpoint::new(ProviderNetworkPolicy::system(), Duration::from_secs(20))
                .unwrap();
        let material = UsgsM2mLoginMaterial::new(
            "sceneport-connectivity-probe-invalid",
            super::super::session::UsgsM2mApplicationToken::new(
                "sceneport-connectivity-probe-invalid-token",
            )
            .unwrap(),
        )
        .unwrap();

        let error = client
            .login_token(&material, &|| false)
            .expect_err("deliberately invalid credentials must not authenticate");
        assert!(
            !matches!(
                error.code(),
                UsgsM2mErrorCode::ConfigInvalid
                    | UsgsM2mErrorCode::TransportFailed
                    | UsgsM2mErrorCode::Timeout
                    | UsgsM2mErrorCode::RedirectRejected
                    | UsgsM2mErrorCode::Cancelled
            ),
            "public USGS endpoint must be reached through TLS; got {error:?}"
        );
    }

    #[test]
    fn envelope_requires_all_fields_and_maps_only_fixed_remote_codes() {
        let valid = br#"{
            "requestId": 7,
            "version": "stable",
            "data": "fixture-runtime-key",
            "errorCode": null,
            "errorMessage": null
        }"#;
        let mut parsed = parse_envelope(valid, UsgsM2mEndpointClass::LoginToken).unwrap();
        assert_eq!(
            parsed.take(),
            Value::String("fixture-runtime-key".to_owned())
        );

        let remote = br#"{
            "requestId": "fixture-request",
            "version": "stable",
            "data": null,
            "errorCode": "AUTH_KEY_EXPIRED",
            "errorMessage": "sensitive remote detail"
        }"#;
        let remote_error = parse_envelope(remote, UsgsM2mEndpointClass::DatasetDiscovery)
            .err()
            .expect("remote error");
        assert_eq!(remote_error.code(), UsgsM2mErrorCode::AuthExpired);
        assert_eq!(remote_error.remote_code(), Some("AUTH_KEY_EXPIRED"));
        assert_eq!(
            remote_error.remote_message(),
            Some("sensitive remote detail")
        );
        assert!(!remote_error.remote_message_redacted());

        let unknown = br#"{
            "requestId": 9,
            "version": "stable",
            "data": null,
            "errorCode": "UNREVIEWED_DYNAMIC_CODE",
            "errorMessage": null
        }"#;
        let unknown_error = parse_envelope(unknown, UsgsM2mEndpointClass::DatasetDiscovery)
            .err()
            .expect("unknown remote error");
        assert_eq!(unknown_error.code(), UsgsM2mErrorCode::RemoteError);
        assert_eq!(unknown_error.remote_code(), Some("UNREVIEWED_DYNAMIC_CODE"));

        let secret_bearing = br#"{
            "requestId": 10,
            "version": "stable",
            "data": null,
            "errorCode": "UNREVIEWED_REMOTE_FAILURE",
            "errorMessage": "request failed https://example.test/path?signature=secret token=abc status=403"
        }"#;
        let safe_error = parse_envelope(secret_bearing, UsgsM2mEndpointClass::DownloadRequest)
            .err()
            .expect("secret-bearing remote error");
        assert_eq!(safe_error.remote_code(), Some("UNREVIEWED_REMOTE_FAILURE"));
        let message = safe_error.remote_message().expect("safe upstream message");
        assert!(safe_error.remote_message_redacted());
        assert!(message.contains("request failed"));
        assert!(message.contains("status=403"));
        assert!(message.contains("token=[REDACTED]"));
        assert!(message.contains("?REDACTED"));
        assert!(!message.contains("signature=secret"));
        assert!(!message.contains("token=abc"));

        for missing in [
            br#"{"version":"stable","data":null,"errorCode":null,"errorMessage":null}"#.as_slice(),
            br#"{"requestId":1,"data":null,"errorCode":null,"errorMessage":null}"#.as_slice(),
            br#"{"requestId":1,"version":"stable","errorCode":null,"errorMessage":null}"#
                .as_slice(),
            br#"{"requestId":1,"version":"stable","data":null,"errorMessage":null}"#.as_slice(),
        ] {
            assert_eq!(
                parse_envelope(missing, UsgsM2mEndpointClass::LoginToken)
                    .err()
                    .expect("missing envelope field")
                    .code(),
                UsgsM2mErrorCode::ResponseInvalid
            );
        }
    }

    #[test]
    fn non_200_standard_envelope_preserves_http_and_upstream_error() {
        let body = br#"{
            "requestId": 11,
            "version": "stable",
            "data": null,
            "errorCode": "AUTH_NOT_AUTHORIZED",
            "errorMessage": "Machine-to-machine download permission is not enabled for this account."
        }"#;
        let exchange = serve_once(response("403 Forbidden", "application/json", body));
        let client = ReqwestUsgsM2mEndpoint::new_for_loopback(
            exchange.origin.clone(),
            Duration::from_secs(2),
        )
        .unwrap();
        let api_key = UsgsM2mApiKey::from_wire("fixture-runtime-api-key".to_owned()).unwrap();

        let error = match client.permissions(&api_key, &|| false) {
            Err(error) => error,
            Ok(_) => panic!("403 M2M envelope must remain an error"),
        };
        assert_eq!(error.code(), UsgsM2mErrorCode::AuthUnauthorized);
        assert_eq!(error.http_status(), Some(403));
        assert_eq!(error.remote_code(), Some("AUTH_NOT_AUTHORIZED"));
        assert_eq!(
            error.remote_message(),
            Some("Machine-to-machine download permission is not enabled for this account.")
        );
        assert!(!error.remote_message_redacted());
        let request = String::from_utf8(finish(exchange)).unwrap();
        assert!(request.starts_with("POST /api/api/json/stable/permissions HTTP/1.1\r\n"));
    }

    #[test]
    fn loopback_login_permissions_and_logout_use_exact_runtime_header_boundary() {
        let login = serve_once(response(
            "200 OK",
            "application/json; charset=utf-8",
            &success_envelope("\"fixture-runtime-api-key\""),
        ));
        let client =
            ReqwestUsgsM2mEndpoint::new_for_loopback(login.origin.clone(), Duration::from_secs(2))
                .unwrap();
        let api_key = client.login_token(&login_material(), &|| false).unwrap();
        let request = String::from_utf8(finish(login)).unwrap();
        assert!(request.starts_with("POST /api/api/json/stable/login-token HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("content-type: application/json"));
        assert!(!request.to_ascii_lowercase().contains("x-auth-token:"));
        assert!(request.contains(r#""username":"fixture-ers-user""#));
        assert!(request.contains(r#""token":"fixture-application-token""#));

        let permissions = serve_once(response(
            "200 OK",
            "application/json",
            &success_envelope(r#"["user","download","order"]"#),
        ));
        let client = ReqwestUsgsM2mEndpoint::new_for_loopback(
            permissions.origin.clone(),
            Duration::from_secs(2),
        )
        .unwrap();
        let permission_data = client.permissions(&api_key, &|| false).unwrap();
        assert_eq!(
            permission_data.value(),
            &json!(["user", "download", "order"])
        );
        let request = String::from_utf8(finish(permissions)).unwrap();
        assert!(request.starts_with("POST /api/api/json/stable/permissions HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-auth-token: fixture-runtime-api-key\r\n"));
        assert!(request.ends_with("\r\n\r\nnull"));

        let logout = serve_once(response(
            "200 OK",
            "application/json",
            &success_envelope("null"),
        ));
        let client =
            ReqwestUsgsM2mEndpoint::new_for_loopback(logout.origin.clone(), Duration::from_secs(2))
                .unwrap();
        client.logout(&api_key, &|| false).unwrap();
        let request = String::from_utf8(finish(logout)).unwrap();
        assert!(request.starts_with("POST /api/api/json/stable/logout HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-auth-token: fixture-runtime-api-key\r\n"));
        assert!(!request.contains("fixture-application-token"));
        assert!(request.ends_with("\r\n\r\nnull"));
    }

    #[test]
    fn loopback_rejects_http_envelope_content_type_size_and_json_failures() {
        let cases = [
            (
                response(
                    "401 Unauthorized",
                    "application/json",
                    &success_envelope("null"),
                ),
                UsgsM2mErrorCode::AuthInvalid,
            ),
            (
                response("200 OK", "text/plain", &success_envelope("null")),
                UsgsM2mErrorCode::ResponseInvalid,
            ),
            (
                response("200 OK", "application/json", b"{not-json"),
                UsgsM2mErrorCode::ResponseInvalid,
            ),
            (
                response(
                    "200 OK",
                    "application/json",
                    br#"{"requestId":1,"version":"stable","data":7,"errorCode":null,"errorMessage":null}"#,
                ),
                UsgsM2mErrorCode::ResponseInvalid,
            ),
            (
                response(
                    "200 OK",
                    "application/json",
                    br#"{"requestId":1,"version":"stable","data":null,"errorCode":"AUTH_INVALID","errorMessage":"do not expose"}"#,
                ),
                UsgsM2mErrorCode::AuthInvalid,
            ),
        ];

        for (wire_response, expected) in cases {
            let exchange = serve_once(wire_response);
            let client = ReqwestUsgsM2mEndpoint::new_for_loopback(
                exchange.origin.clone(),
                Duration::from_secs(2),
            )
            .unwrap();
            let error = client
                .login_token(&login_material(), &|| false)
                .unwrap_err();
            assert_eq!(error.code(), expected);
            let _ = finish(exchange);
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let origin = Url::parse(&format!("http://{address}/api/api/json/stable/")).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 65537\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let client =
            ReqwestUsgsM2mEndpoint::new_for_loopback(origin, Duration::from_secs(2)).unwrap();
        assert_eq!(
            client
                .login_token(&login_material(), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::ResponseTooLarge
        );
        server.join().unwrap();
    }

    #[test]
    fn production_login_rejection_shape_maps_to_a_distinct_auth_code() {
        // Recorded 2026-10-31 from the public login-token endpoint with
        // deliberately invalid credentials: HTTP 200, application/json, and
        // {"requestId":2386390790,"version":"stable","data":null,
        //  "errorCode":"AUTH_INVALID","errorMessage":"User credential
        //  verification failed"}. The envelope (not the HTTP status) carries
        // the rejection, so a transport/proxy/rate-limit failure must never
        // collapse into USGS_M2M_AUTH_INVALID.
        let body = br#"{"requestId":2386390790,"version":"stable","data":null,"errorCode":"AUTH_INVALID","errorMessage":"User credential verification failed"}"#;
        let exchange = serve_once(response("200 OK", "application/json; charset=utf-8", body));
        let client = ReqwestUsgsM2mEndpoint::new_for_loopback(
            exchange.origin.clone(),
            Duration::from_secs(2),
        )
        .unwrap();

        let error = client
            .login_token(&login_material(), &|| false)
            .expect_err("AUTH_INVALID envelope must reject the login");
        assert_eq!(error.code(), UsgsM2mErrorCode::AuthInvalid);
        assert_eq!(error.remote_code(), Some("AUTH_INVALID"));
        assert_eq!(
            error.remote_message(),
            Some("User credential verification failed")
        );
        let diagnostic = error.diagnostic_message();
        assert!(diagnostic.contains("upstream_error_code=AUTH_INVALID"));
        assert!(!diagnostic.contains("fixture-application-token"));
        assert!(!diagnostic.contains("fixture-ers-user"));
        let _ = finish(exchange);
    }

    #[test]
    fn loopback_redirect_is_terminal_and_never_reaches_the_target() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_url =
            Url::parse(&format!("http://{}/capture", target.local_addr().unwrap())).unwrap();
        let source = serve_once(redirect_response(&target_url));
        let client =
            ReqwestUsgsM2mEndpoint::new_for_loopback(source.origin.clone(), Duration::from_secs(2))
                .unwrap();
        assert_eq!(
            client
                .login_token(&login_material(), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::RedirectRejected
        );
        let _ = finish(source);
        thread::sleep(Duration::from_millis(20));
        assert!(matches!(
            target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn loopback_response_read_honors_cancellation_without_retry() {
        let exchange = serve_once(response(
            "200 OK",
            "application/json",
            &success_envelope("\"fixture-runtime-api-key\""),
        ));
        let client = ReqwestUsgsM2mEndpoint::new_for_loopback(
            exchange.origin.clone(),
            Duration::from_secs(2),
        )
        .unwrap();
        let checks = AtomicUsize::new(0);
        let error = client
            .login_token(&login_material(), &|| {
                checks.fetch_add(1, Ordering::SeqCst) >= 2
            })
            .unwrap_err();
        assert_eq!(error.code(), UsgsM2mErrorCode::Cancelled);
        let _ = finish(exchange);
    }

    #[test]
    fn loopback_dataset_discovery_has_one_closed_field_and_the_runtime_key_header() {
        let exchange = serve_once(response(
            "200 OK",
            "application/json",
            &success_envelope(
                r#"[{"datasetId":"fixture-id","datasetAlias":"fixture_alias","datasetCategoryName":"Landsat Collection 2 Level-2","collectionName":"Landsat 8-9 OLI/TIRS C2 L2","collectionLongName":"Fixture Landsat 8-9 Collection 2 Level-2","dataOwner":"LSAA","catalogs":["EE"]}]"#,
            ),
        ));
        let client = ReqwestUsgsM2mEndpoint::new_for_loopback(
            exchange.origin.clone(),
            Duration::from_secs(2),
        )
        .unwrap();
        let api_key = UsgsM2mApiKey::from_wire("fixture-runtime-api-key".to_owned()).unwrap();
        let mut data = client.dataset_search(&api_key, &|| false).unwrap();
        assert!(matches!(data.take(), Value::Array(records) if records.len() == 1));

        let request = String::from_utf8(finish(exchange)).unwrap();
        assert!(request.starts_with("POST /api/api/json/stable/dataset-search HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-auth-token: fixture-runtime-api-key\r\n"));
        assert!(request.ends_with("\r\n\r\n{\"datasetName\":\"Landsat 8-9\"}"));
        for forbidden in [
            "temporalFilter",
            "spatialFilter",
            "cloudCover",
            "entityId",
            "productId",
            "maxResults",
            "cursor",
            "nextRecord",
        ] {
            assert!(!request.contains(forbidden));
        }
    }

    #[test]
    fn loopback_scene_endpoints_use_exact_paths_bodies_and_runtime_header() {
        let api_key = UsgsM2mApiKey::from_wire("fixture-runtime-api-key".to_owned()).unwrap();
        let filters = serve_once(response(
            "200 OK",
            "application/json",
            &success_envelope("[]"),
        ));
        let client = ReqwestUsgsM2mEndpoint::new_for_loopback(
            filters.origin.clone(),
            Duration::from_secs(2),
        )
        .unwrap();
        let mut data = client
            .dataset_filters(&api_key, "landsat_ot_c2_l2", &|| false)
            .unwrap();
        assert_eq!(data.take(), Value::Array(Vec::new()));
        let request = String::from_utf8(finish(filters)).unwrap();
        assert!(request.starts_with("POST /api/api/json/stable/dataset-filters HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-auth-token: fixture-runtime-api-key\r\n"));
        assert!(request.ends_with("\r\n\r\n{\"datasetName\":\"landsat_ot_c2_l2\"}"));

        let scene = serve_once(response(
            "200 OK",
            "application/json",
            &success_envelope(r#"{"results":[]}"#),
        ));
        let client =
            ReqwestUsgsM2mEndpoint::new_for_loopback(scene.origin.clone(), Duration::from_secs(2))
                .unwrap();
        let payload = json!({
            "datasetName": "landsat_ot_c2_l2",
            "maxResults": 25,
            "metadataType": "full",
            "startingNumber": 1,
            "sceneFilter": {
                "spatialFilter": {
                    "filterType": "geojson",
                    "geoJson": {
                        "type": "Polygon",
                        "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
                    }
                }
            }
        });
        let mut data = client.scene_search(&api_key, &payload, &|| false).unwrap();
        assert!(data.take().is_object());
        let request = finish(scene);
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
            .unwrap();
        let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
        assert!(headers.starts_with("POST /api/api/json/stable/scene-search HTTP/1.1\r\n"));
        assert!(headers
            .to_ascii_lowercase()
            .contains("x-auth-token: fixture-runtime-api-key\r\n"));
        assert_eq!(
            serde_json::from_slice::<Value>(&request[header_end..]).unwrap(),
            payload
        );
        assert!(!headers.to_ascii_lowercase().contains("authorization:"));
        assert!(!headers.to_ascii_lowercase().contains("cookie:"));
    }

    #[test]
    fn loopback_scene_endpoints_keep_redirect_size_and_cancellation_closed() {
        let api_key = UsgsM2mApiKey::from_wire("fixture-runtime-api-key".to_owned()).unwrap();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_url =
            Url::parse(&format!("http://{}/capture", target.local_addr().unwrap())).unwrap();
        let source = serve_once(redirect_response(&target_url));
        let client =
            ReqwestUsgsM2mEndpoint::new_for_loopback(source.origin.clone(), Duration::from_secs(2))
                .unwrap();
        assert_eq!(
            client
                .dataset_filters(&api_key, "landsat_ot_c2_l2", &|| false)
                .err()
                .expect("redirect is rejected")
                .code(),
            UsgsM2mErrorCode::RedirectRejected
        );
        let _ = finish(source);
        thread::sleep(Duration::from_millis(20));
        assert!(matches!(
            target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let origin = Url::parse(&format!("http://{address}/api/api/json/stable/")).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16777217\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let client =
            ReqwestUsgsM2mEndpoint::new_for_loopback(origin, Duration::from_secs(2)).unwrap();
        assert_eq!(
            client
                .scene_search(&api_key, &json!({}), &|| false)
                .err()
                .expect("oversized scene response is rejected")
                .code(),
            UsgsM2mErrorCode::ResponseTooLarge
        );
        server.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = Url::parse(&format!(
            "http://{}/api/api/json/stable/",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let client =
            ReqwestUsgsM2mEndpoint::new_for_loopback(origin, Duration::from_secs(2)).unwrap();
        assert_eq!(
            client
                .scene_search(&api_key, &json!({}), &|| true)
                .err()
                .expect("pre-request cancellation is terminal")
                .code(),
            UsgsM2mErrorCode::Cancelled
        );
    }
}
