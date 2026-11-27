//! Conservative outbound transform; redaction is a safety net, not permission to send files.
use serde_json::Value;

const MAX_STRING: usize = 131_072;
/// Upper bound for a single validated image attachment encoded as base64 (~6 MiB binary).
pub const MAX_IMAGE_BASE64_BYTES: usize = 8 * 1024 * 1024;

fn canonical_image_mime(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        _ => None,
    }
}

fn b64_val(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn decode_b64_prefix(clean: &str, max_bytes: usize) -> Option<Vec<u8>> {
    let bytes = clean.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(max_bytes);
    for chunk in bytes.chunks_exact(4) {
        let a = b64_val(chunk[0])?;
        let b = b64_val(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if out.len() >= max_bytes {
            break;
        }
        if chunk[2] != b'=' {
            let c = b64_val(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if out.len() >= max_bytes {
                break;
            }
            if chunk[3] != b'=' {
                let d = b64_val(chunk[3])?;
                out.push((c << 6) | d);
                if out.len() >= max_bytes {
                    break;
                }
            }
        }
    }
    Some(out)
}

fn matches_image_magic(mime: &str, header: &[u8]) -> bool {
    match mime {
        "image/png" => header.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "image/jpeg" => header.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/webp" => header.len() >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"WEBP",
        "image/gif" => header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a"),
        _ => false,
    }
}

/// Validates and normalizes an image payload (either raw base64 or a
/// `data:<mime>;base64,<b64>` URL). Rejects non-image MIME types, malformed
/// base64, oversized payloads, and payloads whose decoded bytes do not begin
/// with a genuine PNG, JPEG, WebP, or GIF file signature.
pub fn normalize_image_attachment(mime_type: &str, raw_data: &str) -> Option<(String, String)> {
    let trimmed = raw_data.trim();
    let (resolved_mime, b64_part) = if let Some(rest) = trimmed.strip_prefix("data:") {
        let (header, data) = rest.split_once(',')?;
        let url_mime = header.strip_suffix(";base64")?.trim();
        let canonical_url = canonical_image_mime(url_mime)?;
        if !mime_type.trim().is_empty() {
            let canonical_arg = canonical_image_mime(mime_type)?;
            if canonical_arg != canonical_url {
                return None;
            }
        }
        (canonical_url, data)
    } else {
        (canonical_image_mime(mime_type)?, trimmed)
    };

    let mut clean = String::with_capacity(b64_part.len().min(MAX_IMAGE_BASE64_BYTES));
    for byte in b64_part.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if clean.len() >= MAX_IMAGE_BASE64_BYTES {
            return None;
        }
        clean.push(byte as char);
    }
    if clean.len() < 16 || clean.len() % 4 == 1 {
        return None;
    }
    while !clean.len().is_multiple_of(4) {
        clean.push('=');
    }
    let bytes = clean.as_bytes();
    let pad_start = bytes.iter().position(|&b| b == b'=').unwrap_or(bytes.len());
    if bytes.len() - pad_start > 2 {
        return None;
    }
    if !bytes[..pad_start].iter().all(|&b| b64_val(b).is_some())
        || !bytes[pad_start..].iter().all(|&b| b == b'=')
    {
        return None;
    }
    let header = decode_b64_prefix(&clean, 16)?;
    if !matches_image_magic(resolved_mime, &header) {
        return None;
    }
    Some((resolved_mime.to_owned(), clean))
}

/// Parses and validates a `data:image/...;base64,...` URL.
pub fn parse_image_data_url(url: &str) -> Option<(String, String)> {
    if !url.trim().starts_with("data:") {
        return None;
    }
    normalize_image_attachment("", url)
}

/// Returns true iff `value` matches one of the four strictly-shaped protocol
/// multimodal image blocks with a verified PNG/JPEG/WebP/GIF base64 payload.
pub fn is_safe_multimodal_image_block(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    let valid_detail = |v: &Value| {
        v.as_str()
            .is_some_and(|s| matches!(s, "auto" | "low" | "high"))
    };
    // 1. OpenAI Chat Completions: {"type":"image_url","image_url":{"url":"data:..."}}
    if obj.get("type").and_then(Value::as_str) == Some("image_url") && obj.len() == 2 {
        if let Some(inner) = obj.get("image_url").and_then(Value::as_object) {
            let keys_ok = inner
                .keys()
                .all(|k| k == "url" || (k == "detail" && inner.get(k).is_some_and(valid_detail)));
            if keys_ok {
                return inner
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(parse_image_data_url)
                    .is_some();
            }
        }
        return false;
    }
    // 2. OpenAI Responses: {"type":"input_image","image_url":"data:..."}
    if obj.get("type").and_then(Value::as_str) == Some("input_image")
        && (obj.len() == 2 || (obj.len() == 3 && obj.get("detail").is_some_and(valid_detail)))
    {
        return obj
            .get("image_url")
            .and_then(Value::as_str)
            .and_then(parse_image_data_url)
            .is_some();
    }
    // 3. Anthropic Messages: {"type":"image","source":{"type":"base64","media_type":"...","data":"..."}, ["cache_control":{"type":"ephemeral"}]}
    let valid_cache_control = |val: &Value| -> bool {
        val.as_object().is_some_and(|cc| {
            cc.len() == 1 && cc.get("type").and_then(Value::as_str) == Some("ephemeral")
        })
    };
    if obj.get("type").and_then(Value::as_str) == Some("image")
        && (obj.len() == 2
            || (obj.len() == 3 && obj.get("cache_control").is_some_and(valid_cache_control)))
    {
        if let Some(src) = obj.get("source").and_then(Value::as_object) {
            if src.len() == 3 && src.get("type").and_then(Value::as_str) == Some("base64") {
                if let (Some(mime), Some(data)) = (
                    src.get("media_type").and_then(Value::as_str),
                    src.get("data").and_then(Value::as_str),
                ) {
                    return normalize_image_attachment(mime, data).is_some();
                }
            }
        }
        return false;
    }
    // 4. Google Gemini: {"inline_data":{"mime_type":"...","data":"..."}}
    if obj.len() == 1 {
        if let Some(inline) = obj.get("inline_data").and_then(Value::as_object) {
            if inline.len() == 2 {
                if let (Some(mime), Some(data)) = (
                    inline.get("mime_type").and_then(Value::as_str),
                    inline.get("data").and_then(Value::as_str),
                ) {
                    return normalize_image_attachment(mime, data).is_some();
                }
            }
        }
    }
    false
}

pub fn scrub(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(scrub_text(text)),
        Value::Array(items) => Value::Array(items.iter().map(scrub).collect()),
        Value::Object(properties) => {
            if is_safe_multimodal_image_block(value) {
                return value.clone();
            }
            Value::Object(
                properties
                    .iter()
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            if sensitive_key(key) {
                                Value::String("[redacted]".into())
                            } else {
                                scrub(value)
                            },
                        )
                    })
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// Exact lowercased token-budget keys that are protocol parameters rather than credentials.
pub fn is_allowed_token_budget_key(lower: &str) -> bool {
    matches!(
        lower,
        "max_tokens"
            | "budget_tokens"
            | "max_output_tokens"
            | "maxoutputtokens"
            | "max_completion_tokens"
    )
}

fn sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    if is_allowed_token_budget_key(&lower) {
        return false;
    }
    [
        "password",
        "token",
        "secret",
        "authorization",
        "cookie",
        "api_key",
        "access_key",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

pub fn scrub_text(text: &str) -> String {
    if let Ok(nested) = serde_json::from_str::<Value>(text) {
        if nested.is_object() || nested.is_array() {
            let scrubbed = scrub(&nested);
            if scrubbed == nested {
                return text.to_owned();
            }
            return scrubbed.to_string();
        }
    }
    let mut output = String::new();
    let mut token = String::new();
    for char in text.chars().take(MAX_STRING) {
        if char.is_whitespace() {
            output.push_str(&scrub_word(&token));
            token.clear();
            output.push(char);
        } else {
            token.push(char);
        }
    }
    output.push_str(&scrub_word(&token));
    for marker in ["Bearer ", "bearer ", "api_key=", "token=", "password="] {
        let mut from = 0;
        while let Some(relative) = output[from..].find(marker) {
            let start = from + relative + marker.len();
            if output[start..].starts_with("[redacted]") {
                from = start + "[redacted]".len();
                continue;
            }
            let length = output[start..]
                .find(char::is_whitespace)
                .unwrap_or(output.len() - start);
            output.replace_range(start..start + length, "[redacted]");
            from = start + "[redacted]".len();
        }
    }
    output
}

fn scrub_word(word: &str) -> String {
    let candidate = word.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | ',' | ';'));
    let prefix_len = word.find(candidate).unwrap_or(0);
    let prefix = &word[..prefix_len];
    let trailing = &word[prefix_len + candidate.len()..];
    if let Some(start) = candidate.find("://") {
        let scheme = &candidate[..start];
        if matches!(scheme, "https" | "http") {
            let without_query = candidate.split(['?', '#']).next().unwrap_or(candidate);
            // Userinfo (`user:password@`) sits in the authority, before the path.
            let rest = &without_query[start + 3..];
            let authority_len = rest.find('/').unwrap_or(rest.len());
            let host = match rest[..authority_len].rfind('@') {
                Some(at) => &rest[at + 1..],
                None => rest,
            };
            return format!("{prefix}{scheme}://{host}{trailing}");
        }
    }
    if candidate.contains(":\\")
        || (candidate.starts_with("\\\\")
            && candidate[2..]
                .trim_start_matches('\\')
                .starts_with(|c: char| c.is_ascii_alphanumeric()))
        || candidate.starts_with("~/")
        || candidate.starts_with("./")
        || candidate.starts_with("../")
        || tilde_home_at(candidate)
        || encoded_path_at(candidate)
        || unix_path_at(candidate)
    {
        return format!("{prefix}[local-reference]{trailing}");
    }
    word.to_owned()
}

/// Whether `word` holds a `/` that begins a Unix path (review 2026-11-02,
/// privacy-first ruling of the second review): its previous character is
/// none of an ASCII letter, digit, `.`, `~`, `:`, `/` or `%` (so `TCP/IP`,
/// `math.pi/2` are not paths), its next character is neither `/` nor
/// whitespace (`//server`, a lone `/`), and the first segment starts with
/// a letter, ASCII or not (`1/2`, `2026/11/02`, `24/7` are not; `/home`,
/// `/data`, `下载到/数据` are — `他/她` is a false positive the
/// privacy-first ruling accepts). The text after `://` is not read (an
/// address keeps its path; only the host is sent).
fn is_unix_root_segment(segment: &str) -> bool {
    let base = segment.split('.').next().unwrap_or(segment);
    matches!(
        base.to_ascii_lowercase().as_str(),
        "users"
            | "home"
            | "tmp"
            | "var"
            | "etc"
            | "opt"
            | "applications"
            | "mnt"
            | "media"
            | "root"
            | "srv"
            | "data"
            | "usr"
            | "bin"
            | "sbin"
            | "dev"
            | "proc"
            | "sys"
            | "run"
            | "private"
            | "library"
            | "system"
            | "volumes"
            | "boot"
            | "lib"
            | "lib64"
            | "snap"
            | "nix"
            | "workspace"
            | "work"
            | "scratch"
            | "help"
    )
}

fn unix_path_after_slash(previous: Option<char>, rest: &str) -> bool {
    let Some(first) = rest.chars().next() else {
        return false;
    };
    if first == '/' || first.is_whitespace() || !first.is_alphabetic() {
        return false;
    }
    if let Some(prev) = previous {
        if prev.is_ascii_alphanumeric()
            || matches!(
                prev,
                '.' | '~'
                    | ':'
                    | '/'
                    | '%'
                    | '\\'
                    | '°'
                    | '℃'
                    | '℉'
                    | '′'
                    | '″'
                    | ')'
                    | ']'
                    | '}'
                    | '>'
                    | '$'
                    | '^'
                    | '_'
                    | '+'
                    | '-'
                    | '*'
                    | '±'
                    | '×'
                    | '÷'
                    | '²'
                    | '³'
            )
        {
            return false;
        }
    }
    let seg_end = rest
        .char_indices()
        .find(|&(_, c)| {
            c.is_whitespace()
                || matches!(
                    c,
                    '/' | '\\'
                        | '\n'
                        | '"'
                        | '\''
                        | '<'
                        | '>'
                        | '|'
                        | ','
                        | ';'
                        | ':'
                        | ')'
                        | ']'
                        | '}'
                        | '。'
                        | '，'
                        | '；'
                        | '：'
                        | '）'
                        | '】'
                        | '”'
                        | '’'
                )
        })
        .map_or(rest.len(), |(idx, _)| idx);
    let first_seg = &rest[..seg_end];
    let has_second_slash = rest[seg_end..].starts_with('/');

    if first.is_ascii_alphabetic() {
        if is_unix_root_segment(first_seg) {
            return true;
        }
        let after_non_path_word = previous
            .is_some_and(|prev| prev.is_alphabetic() && !matches!(prev, '到' | '在' | '至' | '入'));
        !after_non_path_word
            && has_second_slash
            && first_seg.len() >= 2
            && first_seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    } else {
        matches!(previous, Some('到' | '在' | '至' | '入'))
            || (previous == Some('他') && first_seg.starts_with('她'))
            || (has_second_slash
                && matches!(
                    first_seg,
                    "数据" | "下载" | "桌面" | "目录" | "文件" | "用户"
                ))
    }
}

fn unix_path_at(word: &str) -> bool {
    word.char_indices().any(|(index, c)| {
        c == '/' && unix_path_after_slash(word[..index].chars().next_back(), &word[index + 1..])
    })
}

/// Whether `word` holds a URL-encoded separator (`%2F`, case-insensitive)
/// that begins an encoded path (second review P2, third review): the
/// character before the `%` satisfies the Unix path's left boundary, and
/// what follows the code is an ASCII letter or another percent-encoded
/// UTF-8 lead byte `%C2`–`%F4` (`%2Fhome%2Fme%2Fx`,
/// `%2F%E4%B8%8B%E8%BD%BD`; `?a=1%2F2` — a digit after the code — and
/// `%2F%20` — a space — are not).
fn encoded_path_at(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    lower.match_indices("%2f").any(|(index, _)| {
        word[..index].chars().next_back().is_none_or(|previous| {
            !previous.is_ascii_alphanumeric() && !matches!(previous, '.' | '~' | ':' | '/' | '%')
        }) && encoded_lead(&word[index + 3..])
    })
}

/// What may follow a `%2F` separator for it to begin an encoded path
/// (third review): an ASCII letter, or another percent-encoded UTF-8 lead
/// byte `%C2`–`%F4` (case-insensitive).
fn encoded_lead(rest: &str) -> bool {
    match rest.as_bytes().first() {
        Some(b'%') => rest
            .get(1..3)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .is_some_and(|byte| (0xC2..=0xF4).contains(&byte)),
        Some(first) if first.is_ascii_alphabetic() => true,
        _ => false,
    }
}

/// Whether `word` holds another user's home directory: `~` followed by a
/// run of `[A-Za-z0-9_.-]` name characters and then `/` (second review,
/// P3: `~user/x`), with the same left boundary as a Unix path.
fn tilde_home_at(word: &str) -> bool {
    word.match_indices('~').any(|(index, _)| {
        word[..index].chars().next_back().is_none_or(|previous| {
            !previous.is_ascii_alphanumeric() && !matches!(previous, '.' | '~' | ':' | '/' | '%')
        }) && word[index + 1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
            .is_some_and(|offset| word[index + 1 + offset..].starts_with('/'))
    })
}
