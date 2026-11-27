//! MCP boundary; explicit, reviewed servers only; no credential or Store ownership.
use std::{future::Future, pin::Pin, time::Duration};

pub mod replay;
pub mod rmcp_client;

use insar_agent_model::{privacy, ErrorKind, ModelError};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub trait McpClient: Sync {
    fn list_tools<'a>(
        &'a self,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RemoteTool>, ModelError>> + Send + 'a>>;
    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Value,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>>;
    fn shutdown<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), ModelError>> + Send + 'a>>;
}

/// Only tools already registered by the owning catalog can become visible or callable.
pub struct ApprovedTools<C> {
    client: C,
    registered: Vec<String>,
    timeout: Duration,
}

impl<C: McpClient> ApprovedTools<C> {
    pub fn new(client: C, registered: Vec<String>, timeout: Duration) -> Self {
        Self {
            client,
            registered,
            timeout,
        }
    }

    pub async fn list_tools(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Vec<RemoteTool>, ModelError> {
        let tools = self
            .guarded(cancel, || self.client.list_tools(cancel))
            .await?;
        let mut approved = Vec::new();
        for tool in tools {
            if !self.registered.contains(&tool.name) {
                continue;
            }
            if !safe_boundary_text(&tool.description)
                || !safe_boundary_value(&tool.input_schema, false, true, false)
                || !safe_boundary_value(
                    &Value::String(tool.description.clone()),
                    false,
                    true,
                    false,
                )
            {
                return Err(ModelError::new(ErrorKind::Capability, None));
            }
            approved.push(tool);
        }
        Ok(approved)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: &Value,
        cancel: &CancellationToken,
    ) -> Result<Value, ModelError> {
        if !self.registered.iter().any(|registered| registered == name) {
            return Err(ModelError::new(ErrorKind::Capability, None));
        }
        if cancel.is_cancelled() {
            return Err(ModelError::new(ErrorKind::Cancelled, None));
        }
        if !safe_boundary_value(arguments, true, true, false) {
            return Err(ModelError::new(ErrorKind::Parameter, None));
        }
        let result = self
            .guarded(cancel, || self.client.call_tool(name, arguments, cancel))
            .await?;
        // A server cannot make the boundary parse and scrub an unbounded reply;
        // only the first 16 KiB would reach the model anyway.
        const MAX_RAW_RESULT_BYTES: usize = 1024 * 1024;
        if serde_json::to_vec(&result).map_or(true, |raw| raw.len() > MAX_RAW_RESULT_BYTES) {
            return Err(ModelError::new(ErrorKind::Capability, None));
        }
        const MAX_RESULT_CHARS: usize = 16_384;
        let scrubbed = if safe_boundary_value(&result, false, false, false) {
            result
        } else {
            privacy::scrub(&result)
        };
        let json = scrubbed.to_string();
        if !safe_boundary_value(&scrubbed, false, false, true) {
            return Err(ModelError::new(ErrorKind::Capability, None));
        }
        if json.len() > MAX_RESULT_CHARS {
            return Ok(Value::String(truncate_serialized_result(
                &json,
                MAX_RESULT_CHARS,
            )));
        }
        Ok(scrubbed)
    }

    pub async fn shutdown(&self) -> Result<(), ModelError> {
        tokio::time::timeout(self.timeout, self.client.shutdown())
            .await
            .map_err(|_| ModelError::new(ErrorKind::Transient, None))?
    }

    async fn guarded<T, F, Fut>(&self, cancel: &CancellationToken, work: F) -> Result<T, ModelError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ModelError>>,
    {
        if cancel.is_cancelled() {
            return Err(ModelError::new(ErrorKind::Cancelled, None));
        }
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(ModelError::new(ErrorKind::Cancelled, None)),
            result = tokio::time::timeout(self.timeout, async { work().await }) => result.map_err(|_| ModelError::new(ErrorKind::Transient, None))?,
        }
    }
}

fn safe_boundary_value(
    value: &Value,
    arguments: bool,
    limited: bool,
    allow_redacted: bool,
) -> bool {
    safe_boundary_value_at(value, arguments, limited, allow_redacted, 0)
}

