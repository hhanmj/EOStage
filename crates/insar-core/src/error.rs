use std::fmt;

use serde_json::{Map, Value};

use crate::ContractError;

pub const SENSITIVE_CONTEXT_KEYS: &[&str] =
    &["password", "token", "cookie", "authorization", "secret"];

const MAX_EXTERNAL_ERROR_BYTES: usize = 4096;
const SECRET_ASSIGNMENT_MARKERS: &[&str] = &[
    "token=",
    "token:",
    "api_key=",
    "apikey=",
    "password=",
    "password:",
    "secret=",
    "secret:",
    "signature=",
    "sig=",
    "authorization=",
    "authorization:",
    "cookie=",
    "cookie:",
    "x-auth-token=",
    "x-auth-token:",
];

/// Preserve external diagnostic text while removing credential-bearing material.
pub fn sanitize_external_error_message(value: &str) -> (String, bool) {
    let mut redacted = false;
    let bounded = if value.len() > MAX_EXTERNAL_ERROR_BYTES {
        redacted = true;
        let mut end = MAX_EXTERNAL_ERROR_BYTES;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        &value[..end]
    } else {
        value
    };
    let mut parts = Vec::new();
    for token in bounded.split_whitespace() {
        let mut safe = token.to_owned();
        let trimmed = token.trim_matches(|character: char| {
            matches!(
                character,
                '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | ',' | ';'
            )
        });
        if (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
            && url::Url::parse(trimmed).is_ok()
        {
            if let Ok(mut parsed) = url::Url::parse(trimmed) {
                let _ = parsed.set_username("");
                let _ = parsed.set_password(None);
                if parsed.query().is_some() {
                    parsed.set_query(Some("REDACTED"));
                    redacted = true;
                }
                if parsed.fragment().is_some() {
                    parsed.set_fragment(None);
                    redacted = true;
                }
                safe = token.replacen(trimmed, parsed.as_str(), 1);
            }
        }
        let lowered = safe.to_ascii_lowercase();
        for marker in SECRET_ASSIGNMENT_MARKERS {
            if let Some(index) = lowered.find(marker) {
                let value_start = index + marker.len();
                if value_start < safe.len() {
                    let mut value_end = safe.len();
                    for (offset, character) in safe[value_start..].char_indices() {
                        if matches!(character, '&' | ',' | ';' | ')' | ']' | '}') {
                            value_end = value_start + offset;
                            break;
                        }
                    }
                    safe.replace_range(value_start..value_end, "[REDACTED]");
                    redacted = true;
                }
                break;
            }
        }
        parts.push(safe);
    }
    (parts.join(" "), redacted)
}

fn validate_debug_value(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(context) => validate_debug_context(context),
        Value::Array(items) => {
            for item in items {
                validate_debug_value(item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub fn validate_debug_context(context: &Map<String, Value>) -> Result<(), String> {
    for (key, value) in context {
        if SENSITIVE_CONTEXT_KEYS
            .iter()
            .any(|candidate| key.eq_ignore_ascii_case(candidate))
        {
            return Err(format!("sensitive debug-context key is forbidden: {key}"));
        }
        validate_debug_value(value)?;
    }
    Ok(())
}

impl ContractError {
    pub fn new(
        code: impl Into<String>,
        user_message: impl Into<String>,
        debug_context: Map<String, Value>,
    ) -> Result<Self, String> {
        validate_debug_context(&debug_context)?;
        let error = Self {
            code: code.into(),
            user_message: user_message.into(),
            debug_context,
        };
        error.validate()?;
        Ok(error)
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[{}] {}", self.code, self.user_message)
    }
}

impl std::error::Error for ContractError {}

#[cfg(test)]
mod external_error_sanitization_tests {
    use super::sanitize_external_error_message;

    #[test]
    fn preserves_diagnostic_text_but_redacts_signed_urls_and_tokens() {
        let (safe, redacted) = sanitize_external_error_message(
            "download failed https://example.test/path?signature=secret token=abc code=403",
        );
        assert!(redacted);
        assert!(safe.contains("download failed"));
        assert!(safe.contains("example.test/path?REDACTED"));
        assert!(safe.contains("token=[REDACTED]"));
        assert!(safe.contains("code=403"));
        assert!(!safe.contains("secret"));
        assert!(!safe.contains("token=abc"));
    }
}