fn safe_boundary_value_at(
    value: &Value,
    arguments: bool,
    limited: bool,
    allow_redacted: bool,
    initial_depth: usize,
) -> bool {
    const MAX_BYTES: usize = 65_536;
    const MAX_DEPTH: usize = 32;
    const MAX_NODES: usize = 4_096;

    if limited && serde_json::to_vec(value).map_or(true, |serialized| serialized.len() > MAX_BYTES)
    {
        return false;
    }
    if arguments && !value.is_object() {
        return false;
    }
    let mut pending = vec![(value, initial_depth)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((current, depth)) = pending.pop() {
        nodes += 1;
        if nodes > MAX_NODES || depth > MAX_DEPTH {
            return false;
        }
        match current {
            Value::String(text) => {
                bytes = bytes.saturating_add(text.len());
                if bytes > MAX_BYTES && limited {
                    return false;
                }
                if let Ok(nested) = serde_json::from_str::<Value>(text) {
                    if nested.is_object() || nested.is_array() {
                        if !safe_boundary_value_at(
                            &nested,
                            false,
                            limited,
                            allow_redacted,
                            depth + 1,
                        ) {
                            return false;
                        }
                        continue;
                    }
                }
                if !(if limited {
                    safe_boundary_text(text)
                } else {
                    safe_boundary_text_content(text)
                }) {
                    return false;
                }
            }
            Value::Array(items) => {
                pending.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Value::Object(properties) => {
                for (key, entry) in properties {
                    bytes = bytes.saturating_add(key.len());
                    let redacted = allow_redacted
                        && matches!(entry, Value::String(text) if text == "[redacted]" || text == "[local-reference]");
                    let sensitive = sensitive_boundary_key(key) && !is_pagination_cursor(key);
                    // A sensitive field must have a plain name: a placeholder value
                    // never excuses a key that itself carries a secret or a path.
                    if sensitive && !plain_key_name(key)
                        || !(safe_boundary_text_content(key) || redacted && sensitive)
                    {
                        return false;
                    }
                    if is_token_budget(key) {
                        if arguments && entry.as_u64().is_none() {
                            return false;
                        }
                    } else if sensitive && !redacted {
                        return false;
                    }
                    if !redacted {
                        pending.push((entry, depth + 1));
                    }
                }
            }
            _ => {}
        }
        if limited && bytes > MAX_BYTES {
            return false;
        }
    }
    !limited || serde_json::to_vec(value).is_ok_and(|serialized| serialized.len() <= MAX_BYTES)
}

fn is_token_budget(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "max_tokens"
            | "max_output_tokens"
            | "max_completion_tokens"
            | "prompt_tokens"
            | "completion_tokens"
            | "input_tokens"
            | "output_tokens"
            | "total_tokens"
            | "tokens_used"
    )
}

fn sensitive_boundary_key(key: &str) -> bool {
    // "tokenizer" alone is not a credential, but it must not hide another
    // "token" in the same name (tokenizer_auth_token).
    let lower = fold_for_markers(key)
        .to_ascii_lowercase()
        .replace("tokenizer", "");
    [
        "password",
        "passwd",
        "secret",
        "authorization",
        "cookie",
        "credential",
        "密码",
        "口令",
        "密钥",
        "令牌",
        "api_key",
        "apikey",
        "api-key",
        "access_key",
        "netrc",
    ]
    .iter()
    .any(|part| lower.contains(part))
        || lower.contains("token")
}

/// A field name such as `api_key` or `access_token`: short, identifier
/// characters only, and no segment shaped like a key or token value.
fn plain_key_name(key: &str) -> bool {
    key.chars().count() <= 64
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !contains_secret_shape(key)
        && key.split(['_', '-', '.']).all(|segment| {
            segment.len() < 16
                || !(segment.chars().any(|c| c.is_ascii_digit())
                    && segment.chars().any(|c| c.is_ascii_alphabetic()))
        })
}

fn safe_boundary_text(text: &str) -> bool {
    text.len() <= 16_384 && safe_boundary_text_content(text)
}

fn safe_boundary_text_content(text: &str) -> bool {
    let folded = fold_for_markers(text);
    // URLs are judged as a browser would read the raw text (an encoded "/"
    // in the host is not a separator), and again after folding.
    urls_safe(text) && urls_safe(&folded) && safe_folded_text(&folded)
}

fn urls_safe(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // Browsers repair "https:/host" and "https:\host" into a full URL.
    for scheme in ["http:", "https:"] {
        if lower
            .match_indices(scheme)
            .any(|(at, _)| !lower[at + scheme.len()..].starts_with("//"))
        {
            return false;
        }
    }
    text.match_indices("://").all(|(at, _)| {
        let scheme = text[..at]
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
            .next()
            .unwrap_or("");
        let address = &text[at + 3..];
        let address = &address[..address.find(char::is_whitespace).unwrap_or(address.len())];
        let authority = address.split(['/', '?', '#']).next().unwrap_or("");
        if authority.contains(['@', '%', '\\']) {
            return false;
        }
        // A query string can carry a credential, so only reviewed public
        // catalogs may keep one, and only without credential parameters.
        address
            .split_once('?')
            .is_none_or(|(_, query)| public_catalog_query(scheme, authority, query))
    })
}

/// Full-width ASCII and percent-encoded text read as their plain forms, so
/// `Ｂｅａｒｅｒ`, `C%3A%5CUsers` or `%26token%3D` meet the same checks as
/// `Bearer`, `C:\Users` or `&token=`.
fn fold_for_markers(text: &str) -> String {
    let folded = text
        .chars()
        .map(|c| match c {
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
            '\u{3000}' => ' ',
            _ => c,
        })
        .collect::<String>();
    // Twice, so a doubly encoded label (%2526token%253D) is read too.
    percent_decoded(&percent_decoded(&folded))
}

fn safe_folded_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if [
        "authorization",
        "password",
        "passwd",
        "bearer",
        "cookie",
        "api_key",
        "api-key",
        "access_key",
        "credential",
        "密码",
        "口令",
        "密钥",
        "令牌",
        "secret=",
        "secret:",
        ".netrc",
        "private key",
        "file://",
        "x-amz-",
        "x-goog-",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || labelled_token(&lower)
        || lower.contains("\\\\")
        || lower.contains("/home/")
        || lower.contains("/users/")
        || lower.contains("/var/")
    {
        return false;
    }
    let ascii = lower.as_bytes();
    if ascii.windows(3).enumerate().any(|(at, part)| {
        part[0].is_ascii_alphabetic()
            && part[1] == b':'
            && matches!(part[2], b'/' | b'\\')
            && (at == 0 || !ascii[at - 1].is_ascii_alphanumeric())
    }) {
        return false;
    }
    for word in text.split_whitespace() {
        let word = word.trim_start_matches(['"', '\'', '(', '[', '{']);
        if (word.starts_with('/') && word.len() > 1) || word.starts_with('\\') {
            return false;
        }
    }
    !contains_secret_shape(text)
}

/// Public, anonymous search endpoints of the data sources EOStage reads.
/// Their search URLs may keep a query string (Owner 2026-10-30).
const PUBLIC_CATALOG_HOSTS: &[&str] = &[
    "catalogue.dataspace.copernicus.eu",
    "stac.dataspace.copernicus.eu",
    "cmr.earthdata.nasa.gov",
    "api.daac.asf.alaska.edu",
    "search.asf.alaska.edu",
    "earth-search.aws.element84.com",
    "planetarycomputer.microsoft.com",
];

fn public_catalog_query(scheme: &str, authority: &str, query: &str) -> bool {
    scheme.eq_ignore_ascii_case("https")
        && PUBLIC_CATALOG_HOSTS
            .iter()
            .any(|host| host.eq_ignore_ascii_case(authority))
        // No second address or account hidden inside the allowed query.
        && !query.contains("://")
        && !query.contains(['@', '\\'])
        && query.split(['&', ';', '#']).all(|pair| {
            let name = percent_decoded(pair.split('=').next().unwrap_or("")).to_ascii_lowercase();
            let cursor = is_pagination_cursor(name.trim_start_matches('$'));
            cursor
                || !(sensitive_boundary_key(&name)
                    || name.ends_with("key")
                    || name == "code"
                    || ["sig", "auth", "credential", "session", "pass", "jwt", "%"]
                        .iter()
                        .any(|part| name.contains(part)))
        })
}

/// Text as a server reads it after one round of URL decoding (t%6Fken is token).
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escape = (bytes[at] == b'%')
            .then(|| text.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escape {
            Some(byte) => {
                decoded.push(byte);
                at += 3;
            }
            None => {
                decoded.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// `token=` or `token:` labels a credential unless the whole label is a page
/// cursor such as `$skiptoken=` or `next_page_token:`.
fn labelled_token(lower: &str) -> bool {
    ["token=", "token:"].iter().any(|marker| {
        lower.match_indices(marker).any(|(at, _)| {
            let label_start = lower[..at]
                .char_indices()
                .rev()
                .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$')))
                .map_or(0, |(index, c)| index + c.len_utf8());
            let label = format!("{}token", &lower[label_start..at]);
            !is_pagination_cursor(label.trim_start_matches('$'))
        })
    })
}

/// Opaque "next page" cursors. Named like tokens, but they only page through
/// the server's own results; their values are still checked as text.
fn is_pagination_cursor(key: &str) -> bool {
    let name = key.to_ascii_lowercase().replace(['_', '-'], "");
    matches!(
        name.as_str(),
        "nextpagetoken"
            | "pagetoken"
            | "nexttoken"
            | "continuationtoken"
            | "pagingtoken"
            | "skiptoken"
    )
}

/// Known key and token prefixes followed by a value-like run, and JWTs.
fn contains_secret_shape(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let token_prefixes = [
        "tok_",
        "sk-",
        "sk_",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "glpat-",
        "npm_",
        "pypi-",
        "ya29.",
    ];
    token_prefixes.iter().any(|prefix| {
        lower.match_indices(prefix).any(|(at, _)| {
            lower[at + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                .count()
                >= 8
        })
    }) || text.match_indices("eyJ").any(|(at, _)| {
        let run = text[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            .collect::<String>();
        run.len() >= 20 && run.contains('.')
    })
}

fn truncate_serialized_result(text: &str, max_bytes: usize) -> String {
    const SUFFIX: &str = "[truncated]";
    let mut boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    boundaries.push(text.len());
    let mut low = 0;
    let mut high = boundaries.len() - 1;
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let candidate = format!("{}{}", &text[..boundaries[middle]], SUFFIX);
        if Value::String(candidate).to_string().len() <= max_bytes {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    format!("{}{}", &text[..boundaries[low]], SUFFIX)
}
