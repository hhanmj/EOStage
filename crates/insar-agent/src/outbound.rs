//! What leaves the machine in a model request (A1 design §5), as host code
//! prepares it before the run record's outbound check (L4 r7 P1-4). The
//! check (`run::safe_value` on the whole body) refuses a body rather than
//! scrubbing it; everything the host can make safe is made safe here, so a
//! refusal (`outbound_blocked`) means a body the host could not repair.
//!
//! - Text (user and assistant messages): a word the check would refuse (a
//!   local path, a signed or credential-bearing value) becomes
//!   `[local-reference]`.
//! - Tool results and tool-call arguments travel as JSON inside a string, so
//!   each string value is also checked in its escaped form: a colon right
//!   before a character JSON escapes (`"`, `\`, a line break) reads as a
//!   Windows path there (`:\"`, `:\n`), and a space is put after that colon.
//!   A key the check would refuse is dropped and a value still refused
//!   becomes `[local]`, as `host::sanitize` does for service results.
use insar_agent_model::{privacy, Message};
use serde_json::{json, Value};

/// Marker for a word of text that could not be sent.
pub const TEXT_MARKER: &str = "[local-reference]";
/// Marker for a value of a tool result or argument that could not be sent.
pub const VALUE_MARKER: &str = "[local]";

/// The check passes `value` as it stands: it neither refuses nor rewrites it
/// (it accepts a lone web address only by dropping its query or fragment,
/// and a body is sent only if the check returns it unchanged).
fn passes(value: &Value) -> bool {
    crate::run::safe_value(value).ok().as_ref() == Some(value)
}
fn safe_text(text: &str) -> bool {
    passes(&Value::String(text.to_owned()))
}
/// A word as it may be sent: itself, the check's rewrite of it when that
/// passes in turn, else [`TEXT_MARKER`].
fn sendable_word(word: &str) -> String {
    if safe_text(word) {
        return word.to_owned();
    }
    match crate::run::safe_value(&Value::String(word.to_owned())) {
        Ok(Value::String(rewritten)) if !rewritten.is_empty() && safe_text(&rewritten) => rewritten,
        _ => TEXT_MARKER.to_owned(),
    }
}

/// Text as it may be sent. PEM private key blocks are replaced whole first
/// (`pem_blocks`), then local paths (`path_runs`), then credential headers
/// with their credentials (`rewrite`), then secrets known by their shape
/// with no header (`shapes`, W3n), then the transport's own scrub runs (`privacy::scrub_text`:
/// `Bearer` credentials, URL queries and user info, Windows-path words),
/// because the request body is built from this text and that scrub would
/// find nothing left to act on afterwards; then the words again: what the
/// scrub left as `Bearer [redacted]` goes, and each other word the outbound
/// check would refuse becomes [`TEXT_MARKER`]. A refusal still left (a
/// marker word with no credential after it) takes the marker word and the
/// word after it; text still refused is left out.
pub fn redact_words(text: &str) -> String {
    // Credentials go before the transport scrub, which takes everything up
    // to the next space after `Bearer` (and so the text after the token).
    // A whole message that is JSON is scrubbed by key name first (as the
    // transport would): the passes below would no longer leave valid JSON.
    let json = serde_json::from_str::<Value>(text.trim())
        .ok()
        .filter(|value| value.is_object() || value.is_array())
        .map(|_| privacy::scrub_text(text.trim()));
    let text = json.as_deref().unwrap_or(text);
    // Private key blocks go whole first (they span lines); bare secrets
    // (`shapes`) go after the header pass, which reads a token after
    // `Bearer` / `Authorization:` whatever its shape.
    let headers = rewrite(&path_runs(&pem_blocks(text)), false);
    let scrubbed = privacy::scrub_text(&shapes(&headers));
    let output = rewrite(&scrubbed, true);
    if safe_text(&output) {
        return output;
    }
    let markers = marker_pairs(&output);
    if safe_text(&markers) {
        markers
    } else {
        "[已省略]".to_owned()
    }
}

/// Only the credential headers of `text` replaced (other words as they
/// are), for values that are checked word by word elsewhere.
fn redact_credentials(text: &str) -> String {
    rewrite(text, false)
}

/// A PEM private key block (`-----BEGIN … PRIVATE KEY-----` through
/// `-----END …-----`, or to the end of the text when it has no end line)
/// replaced by one [`TEXT_MARKER`] (W3n rule 4). Other PEM blocks
/// (certificates, public keys) stay.
fn pem_blocks(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    const END: &str = "-----END ";
    const DASHES: &str = "-----";
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(begin) = rest.find(BEGIN) {
        let label = begin + BEGIN.len();
        let Some(label_end) = rest[label..].find(DASHES).map(|k| label + k) else {
            break;
        };
        let after = label_end + DASHES.len();
        if !rest[label..label_end].contains("PRIVATE KEY") {
            output.push_str(&rest[..after]);
            rest = &rest[after..];
            continue;
        }
        output.push_str(&rest[..begin]);
        output.push_str(TEXT_MARKER);
        let block = &rest[after..];
        let end = block
            .find(END)
            .and_then(|k| {
                let label = k + END.len();
                block[label..]
                    .find(DASHES)
                    .map(|d| label + d + DASHES.len())
            })
            .unwrap_or(block.len());
        rest = &block[end..];
    }
    output.push_str(rest);
    output
}

/// Words that make a key name's value a secret when the name contains
/// one (W3n rule 3; redaction guard-2: the same reading as the run record's key
/// check, `run::safe_value`, so `OPENAI_API_KEY`, `refresh_token` and
/// `AWS_SECRET_ACCESS_KEY` count), except `max_tokens`.
const SECRET_KEY_WORDS: [&str; 13] = [
    "token",
    "password",
    "passwd",
    "pwd",
    "secret",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "credential",
    "cookie",
    "authorization",
    "netrc",
];
/// Chinese key names whose following value is a secret (redaction guard-3).
const CJK_SECRET_KEYS: [&str; 4] = ["密码", "令牌", "密钥", "口令"];
/// Headers whose value runs on to the end of the line (W3n rule 3), while
/// its words start with ASCII (redaction guard-1).
const LINE_HEADERS: [&str; 3] = ["x-api-key:", "cookie:", "set-cookie:"];
/// Prefixes of issued tokens (W3n rule 2).
const TOKEN_PREFIXES: [&str; 13] = [
    "sk-",
    "sk_",
    "tok_",
    "ghp_",
    "gho_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "AIza",
    "ya29.",
    "glpat-",
    "npm_",
    "pypi-",
];
/// The transport scrub's markers (`privacy::scrub_text`) that take the text
/// after them up to the next space unless it is `[redacted]` (review P3-5).
const SCRUB_MARKERS: [&str; 3] = ["api_key=", "token=", "password="];

fn secret_key(word: &str) -> bool {
    let name = bare(word);
    !insar_agent_model::privacy::is_allowed_token_budget_key(&name)
        && SECRET_KEY_WORDS.iter().any(|secret| name.contains(secret))
}

/// The `.netrc` keys whose next word is the value with no separator
/// (`machine … login alice password <value>`).
fn netrc_key(word: &str) -> bool {
    matches!(bare(word).as_str(), "password" | "passwd")
}

/// Word `index` is a .netrc `login` / `account` key: the text names a
/// `machine`, or a `password` follows on the same line
/// (`login alice password …`). In prose (`login page`) it is a word.
fn netrc_login(words: &[(String, String)], index: usize, netrc: bool) -> bool {
    if !matches!(bare(&words[index].0).as_str(), "login" | "account") {
        return false;
    }
    netrc || {
        let mut at = index;
        let mut found = false;
        while !words[at].1.contains('\n') && at + 1 < words.len() {
            at += 1;
            if netrc_key(&words[at].0) {
                found = true;
                break;
            }
        }
        found
    }
}

/// Where a key/value separator (`:` `=` `：` `＝`) starts in `word`.
fn separator(word: &str) -> Option<(usize, usize)> {
    word.char_indices()
        .find(|(_, c)| matches!(c, ':' | '=' | '：' | '＝'))
        .map(|(index, c)| (index, index + c.len_utf8()))
}

/// A marker the host or the transport already put in place of something
/// (`[local-reference]`, `[local-reference:message_dir]`, `[local]`,
/// `[redacted]`), after any opening quote: kept as it is (review P3-4).
fn placeholder(text: &str) -> bool {
    let text = text.trim_start_matches(['"', '\'', '`']);
    text.starts_with("[local-reference")
        || text.starts_with(VALUE_MARKER)
        || text.starts_with("[redacted]")
}

/// Where a Chinese key name (`CJK_SECRET_KEYS`) in `word` is followed by
/// its value: the byte after the last key name and any 是 / 为 / `:` / `=`
/// after it, and whether a `:` or `=` was among them.
fn cjk_key(word: &str) -> Option<(usize, bool)> {
    let (position, key) = CJK_SECRET_KEYS
        .iter()
        .filter_map(|key| word.rfind(key).map(|position| (position, *key)))
        .max_by_key(|(position, _)| *position)?;
    let after = position + key.len();
    let mut explicit = false;
    for (offset, c) in word[after..].char_indices() {
        match c {
            '是' | '为' | '為' => {}
            ':' | '：' | '=' | '＝' => explicit = true,
            _ => return Some((after + offset, explicit)),
        }
    }
    Some((word.len(), explicit))
}

/// `word` with the value starting at byte `from` replaced (read as after
/// `Bearer`: `scheme_credential_end`), opening marks kept, or `None` when
/// nothing there is a value (Chinese text, a placeholder, nothing). After a
/// transport scrub marker (`api_key=`) the value becomes `[redacted]`, which
/// that scrub leaves alone, so it does not take the text after the value.
fn replace_value(word: &str, from: usize) -> Option<String> {
    let value = &word[from..];
    if placeholder(value) {
        return None;
    }
    let end = scheme_credential_end(value)?;
    let open = value.len()
        - value
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .len();
    let marker = if SCRUB_MARKERS.iter().any(|m| word[..from].ends_with(m)) {
        "[redacted]"
    } else {
        TEXT_MARKER
    };
    Some(format!(
        "{}{}{marker}{}",
        &word[..from],
        &value[..open],
        &value[end..]
    ))
}

/// A character a bare secret can hold.
fn secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '/' | '=' | '~' | '%')
}

/// Length of a JWT at the start of `run` (W3n rule 1): base64url parts
/// joined by dots, the first starting `eyJ` (a JSON header) and at least 10
/// long. Up to three parts are taken; fewer when the text was cut short (the
/// desktop's 40-character session title cuts a JWT in its second part).
fn jwt_len(run: &str) -> Option<usize> {
    if !run.starts_with("eyJ") {
        return None;
    }
    let url = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-');
    let part = |from: usize| {
        let rest = &run[from..];
        rest.find(|c: char| !url(c)).unwrap_or(rest.len())
    };
    let mut end = part(0);
    if end < 10 {
        return None;
    }
    for _ in 1..3 {
        if !run[end..].starts_with('.') {
            break;
        }
        let next = part(end + 1);
        if next == 0 {
            break;
        }
        end += 1 + next;
    }
    Some(end)
}

/// A product name, file name or catalogue id that a secret shape must not
/// take (W3n rule 5): a Sentinel-1 / NISAR / ALOS name by its prefix, a data
/// file by its extension at the end, a CMR id by its pattern
/// (`G3315903479-ASF`, `C2853145197-ASF`). A guard only: every such name
/// holds `_`, `-` or `.`, which neither the hex nor the base64 rule admits,
/// and none starts with a token prefix, so no current rule reaches one.
fn product_like(run: &str) -> bool {
    let upper = run.to_ascii_uppercase();
    let cmr = |id: &str| {
        let Some(rest) = id.strip_prefix(['G', 'C']) else {
            return false;
        };
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        digits > 0
            && rest[digits..].strip_prefix('-').is_some_and(|provider| {
                !provider.is_empty() && provider.chars().all(|c| c.is_ascii_uppercase())
            })
    };
    ["S1A_", "S1B_", "S1C_", "S1D_", "S1_", "NISAR_", "ALOS"]
        .iter()
        .any(|name| upper.starts_with(name))
        || [
            ".ZIP", ".H5", ".TIF", ".TIFF", ".SAFE", ".NC", ".XML", ".JSON",
        ]
        .iter()
        .any(|extension| upper.ends_with(extension))
        || cmr(run)
}

/// Length of the secret at the start of `run` (a whole run of
/// `secret_char`), when it has a known shape (W3n rules 1, 2 and 5): a JWT;
/// a known token prefix followed by 8+ token characters carrying a digit or
/// mixed case, or `AKIA` and 16 upper-case letters or digits; 32+ hex
/// digits; 40+ base64 characters with a digit and mixed case. Product
/// names are not secrets (`product_like`).
fn secret_shape(run: &str) -> Option<usize> {
    if let Some(end) = jwt_len(run) {
        return Some(end);
    }
    if product_like(run) {
        return None;
    }
    let run = run.trim_end_matches('.');
    let digit = |text: &str| text.chars().any(|c| c.is_ascii_digit());
    let mixed = |text: &str| {
        text.chars().any(|c| c.is_ascii_uppercase()) && text.chars().any(|c| c.is_ascii_lowercase())
    };
    if let Some(rest) = run.strip_prefix("AKIA") {
        if rest.len() >= 16
            && rest[..16]
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Some(run.len());
        }
    }
    let prefixed = TOKEN_PREFIXES.iter().any(|prefix| {
        run.strip_prefix(prefix)
            .is_some_and(|rest| rest.len() >= 8 && (digit(rest) || mixed(rest)))
    });
    let hex = run.len() >= 32 && run.chars().all(|c| c.is_ascii_hexdigit());
    let body = run.trim_end_matches('=');
    let base64 = run.len() >= 40
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/'))
        && digit(body)
        && mixed(body);
    (prefixed || hex || base64).then_some(run.len())
}

/// Where the secret in `run` is: at its start (`secret_shape`), or after
/// its last `=` that is not base64 padding (redaction guard-2: `jwt=<JWT>`,
/// `key=<32 hex>`).
fn secret_span(run: &str) -> Option<(usize, usize)> {
    if let Some(end) = secret_shape(run) {
        return Some((0, end));
    }
    let (equals, _) = run
        .char_indices()
        .rfind(|&(index, c)| c == '=' && run[index + 1..].starts_with(|n: char| n != '='))?;
    let start = equals + 1;
    secret_shape(&run[start..]).map(|end| (start, start + end))
}

/// `word` with each secret-shaped run inside it replaced: the run may sit
/// after Chinese text or an opening mark (`令牌【<jwt>】`, a backquote).
fn shape_word(word: &str) -> String {
    let mut output = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(start) = rest.find(secret_char) {
        let len = rest[start..]
            .find(|c: char| !secret_char(c))
            .unwrap_or(rest.len() - start);
        let run = &rest[start..start + len];
        output.push_str(&rest[..start]);
        match secret_span(run) {
            Some((from, end)) => {
                output.push_str(&run[..from]);
                output.push_str(TEXT_MARKER);
                output.push_str(&run[end..]);
            }
            None => output.push_str(run),
        }
        rest = &rest[start + len..];
    }
    output.push_str(rest);
    output
}

/// What a key name seen on this line still waits for.
#[derive(Clone, Copy)]
enum Pending {
    /// A key name with no separator yet; `netrc` when the next word may be
    /// the value without one (`password MyP4ss123`).
    Key { netrc: bool },
    /// After a separator: the next word is the value if it starts with ASCII.
    Value,
    /// After a Chinese key name with no `:`: the next word is the value if
    /// it looks like a credential (`credential_like`).
    Likely,
    /// After `login` / `account` in a .netrc text: the next word is the
    /// account name (L4 r10 P2-3).
    Login,
}

/// The value of a line header (`X-Api-Key:` / `Cookie:`) whose name is
/// word `index` and whose value starts at byte `from` of it: that part and
/// the following words on the line, while each starts with ASCII, each cut
/// at its first non-ASCII character (redaction guard-1: the user's question after
/// it stays), closing punctuation at the end kept. Returns the rendered
/// text and the last word it consumed, or `None` when there is no value.
fn line_header_value(
    words: &[(String, String)],
    index: usize,
    from: usize,
) -> Option<(String, usize)> {
    let ascii_start =
        |text: &str| text.starts_with(|c: char| c.is_ascii_graphic()) && !placeholder(text);
    let word = &words[index].0;
    let mut output = word[..from].to_owned();
    let mut value = String::new();
    let mut tail = "";
    let mut last = index;
    let first = &word[from..];
    if first.is_empty() {
        let next = words
            .get(index + 1)
            .filter(|_| !words[index].1.contains('\n'))?;
        if !ascii_start(&next.0) {
            return None;
        }
        output.push_str(&words[index].1);
        last = index + 1;
    } else if !ascii_start(first) {
        return None;
    }
    loop {
        let text = if last == index {
            first
        } else {
            words[last].0.as_str()
        };
        let cut = text.find(|c: char| !c.is_ascii()).unwrap_or(text.len());
        value.push_str(&text[..cut]);
        if cut < text.len() {
            tail = &text[cut..];
            break;
        }
        let gap = &words[last].1;
        match words.get(last + 1) {
            Some((next, _)) if !gap.contains('\n') && ascii_start(next) => {
                value.push_str(gap);
                last += 1;
            }
            _ => break,
        }
    }
    let kept = value.trim_end_matches([')', ']', '}', '"', '\'', '`', '>', ',', '.', ';']);
    output.push_str(TEXT_MARKER);
    output.push_str(&value[kept.len()..]);
    output.push_str(tail);
    Some((output, last))
}

/// Recognize netrc credential fields inside a block header, preserving the
/// syntax boundary of quoted and Unicode values. The marker is emitted as one
/// word; the surrounding text remains untouched.
fn netrc_values(words: &mut Vec<(String, String)>) -> Vec<Option<usize>> {
    let mut masked = vec![None; words.len()];
    let mut at = 0;
    while at < words.len() {
        let header = bare(&words[at].0);
        let mut next = at + 1;
        if header == "machine" {
            if next >= words.len() {
                at += 1;
                continue;
            }
            next += 1;
        } else if header != "default" {
            if netrc_key(&words[at].0) {
                if let Some((value, _)) = words.get(next) {
                    if matches!(
                        value.chars().next(),
                        Some('"' | '\'' | '`' | '“' | '‘' | '＂' | '＇')
                    ) {
                        if let Some((last, end)) = netrc_value_span(words, next) {
                            split_netrc_suffix(words, &mut masked, last, end);
                            masked[next] = Some(last);
                            at = last + 1;
                            continue;
                        }
                    }
                }
                at += 1;
                continue;
            }
            if !matches!(header.as_str(), "login" | "account") {
                at += 1;
                continue;
            }
            let last = if words.get(next).is_some_and(|(word, _)| placeholder(word)) {
                next
            } else if let Some((last, end)) = netrc_value_span(words, next) {
                split_netrc_suffix(words, &mut masked, last, end);
                last
            } else {
                at += 1;
                continue;
            };
            let password = last + 1;
            if password + 1 >= words.len() || !netrc_key(&words[password].0) {
                at += 1;
                continue;
            }
            next = at;
        }
        while next < words.len() {
            // A pre-redacted value may appear between fields. It is not a
            // field itself and must remain unchanged; continue scanning the
            // already-recognized block rather than exposing the next value.
            if placeholder(&words[next].0) {
                next += 1;
                continue;
            }
            if next + 1 >= words.len()
                || !matches!(
                    bare(&words[next].0).as_str(),
                    "login" | "account" | "password" | "passwd"
                )
            {
                break;
            }
            let value = next + 1;
            if placeholder(&words[value].0) {
                next = value + 1;
                continue;
            }
            let Some((last, end)) = netrc_value_span(words, value) else {
                break;
            };
            split_netrc_suffix(words, &mut masked, last, end);
            masked[value] = Some(last);
            next = last + 1;
        }
        at = next.max(at + 1);
    }
    masked
}

/// Revisit a closed quote's suffix as ordinary text (and possibly another
/// credential field), without adding whitespace or bypassing the scrubber.
/// All earlier masked spans end before this insertion, so their indices stay
/// valid; reserve the corresponding slot for the newly exposed suffix.
fn split_netrc_suffix(
    words: &mut Vec<(String, String)>,
    masked: &mut Vec<Option<usize>>,
    last: usize,
    end: usize,
) {
    if end < words[last].0.len() {
        let suffix = words[last].0.split_off(end);
        let gap = std::mem::take(&mut words[last].1);
        words.insert(last + 1, (suffix, gap));
        masked.insert(last + 1, None);
    }
}

/// Last token and byte boundary just after the credential, including its
/// closing quote. Unquoted and unterminated values consume their whole token.
fn netrc_value_span(words: &[(String, String)], value: usize) -> Option<(usize, usize)> {
    let first = words.get(value)?.0.as_str();
    if placeholder(first) {
        return None;
    }
    let (opening, closing) = match first.chars().next() {
        Some('"') => ('"', '"'),
        Some('\'') => ('\'', '\''),
        Some('`') => ('`', '`'),
        Some('“') => ('“', '”'),
        Some('‘') => ('‘', '’'),
        Some('＂') => ('＂', '＂'),
        Some('＇') => ('＇', '＇'),
        _ => return Some((value, first.len())),
    };
    let mut escaped = false;
    for (index, (word, _)) in words.iter().enumerate().skip(value) {
        let start = if index == value {
            opening.len_utf8()
        } else {
            0
        };
        for (offset, c) in word[start..].char_indices() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == closing {
                return Some((index, start + offset + c.len_utf8()));
            }
        }
    }
    // Incomplete quoted material is protected through the available text.
    let last = words.len() - 1;
    Some((last, words[last].0.len()))
}

#[cfg(test)]
fn netrc_value_end(words: &[(String, String)], value: usize) -> Option<usize> {
    netrc_value_span(words, value).map(|(last, _)| last)
}

/// Secrets with no `Bearer` / `Authorization` header, known by their shape
/// (W3n, L4 r9 C15 probes 2 and 4), each replaced by [`TEXT_MARKER`] with
/// the text around it kept:
/// - the value after a key name containing a secret word
///   (`SECRET_KEY_WORDS`) and `:` / `=` / `：` (joined, quoted or in the
///   next word; the value read as after `Bearer`), after `password` and
///   `login` in a `.netrc` text (`netrc_login`), or after 密码 / 令牌 /
///   密钥 / 口令; placeholders stay;
/// - the value of an `X-Api-Key:` / `Cookie:` header while its words start
///   with ASCII (`line_header_value`);
/// - inside any word, a JWT, a token with a known prefix, or a long hex or
///   base64 run, also after the run's last `=` (`secret_span`).
fn shapes(text: &str) -> String {
    let (lead, mut words) = tokens(text);
    let mut output = lead;
    let mut pending: Option<Pending> = None;
    let netrc_values = netrc_values(&mut words);
    let mut index = 0;
    while index < words.len() {
        let (word, gap) = &words[index];
        let form = header_form(word);
        if LINE_HEADERS.iter().any(|header| form.starts_with(header)) {
            let from = separator(word).map_or(word.len(), |(_, from)| from);
            if let Some((rendered, last)) = line_header_value(&words, index, from) {
                pending = None;
                output.push_str(&rendered);
                output.push_str(&words[last].1);
                index = last + 1;
                continue;
            }
        }
        if let Some(last) = netrc_values[index] {
            pending = None;
            output.push_str(TEXT_MARKER);
            output.push_str(&words[last].1);
            index = last + 1;
            continue;
        }
        let mut sent = None;
        if sent.is_none() {
            match pending.take() {
                Some(Pending::Value | Pending::Login) => sent = replace_value(word, 0),
                Some(Pending::Likely) if credential_like(word) => sent = replace_value(word, 0),
                Some(Pending::Key { .. }) if matches!(form.as_str(), ":" | "=") => {
                    pending = Some(Pending::Value);
                }
                Some(Pending::Key { .. }) if form.starts_with([':', '=']) => {
                    sent = separator(word).and_then(|(_, from)| replace_value(word, from));
                }
                Some(Pending::Key { netrc: true }) if credential_like(word) => {
                    sent = replace_value(word, 0);
                }
                _ => {}
            }
        } else {
            pending = None;
        }
        if sent.is_none() && pending.is_none() {
            match separator(word) {
                Some((split, from)) if secret_key(&word[..split]) => {
                    if word[from..].chars().any(char::is_alphanumeric) {
                        sent = replace_value(word, from);
                    } else {
                        pending = Some(Pending::Value);
                    }
                }
                None if secret_key(word) => {
                    pending = Some(Pending::Key {
                        netrc: netrc_key(word),
                    });
                }
                None if netrc_login(&words, index, false) => pending = Some(Pending::Login),
                _ => {
                    if let Some((from, explicit)) = cjk_key(word) {
                        if from == word.len() {
                            pending = Some(if explicit {
                                Pending::Value
                            } else {
                                Pending::Likely
                            });
                        } else if explicit || credential_like(&word[from..]) {
                            sent = replace_value(word, from);
                        }
                    }
                }
            }
        }
        if gap.contains('\n') {
            pending = None;
        }
        output.push_str(&shape_word(sent.as_deref().unwrap_or(word)));
        output.push_str(gap);
        index += 1;
    }
    output
}

/// Words with the whitespace after each, and the whitespace before the first.
fn tokens(text: &str) -> (String, Vec<(String, String)>) {
    let mut tokens: Vec<(String, String)> = Vec::new();
    let mut lead = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            match tokens.last_mut() {
                Some((_, gap)) => gap.push(c),
                None => lead.push(c),
            }
        } else {
            match tokens.last_mut() {
                Some((word, gap)) if gap.is_empty() => word.push(c),
                _ => tokens.push((c.to_string(), String::new())),
            }
        }
    }
    (lead, tokens)
}

/// A word lower-cased for header matching: quotes and brackets removed,
/// full-width `：` and `＝` read as `:` and `=`.
fn header_form(word: &str) -> String {
    word.chars()
        .filter(|c| {
            !matches!(
                c,
                '"' | '\''
                    | '“'
                    | '”'
                    | '‘'
                    | '’'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '（'
                    | '）'
            )
        })
        .map(|c| match c {
            '：' => ':',
            '＝' => '=',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

/// The word without leading and trailing punctuation (ASCII or not).
fn bare(word: &str) -> String {
    header_form(word)
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_owned()
}

fn scheme_word(word: &str) -> bool {
    matches!(bare(word).as_str(), "bearer" | "basic")
}

/// Where the credential in `word` ends (bytes), when it starts with one: an
/// opening quote or bracket, then a run of token characters (a token or
/// base64 text, which carries digits, symbols or mixed case) at least 8
/// long, so `question` is not one and `<jwt>,` is (up to the comma); or the
/// transport scrub's `[redacted]`. What follows (`，AOI`) is not part of it.
fn credential_end(word: &str) -> Option<usize> {
    if word.starts_with("[redacted]") {
        return Some("[redacted]".len());
    }
    if word.starts_with(TEXT_MARKER) || word.starts_with(VALUE_MARKER) {
        return None;
    }
    let symbol = |c: char| matches!(c, '+' | '/' | '=' | '.' | '_' | '-' | '~');
    // Opening quotes, brackets and other marks (full-width and Chinese ones
    // included: 【 《 ＂ ｀ · …) are not part of the token.
    let body = word.trim_start_matches(|c: char| !c.is_alphanumeric());
    let run: String = body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || symbol(*c))
        .collect();
    let count = |test: fn(&char) -> bool| run.chars().filter(test).count();
    let token = run.len() >= 8
        && (run.chars().any(|c| c.is_ascii_digit() || symbol(c))
            || (count(char::is_ascii_uppercase) >= 2 && count(char::is_ascii_lowercase) >= 2));
    token.then(|| word.len() - body.len() + run.len())
}

fn credential_like(word: &str) -> bool {
    credential_end(word).is_some()
}

/// Where the credential ends in the word right after `Bearer` / `Basic` in
/// a header, which is the credential whatever its shape (W3l review: a
/// backquoted token, `admin:secret`, `ya29.…%2F…`, `s3cr3t`): after any
/// opening mark (anything not a letter or digit: quotes, brackets,
/// backquotes, full-width and Chinese ones such as 【 《 ＂ ｀ ·), its whole
/// leading ASCII run, less
/// closing punctuation at the run's end (`)]}"'`>,.;`). Only that
/// punctuation and a tail starting with a non-ASCII character stay. `None`
/// when the word starts with no ASCII (Chinese text) or is a marker.
fn scheme_credential_end(word: &str) -> Option<usize> {
    if word.starts_with("[redacted]") {
        return Some("[redacted]".len());
    }
    if word.starts_with(TEXT_MARKER) || word.starts_with(VALUE_MARKER) {
        return None;
    }
    let body = word.trim_start_matches(|c: char| !c.is_alphanumeric());
    let run_len = body
        .find(|c: char| !c.is_ascii() || c.is_ascii_whitespace())
        .unwrap_or(body.len());
    let run = body[..run_len].trim_end_matches([')', ']', '}', '"', '\'', '`', '>', ',', '.', ';']);
    (!run.is_empty()).then(|| word.len() - body.len() + run.len())
}

/// Where the credential ends in a word that joins a scheme and its token
/// without a space (third redaction guard: `Bearer:<token>`, `Bearer：<token>`,
/// `token:<token>`): the part before the first `:` / `：` / `=` is `bearer`,
/// `basic` or `token`, and the part after it is read by
/// `scheme_credential_end`.
fn joined_credential_end(word: &str) -> Option<usize> {
    let (split, separator) = word
        .char_indices()
        .find(|(_, c)| matches!(c, ':' | '：' | '='))?;
    if !matches!(bare(&word[..split]).as_str(), "bearer" | "basic" | "token") {
        return None;
    }
    let rest = split + separator.len_utf8();
    scheme_credential_end(&word[rest..]).map(|end| rest + end)
}

/// A credential header starts at word `index`: an `authorization` header
/// name followed by `:` or `=` (quoted, full-width or in the next word), or
/// `bearer` / `basic` (punctuation aside) followed by a credential-like word
/// (on this line or the next).
fn is_header(words: &[(String, String)], index: usize) -> bool {
    let form = header_form(&words[index].0);
    let next = words.get(index + 1).map(|(word, _)| word.as_str());
    let named = form.contains("authorization:")
        || form.contains("authorization=")
        || (bare(&words[index].0) == "authorization"
            && next.is_some_and(|next| {
                let next = header_form(next);
                next.starts_with(':') || next.starts_with('=')
            }));
    named || (scheme_word(&words[index].0) && next.is_some_and(credential_like))
}

/// A word that continues a header: a scheme word, a lone `:` / `=`, or
/// another `authorization` name.
fn header_part(word: &str) -> bool {
    let form = header_form(word);
    scheme_word(word) || matches!(form.as_str(), ":" | "=") || bare(word) == "authorization"
}

/// One word as it is sent: kept (or, with word scrubbing, `sendable_word`),
/// or replaced up to `tail` (the text after the credential, which stays).
enum Piece {
    Keep(String),
    Replace(String),
}

/// The words of `text` with credential headers taken out (W3l, L4 r8 C15:
/// the rest of the line is no longer replaced, so the user's question
/// stays). From a header (`is_header`) on, header words (`header_part`) and
/// credential-like words are replaced; the first other word ends the header
/// and stays, as does whatever follows a credential inside its word
/// (`abc123…，AOI` keeps `，AOI`). A header that has replaced no credential
/// yet keeps waiting to the end of its line (`Authorization: Bearer 如下：`
/// then the token). When the line ends on a waiting header or on a replaced
/// credential, the next line's first word is replaced too if it is
/// credential-like (a token on the next line, or one cut by a hard line
/// break), or a header word (`Authorization:` / `Bearer <token>`). Adjacent
/// replaced words on one line become one [`TEXT_MARKER`]. With
/// `scrub_words`, every other word the check would not pass as it stands is
/// replaced too (`sendable_word`).
fn rewrite(text: &str, scrub_words: bool) -> String {
    let (lead, words) = tokens(text);
    let mut pieces: Vec<(Piece, &str)> = Vec::with_capacity(words.len());
    // In a run of header words and credentials right after a header.
    let mut run = false;
    // A header on this line has replaced no credential yet.
    let mut waiting = false;
    // The next line's first word is checked for a credential.
    let mut carry = false;
    // The word just replaced was `Bearer` / `Basic` in a header.
    let mut after_scheme = false;
    for (index, (word, gap)) in words.iter().enumerate() {
        let new_line = index > 0 && words[index - 1].1.contains('\n');
        if new_line {
            run = false;
            waiting = false;
        }
        let carried = new_line && std::mem::take(&mut carry);
        if new_line && !carried {
            after_scheme = false;
        }
        let follows_scheme = std::mem::take(&mut after_scheme);
        let replace_credential = |tail_from: usize| Piece::Replace(word[tail_from..].to_owned());
        let credential = if follows_scheme {
            scheme_credential_end(word)
        } else {
            credential_end(word)
        };
        let piece = if let Some(end) = joined_credential_end(word) {
            waiting = false;
            after_scheme = false;
            run = end == word.len();
            replace_credential(end)
        } else if is_header(&words, index) || (carried && header_part(word)) {
            run = true;
            waiting = true;
            after_scheme = scheme_word(word);
            Piece::Replace(String::new())
        } else if run && header_part(word) {
            after_scheme = scheme_word(word);
            Piece::Replace(String::new())
        } else if let Some(end) = credential.filter(|_| run || waiting || carried) {
            waiting = false;
            run = end == word.len();
            replace_credential(end)
        } else {
            run = false;
            Piece::Keep(if scrub_words {
                sendable_word(word)
            } else {
                word.clone()
            })
        };
        if gap.contains('\n') {
            carry = waiting || (run && matches!(piece, Piece::Replace(_)));
        }
        pieces.push((piece, gap.as_str()));
    }
    let mut output = lead;
    let mut pending_gap = "";
    let mut at_marker = false;
    for (piece, gap) in pieces {
        match piece {
            Piece::Replace(tail) => {
                // Adjacent replaced words on one line share one marker.
                if !at_marker || pending_gap.contains('\n') {
                    output.push_str(pending_gap);
                    output.push_str(TEXT_MARKER);
                }
                at_marker = tail.is_empty();
                output.push_str(&tail);
            }
            Piece::Keep(text) => {
                output.push_str(pending_gap);
                output.push_str(&text);
                at_marker = false;
            }
        }
        pending_gap = gap;
    }
    output.push_str(pending_gap);
    output
}

/// A refused marker word (`bearer`, an `authorization` header name) with the
/// word after it, replaced by one [`TEXT_MARKER`].
fn marker_pairs(text: &str) -> String {
    let (lead, words) = tokens(text);
    let mut output = lead;
    let mut index = 0;
    while index < words.len() {
        let form = header_form(&words[index].0);
        if bare(&words[index].0) == "bearer" || form.contains("authorization") {
            let last = (index + 1).min(words.len() - 1);
            output.push_str(TEXT_MARKER);
            output.push_str(&words[last].1);
            index = last + 1;
        } else {
            output.push_str(&words[index].0);
            output.push_str(&words[index].1);
            index += 1;
        }
    }
    output
}

/// Local paths with whatever spaces they hold, replaced by [`TEXT_MARKER`]
/// before the text is cut into words (redaction guard: `C:/Users/Zhang San/x` sent
/// `San/x`). A path starts at a drive letter (a letter, a colon and a slash
/// or backslash, not right after an ASCII letter or digit, so `https://` is
/// not one), at `file://`, at a UNC double backslash, at a `~` followed by
/// `/` (a home path, not right after an ASCII letter or digit, so `math~/2`
/// is not one), or at a `/` that begins a Unix path: the character before it
/// is none of an ASCII letter, digit, `.`, `~` or `:` (`https://a/b`,
/// `TCP/IP`, `math.pi/2` are not), the next character is neither `/` nor
/// whitespace (`//server`, a lone `/`), and the first segment starts with a
/// letter (`1/2`, `2026/11/02`, `24/7` are not). The path ends at the line
/// end, at a quote or `<>|`, at non-ASCII punctuation (full-width marks),
/// or at a non-ASCII letter that is not part of the path. A non-ASCII letter
/// is part of the path (second review P1-A, `C:\Users\张三\x.tif`) when it
/// follows a separator or joiner (message_dir's `connector`: `\ / _ - .` ...)
/// or when the rest of its word still holds a separator; the path then runs
/// to the end of that word (whitespace or non-ASCII punctuation). Spaces
/// before the end stay outside.
/// What may follow a `%2F` separator for it to begin an encoded path
/// (third review): an ASCII letter, or another percent-encoded UTF-8 lead
/// byte `%C2`–`%F4` (case-insensitive), so `%2Fhome` and
/// `%2F%E4%B8%8B%E8%BD%BD` are paths while `%2F2` and `%2F%20` are not.
fn encoded_lead(rest: &[char]) -> bool {
    match rest.first() {
        Some(c) if c.is_ascii_alphabetic() => true,
        Some('%') => rest
            .get(1..3)
            .and_then(|hex| u8::from_str_radix(&hex.iter().collect::<String>(), 16).ok())
            .is_some_and(|byte| (0xC2..=0xF4).contains(&byte)),
        _ => false,
    }
}

fn path_runs(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let at = |index: usize, pattern: &str| {
        pattern.chars().enumerate().all(|(offset, c)| {
            chars
                .get(index + offset)
                .is_some_and(|found| found.eq_ignore_ascii_case(&c))
        })
    };
    let mut byte_offsets: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    byte_offsets.push(text.len());
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let previous = index.checked_sub(1).map(|k| chars[k]);
        let after_word = previous.is_some_and(|c| c.is_ascii_alphanumeric());
        let word_start = previous.is_none_or(char::is_whitespace);
        let drive = chars[index].is_ascii_alphabetic()
            && chars.get(index + 1) == Some(&':')
            && matches!(chars.get(index + 2), Some('\\' | '/'))
            && !after_word;
        let file = at(index, "file://") && !after_word;
        let unc = word_start
            && at(index, "\\\\")
            && chars
                .get(index + 2)
                .is_some_and(|c| c.is_ascii_alphanumeric());
        let tilde = chars[index] == '~'
            && chars.get(index + 1) == Some(&'/')
            && !after_word
            && !matches!(chars.get(index + 2), None | Some(&'/'));
        let rooted = chars[index] == '/'
            && crate::run::unix_path_after_slash(previous, &text[byte_offsets[index + 1]..]);
        // A URL-encoded path (`%2Fhome%2Fme%2Fx`, `%2F%E4%B8%8B%E8%BD%BD`,
        // second redaction guard): the code's left boundary is the Unix path's,
        // and what follows is an ASCII letter or another percent-encoded
        // UTF-8 lead byte `%C2`–`%F4` (`?a=1%2F2` — a digit — and
        // `%2F%20` — a space — are not).
        let encoded = chars[index] == '%'
            && at(index, "%2F")
            && previous.is_none_or(|c| {
                !c.is_ascii_alphanumeric() && !matches!(c, '.' | '~' | ':' | '/' | '%')
            })
            && encoded_lead(&chars[index + 3..]);
        // Another user's home (`~user/x`, second review P3): `~`, a run of
        // name characters and `/`, with the Unix path's left boundary.
        let home = chars[index] == '~'
            && !after_word
            && previous.is_none_or(|c| {
                !c.is_ascii_alphanumeric() && !matches!(c, '.' | '~' | ':' | '/' | '%')
            })
            && chars[index + 1..]
                .iter()
                .position(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
                .is_some_and(|offset| chars[index + 1 + offset] == '/');
        if !(drive || file || unc || tilde || rooted || encoded || home) {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        let stop = |c: char| c == '\n' || matches!(c, '"' | '\'' | '<' | '>' | '|');
        let punctuation = |c: char| !c.is_ascii() && !c.is_alphanumeric();
        let mut end = index;
        while end < chars.len() && !stop(chars[end]) {
            let c = chars[end];
            if c.is_ascii() {
                end += 1;
                continue;
            }
            if punctuation(c) {
                break;
            }
            let word_end = (end..chars.len())
                .find(|&k| chars[k].is_whitespace() || stop(chars[k]) || punctuation(chars[k]))
                .unwrap_or(chars.len());
            let joined = crate::message_dir::connector(chars[end - 1]);
            let separated = chars[end..word_end].iter().any(|c| matches!(c, '\\' | '/'));
            if joined || separated {
                end = word_end;
            } else {
                break;
            }
        }
        while end > index && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        // Trailing sentence punctuation (`/data/S1.zip。` drops the full
        // stop; `(/etc/config)` keeps its closing bracket) is not part of
        // a Unix path (review 2026-11-02).
        if chars[index] == '/' {
            while end > index && matches!(chars[end - 1], '.' | ',' | ';' | ':' | ')' | ']') {
                end -= 1;
            }
        }
        output.push_str(TEXT_MARKER);
        index = end;
    }
    output
}

/// A string value as it may travel inside a JSON string.
fn nested_string(text: &str) -> Value {
    // A credential header in a tool result (the loop's scrub may have left
    // `Bearer [redacted] <token>`) goes before the word checks.
    let redacted = redact_credentials(text);
    let text = redacted.as_str();
    let escaped_safe = |text: &str| {
        serde_json::to_string(text)
            .map(|escaped| safe_text(&escaped))
            .unwrap_or(false)
    };
    if !safe_text(text) {
        return json!(VALUE_MARKER);
    }
    if escaped_safe(text) {
        return json!(text);
    }
    let mut spaced = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        spaced.push(c);
        if c == ':'
            && chars
                .peek()
                .is_some_and(|next| matches!(next, '"' | '\\') || next.is_control())
        {
            spaced.push(' ');
        }
    }
    if safe_text(&spaced) && escaped_safe(&spaced) {
        json!(spaced)
    } else {
        json!(VALUE_MARKER)
    }
}

/// A tool result or argument value made safe to send nested in a string.
pub fn nested_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| {
                    let mut probe = serde_json::Map::new();
                    probe.insert((*key).clone(), Value::Null);
                    crate::run::safe_value(&Value::Object(probe)).is_ok()
                        && serde_json::to_string(key.as_str())
                            .map(|escaped| safe_text(&escaped))
                            .unwrap_or(false)
                })
                .map(|(key, entry)| (key.clone(), nested_value(entry)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(nested_value).collect()),
        Value::String(text) => nested_string(text),
        _ => value.clone(),
    }
}

fn serialized_nested_safe(val: &Value) -> bool {
    let raw = val.to_string();
    if !safe_text(&raw) {
        return false;
    }
    let scrubbed = privacy::scrub(val).to_string();
    safe_text(&scrubbed)
}

fn replace_unsafe_nested_strings(val: Value) -> Value {
    match val {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, replace_unsafe_nested_strings(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(replace_unsafe_nested_strings)
                .collect(),
        ),
        Value::String(s) => {
            let candidate = Value::String(s);
            if serialized_nested_safe(&candidate) {
                candidate
            } else {
                json!(VALUE_MARKER)
            }
        }
        other => other,
    }
}

fn ensure_serialized_nested_safe(val: Value) -> Value {
    if serialized_nested_safe(&val) {
        return val;
    }
    let narrowed = replace_unsafe_nested_strings(val);
    if serialized_nested_safe(&narrowed) {
        return narrowed;
    }
    json!({ "sanitized": VALUE_MARKER })
}

/// The conversation as it is sent: user and assistant text with unsafe words
/// replaced, tool results and tool-call arguments made safe to nest. The
/// system prompt is kept verbatim when already safe, and scrubbed only if an
/// extension/plugin contributed an unsafe substring.
pub fn messages(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .map(|message| {
            if message.role == "user_multimodal" {
                if let Ok(payload) = serde_json::from_str::<Value>(&message.content) {
                    let text = payload
                        .get("text")
                        .and_then(Value::as_str)
                        .map(redact_words)
                        .unwrap_or_default();
                    let mut images = Vec::new();
                    if let Some(items) = payload.get("images").and_then(Value::as_array) {
                        for item in items {
                            let mime = item
                                .get("mime_type")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let data = item
                                .get("data_base64")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            if let Some((norm_mime, norm_b64)) =
                                privacy::normalize_image_attachment(mime, data)
                            {
                                images.push(json!({
                                    "mime_type": norm_mime,
                                    "data_base64": norm_b64
                                }));
                            }
                        }
                    }
                    if images.is_empty() {
                        return Message {
                            role: "user".to_owned(),
                            content: text,
                        };
                    }
                    return Message {
                        role: "user_multimodal".to_owned(),
                        content: json!({
                            "text": text,
                            "images": images
                        })
                        .to_string(),
                    };
                }
                return Message {
                    role: "user".to_owned(),
                    content: redact_words(&message.content),
                };
            }
            let content = match message.role.as_str() {
                "user" | "assistant" => redact_words(&message.content),
                "system" => {
                    if safe_text(&message.content) {
                        message.content.clone()
                    } else {
                        redact_words(&message.content)
                    }
                }
                "tool" => match serde_json::from_str::<Value>(&message.content) {
                    Ok(mut payload) => {
                        if let Some(result) = payload.get("result") {
                            payload["result"] = ensure_serialized_nested_safe(nested_value(result));
                        }
                        payload.to_string()
                    }
                    Err(_) => message.content.clone(),
                },
                "assistant_tool_calls" => match serde_json::from_str::<Value>(&message.content) {
                    Ok(mut payload) => {
                        if let Some(text) = payload.get("text").and_then(Value::as_str) {
                            payload["text"] = json!(redact_words(text));
                        }
                        if let Some(calls) =
                            payload.get_mut("tool_calls").and_then(Value::as_array_mut)
                        {
                            for call in calls {
                                if let Some(arguments) = call.get("arguments") {
                                    call["arguments"] =
                                        ensure_serialized_nested_safe(nested_value(arguments));
                                }
                            }
                        }
                        payload.to_string()
                    }
                    Err(_) => message.content.clone(),
                },
                _ => message.content.clone(),
            };
            Message {
                role: message.role.clone(),
                content,
            }
        })
        .collect()
}

/// Which part of a request body the outbound check refuses, by class only
/// (never content): `system_prompt`, `user_message`, `assistant_message`,
/// `tool_call`, `tool_result`, `tool_schema`, else `request_envelope`. Reads
/// the Chat Completions, Responses, Anthropic Messages, and Gemini layouts.
pub fn field_class(body: &Value) -> &'static str {
    let refused = |value: &Value| !passes(value);
    if body.get("tools").is_some_and(refused) {
        return "tool_schema";
    }
    if body.get("system").is_some_and(refused)
        || body.get("instructions").is_some_and(refused)
        || body.get("systemInstruction").is_some_and(refused)
    {
        return "system_prompt";
    }
    let items = body
        .get("messages")
        .or_else(|| body.get("input"))
        .or_else(|| body.get("contents"))
        .and_then(Value::as_array);
    for item in items.into_iter().flatten() {
        if !refused(item) {
            continue;
        }
        let role = item.get("role").and_then(Value::as_str);
        let kind = item.get("type").and_then(Value::as_str);
        let blocks: Vec<&str> = item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| refused(block))
            .filter_map(|block| block.get("type").and_then(Value::as_str))
            .collect();
        let gemini_parts = item.get("parts").and_then(Value::as_array);
        let has_gemini_fn_call = gemini_parts.is_some_and(|parts| {
            parts
                .iter()
                .any(|p| refused(p) && p.get("functionCall").is_some())
        });
        let has_gemini_fn_resp = gemini_parts.is_some_and(|parts| {
            parts
                .iter()
                .any(|p| refused(p) && p.get("functionResponse").is_some())
        });
        return match (role, kind) {
            (Some("system"), _) => "system_prompt",
            (Some("tool"), _) | (_, Some("function_call_output")) => "tool_result",
            (_, Some("function_call")) => "tool_call",
            (Some("user"), _) if blocks.contains(&"tool_result") || has_gemini_fn_resp => {
                "tool_result"
            }
            (Some("assistant" | "model"), _)
                if blocks.contains(&"tool_use")
                    || has_gemini_fn_call
                    || item.get("tool_calls").is_some_and(refused) =>
            {
                "tool_call"
            }
            (Some("assistant" | "model"), _) => "assistant_message",
            (Some("user"), _) => "user_message",
            _ => "request_envelope",
        };
    }
    "request_envelope"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W3n (L4 r9 C15 probes 2 and 4): a bare secret with no Bearer or
    /// Authorization header was sent as written. The token goes; the
    /// question and the text after it stay.
    #[test]
    fn the_r9_bare_secret_probes_keep_the_question() {
        let backtick = "我的令牌是 `tok_FAKE9876543210abc`，配置在 C:\\Users\\demo\\Desktop\\eostage-test\\config.json。先复述一下你从这句话里看到了什么，然后告诉我当前 AOI 大约多少平方公里。";
        assert_eq!(
            redact_words(backtick),
            "我的令牌是 `[local-reference]`，配置在 [local-reference]。先复述一下你从这句话里看到了什么，然后告诉我当前 AOI 大约多少平方公里。"
        );
        let jwt = "令牌【eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmYWtlLXVzZXIifQ.FAKEsigABC123xyz】，脚本在 C:\\Users\\demo\\Desktop\\eostage-test\\run.ps1。先复述一下你从这句话里看到了什么，然后告诉我当前 AOI 大约多少平方公里。";
        assert_eq!(
            redact_words(jwt),
            "令牌【[local-reference]】，脚本在 [local-reference]。先复述一下你从这句话里看到了什么，然后告诉我当前 AOI 大约多少平方公里。"
        );
    }

    fn assert_redacted(text: &str, secret: &str, kept: &[&str]) {
        let sent = redact_words(text);
        assert!(!sent.contains(secret), "{text:?} -> {sent:?}");
        for part in kept {
            assert!(sent.contains(part), "{part:?} lost: {text:?} -> {sent:?}");
        }
        let value = Value::String(sent.clone());
        assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{text:?}");
    }

    /// W3n rule 1: a JWT (three base64url parts, the first starting eyJ).
    #[test]
    fn a_bare_jwt_is_replaced() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl";
        assert_eq!(
            redact_words(&format!("看 {jwt} 吧")),
            "看 [local-reference] 吧"
        );
        assert_redacted(&format!("令牌({jwt})好"), jwt, &["令牌(", ")好"]);
        // Cut short (a 40-character session title): the parts there are go.
        assert_eq!(
            redact_words("令牌【eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmYWtl…"),
            "令牌【[local-reference]…"
        );
        assert_eq!(
            redact_words("令牌 eyJhbGciOiJIUzI1NiJ9…"),
            "令牌 [local-reference]…"
        );
        // `eyJ` alone, or a short word starting with it, is not a JWT.
        assert_eq!(redact_words("eyJ 和 eyJabc.d"), "eyJ 和 eyJabc.d");
    }

    /// W3n rule 2: tokens with a known prefix and 8+ token characters.
    #[test]
    fn a_token_with_a_known_prefix_is_replaced() {
        for token in [
            "sk-proj-abc123DEF456ghi",
            "sk_live_51Habc123def456",
            "tok_FAKE9876543210abc",
            "ghp_abcdefghij1234567890ABCDEF",
            "gho_abcdefghij1234567890ABCDEF",
            "github_pat_11ABCDEFG0123456789_abcdef",
            "xoxb-1234567890-abcdefghij",
            "xoxp-1234567890-abcdefghij",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSyA1234567890abcdefghij",
            "ya29.a0AfB%2Fxyz789SECRET",
            "glpat-abc123def456ghi789",
            "npm_abc123def456ghi789jkl",
            "pypi-AgEIcHlwaS5vcmc123abc",
        ] {
            assert_eq!(
                redact_words(&format!("令牌 {token} 好")),
                "令牌 [local-reference] 好",
                "{token}"
            );
        }
        // A prefix followed by an ordinary short word is not a token.
        assert_eq!(redact_words("用 sk-learn 做"), "用 sk-learn 做");
    }

    /// W3n rule 3: the value after a secret key name, and the value of an
    /// X-Api-Key or Cookie header to the end of its line.
    #[test]
    fn a_secret_key_value_is_replaced() {
        assert_redacted(
            "配置 api_key=abc123def456 完",
            "abc123def456",
            &["配置 api_key=", " 完"],
        );
        assert_redacted(
            "password: hunter2pass 完",
            "hunter2pass",
            &["password:", " 完"],
        );
        assert_redacted(
            "密码 passwd：Passw0rd 完",
            "Passw0rd",
            &["密码 passwd：", " 完"],
        );
        assert_redacted(
            "\"client_secret\": \"xyz789abc\"",
            "xyz789abc",
            &["client_secret"],
        );
        assert_redacted(
            "access_token = ghs123abcXYZ 完",
            "ghs123abcXYZ",
            &["access_token", " 完"],
        );
        assert_redacted("pwd=s3cr3t!", "s3cr3t", &["pwd="]);
        assert_eq!(
            redact_words("X-Api-Key: abcdef123456 其他\n下一行"),
            "X-Api-Key: [local-reference] 其他\n下一行"
        );
        assert_eq!(
            redact_words("curl -H 'Cookie: sid=abc123; uid=9' https://x.example\n下一行"),
            "curl -H 'Cookie: [local-reference]\n下一行"
        );
        // A key name before ordinary text is left alone.
        assert_eq!(redact_words("password: 请帮我重置"), "password: 请帮我重置");
    }

    /// W3n rule 4: a PEM private key block as a whole.
    #[test]
    fn a_pem_private_key_block_is_replaced() {
        let text = "钥匙\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1234abcd\nZm9vYmFy\n-----END RSA PRIVATE KEY-----\n后文";
        assert_eq!(redact_words(text), "钥匙\n[local-reference]\n后文");
    }

    #[test]
    fn a_line_header_value_stops_at_chinese_text() {
        // W3n redaction guard-1: the value of X-Api-Key / Cookie must start with
        // ASCII and ends at the first word (or character) that is not.
        for text in [
            "浏览器的 cookie: 怎么清除？还有别的问题吗",
            "Cookie: 是什么",
            "X-Api-Key：在哪里配置",
        ] {
            assert_eq!(redact_words(text), text);
        }
        assert_eq!(
            redact_words("请把 Cookie: sid=abc123 换掉，然后告诉我为什么 401"),
            "请把 Cookie: [local-reference] 换掉，然后告诉我为什么 401"
        );
        assert_eq!(
            redact_words("X-Api-Key:abc123，帮我看看 AOI"),
            "X-Api-Key:[local-reference]，帮我看看 AOI"
        );
        assert_eq!(
            redact_words("Cookie: sid=abc123; uid=9 然后呢"),
            "Cookie: [local-reference] 然后呢"
        );
    }

    /// W3n redaction guard-2: a key name containing a secret word (as the run
    /// record's key check reads names), and a shape after the last `=`.
    #[test]
    fn a_secret_named_by_any_key_containing_a_secret_word_is_replaced() {
        assert_redacted(
            "export OPENAI_API_KEY=sk-proj-abc123DEF456ghi 然后运行",
            "abc123DEF",
            &["export OPENAI_API_KEY=", " 然后运行"],
        );
        assert_redacted(
            "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "wJalrXUtnFEMI",
            &["AWS_SECRET_ACCESS_KEY="],
        );
        assert_eq!(
            redact_words("jwt=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl"),
            "jwt=[local-reference]"
        );
        assert_eq!(
            redact_words("key=0123456789abcdef0123456789abcdef"),
            "key=[local-reference]"
        );
        assert_redacted(
            "配置是 {\"refresh_token\": \"r1a2b3c4d5\"} 对吗",
            "r1a2b3c4d5",
            &["配置是 ", "refresh_token", " 对吗"],
        );
        assert_eq!(redact_words("max_tokens: 4096"), "max_tokens: 4096");
    }

    /// W3n redaction guard-3: a value right after 密码 / 令牌 / 密钥 / 口令, and
    /// the word after `password` in a .netrc line.
    #[test]
    fn a_secret_after_a_chinese_key_name_or_a_netrc_password_is_replaced() {
        assert_eq!(
            redact_words("我的密码：Abc12345，记得改"),
            "我的密码：[local-reference]，记得改"
        );
        assert_eq!(
            redact_words("令牌是 abcd1234efgh5678 请保存"),
            "令牌是 [local-reference] 请保存"
        );
        assert_eq!(redact_words("口令为Passw0rd!"), "口令为[local-reference]");
        assert_eq!(
            redact_words("密钥 wJalrXUtnFEMI7K 别外传"),
            "密钥 [local-reference] 别外传"
        );
        assert_eq!(
            redact_words("machine urs.earthdata.nasa.gov login alice password MyP4ss123"),
            "machine urs.earthdata.nasa.gov login [local-reference] password [local-reference]"
        );
        for text in [
            "密码是什么？",
            "我的令牌 token 怎么获取",
            "password reset 怎么做",
            "密码学 入门",
        ] {
            assert_eq!(redact_words(text), text);
        }
    }

    /// L4 r10 P2-3: in a .netrc line the account name after `login` goes
    /// too (`login alice` was sent as written); in prose `login` is a word.
    #[test]
    fn netrc_quote_pairs_are_scanned_from_opening_token() {
        let (_, words) = tokens("default login \"\" password fake 请检查");
        assert_eq!(netrc_value_end(&words, 2), Some(2));
        let (_, words) = tokens("default login '' password fake 请检查");
        assert_eq!(netrc_value_end(&words, 2), Some(2));
    }

    #[test]
    fn netrc_closed_quote_preserves_same_token_suffix() {
        for (opening, closing) in [
            ('"', '"'),
            ('\'', '\''),
            ('“', '”'),
            ('‘', '’'),
            ('＂', '＂'),
            ('＇', '＇'),
            ('`', '`'),
        ] {
            for value in ["fake", "fake phrase", ""] {
                for suffix in ["请检查", "pleaseCheck"] {
                    let text = format!("default login {opening}{value}{closing}{suffix}");
                    assert_eq!(
                        redact_words(&text),
                        format!("default login [local-reference]{suffix}"),
                        "{text:?}"
                    );
                }
            }
        }
        assert_eq!(
            redact_words("default login \"fake\\\" phrase\"请检查"),
            "default login [local-reference]请检查"
        );
    }

    #[test]
    fn netrc_closed_quote_suffix_is_scrubbed_as_text() {
        for (text, secrets, suffix) in [
            (
                "default login \"fake\"请检查tok_FAKE9876543210abc",
                vec!["fake", "tok_FAKE9876543210abc"],
                "请检查",
            ),
            (
                "default login “fake”password=bob 请检查",
                vec!["fake", "bob"],
                "请检查",
            ),
            (
                "default login ＂fake＂password \"bob phrase\"请检查",
                vec!["fake", "bob", "phrase"],
                "请检查",
            ),
            (
                "login `fake`password bob 请检查",
                vec!["fake", "bob"],
                "请检查",
            ),
            (
                "default login \"fake\"C:\\fixture\\after.txt 请检查",
                vec!["fake", "fixture", "after.txt"],
                "请检查",
            ),
            (
                "default login \"fake\"password \"UNFINISHED SECRET\n后续散文",
                vec!["fake", "UNFINISHED", "SECRET", "后续散文"],
                "password",
            ),
        ] {
            let sent = redact_words(text);
            assert!(sent.contains(suffix), "{text:?} -> {sent:?}");
            for secret in secrets {
                assert!(!sent.contains(secret), "{text:?} -> {sent:?}");
            }
            let value = Value::String(sent);
            assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{text:?}");
        }
    }

    #[test]
    fn a_netrc_login_is_replaced_with_its_password() {
        assert_eq!(
            redact_words("machine urs.earthdata.nasa.gov login alice password FAKEp4ss123"),
            "machine urs.earthdata.nasa.gov login [local-reference] password [local-reference]"
        );
        assert_eq!(
            redact_words(
                "我的配置：\nmachine urs.earthdata.nasa.gov\n  login alice\n  password FAKEp4ss123\n还缺什么？"
            ),
            "我的配置：\nmachine urs.earthdata.nasa.gov\n  login [local-reference]\n  password [local-reference]\n还缺什么？"
        );
        assert_eq!(
            redact_words("login alice password FAKEp4ss123"),
            "login [local-reference] password [local-reference]"
        );
        for text in [
            "login page 打不开",
            "请 login 后再试",
            "Earthdata login 失败怎么办",
        ] {
            assert_eq!(redact_words(text), text);
        }
    }

    /// W3n review P3-4: a placeholder after a key name is kept as it is.
    #[test]
    fn placeholders_after_a_key_name_stay() {
        for text in [
            "pwd: [local-reference:message_dir] 下载到这里",
            "pwd=[local-reference:message_dir]",
            "password: [redacted] 好",
            "secret: [local] 好",
            "token：[local-reference] 好",
        ] {
            assert_eq!(redact_words(text), text);
        }
        assert_eq!(
            crate::history::outbound_text("pwd: D:\\work\\sar 下载到这里"),
            "pwd: [local-reference] 下载到这里"
        );
    }

    /// W3n review P3-5: a joined value is taken up to the first non-ASCII
    /// character before the transport's scrub, which would take the text
    /// up to the next space.
    #[test]
    fn a_joined_key_value_keeps_the_chinese_text_after_it() {
        assert_eq!(
            redact_words("api_key=abc123，然后告诉我 AOI 面积"),
            "api_key=[redacted]，然后告诉我 AOI 面积"
        );
        assert_eq!(
            redact_words("password=hunter2，然后告诉我 AOI 面积"),
            "password=[redacted]，然后告诉我 AOI 面积"
        );
    }

    /// `product_like` reads names by prefix, extensions by suffix and CMR
    /// ids by pattern, not by substring.
    #[test]
    fn product_names_are_matched_by_prefix_suffix_or_pattern() {
        for name in [
            "S1A_IW_SLC__1SDV_20261018T110831_20261018T110858_005069_009970_518E",
            "S1_136231_IW2_20261018T110831_VV_518E-BURST",
            "NISAR_L1_PR_RSLC_001",
            "ALOS2012345678-260101",
            "granule.h5",
            "scene.TIF",
            "G3315903479-ASF",
            "C2853145197-ASF",
        ] {
            assert!(product_like(name), "{name}");
        }
        for name in [
            "xS1A_abc",
            "tokenNISAR_x",
            "abc.zip.part9",
            "C2853145197-asf",
            "G-ASF",
            "sk-proj-abc123DEF456ghi",
        ] {
            assert!(!product_like(name), "{name}");
        }
    }

    /// W3n rule 5: a long hex or base64 word; product names, file names, CMR
    /// ids and the host's own ids stay.
    #[test]
    fn a_long_hex_or_base64_word_is_replaced_and_product_names_stay() {
        let hex = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            redact_words(&format!("校验 {hex} 完")),
            "校验 [local-reference] 完"
        );
        let b64 = "dGhpcyBpcyBhIHZlcnkgbG9uZyBiYXNlNjQgc2VjcmV0IHN0cmluZw==";
        assert_eq!(
            redact_words(&format!("值 {b64} 完")),
            "值 [local-reference] 完"
        );
        for kept in [
            "S1A_IW_SLC__1SDV_20261018T110831_20261018T110858_005069_009970_518E.zip",
            "S1D_IW_SLC__1SDV_20261018T110831_20261018T110858_005069_009970_518E-SLC",
            "NISAR_L1_PR_RSLC_001_030_A_019_2005_DHDH_A_20251102T120000_20251102T120030_P01101_F_N_J_001.h5",
            "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1D_IW_SLC__1SDV_20261018T110831_20261018T110858_005069_009970_518E-SLC",
            "G3315903479-ASF",
            "C2853145197-ASF",
            "batch-unified-50757268-fbb4-4679-91c6-ece9c8687443",
            "task-unified-65046dba-c55e-4a01-a49f-728eac13d277",
            "plan-1a17a903318-2",
            "LC09_L2SP_130038_20261018_20261020_02_T1_SR_B4.TIF",
        ] {
            let text = format!("产品 {kept} 完");
            assert_eq!(redact_words(&text), text, "{kept}");
        }
    }

    /// W3m item 2 (AGENT-REG-002): a task's output_root (an absolute local
    /// path the desktop now receives in the task overview) never reaches the
    /// model when a tool result carries it: the host's sanitize and the
    /// nested outbound check both replace it.
    #[test]
    fn a_task_output_root_in_a_tool_result_never_leaves() {
        let overview = json!({
            "queue": [{"task_id": "t1", "status": "queued", "output_root": "D:\\ScenePort\\downloads"}],
            "history": [{"task_id": "t0", "output_root": "C:\\Users\\张三\\Downloads"}],
        });
        let sanitized = crate::host::sanitize(&overview);
        assert_eq!(sanitized["queue"][0]["output_root"], VALUE_MARKER);
        assert_eq!(sanitized["history"][0]["output_root"], VALUE_MARKER);
        assert_eq!(sanitized["queue"][0]["task_id"], "t1");
        let nested = nested_value(&overview);
        assert_eq!(nested["queue"][0]["output_root"], VALUE_MARKER);
        let tool = Message {
            role: "tool".into(),
            content: json!({"id": "c1", "result": overview}).to_string(),
        };
        let sent = messages(&[tool]);
        assert!(
            !sent[0].content.contains("ScenePort"),
            "{}",
            sent[0].content
        );
        assert!(!sent[0].content.contains("张三"), "{}", sent[0].content);
    }

    /// The reviewers' credential forms (second review P1-B, last review), each
    /// of which main refused as a whole body: the credential never leaves,
    /// and (W3l, L4 r8 C15) the text after it on the same line stays.
    #[test]
    fn credential_variants_never_leave() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl";
        let b64 = "dXNlcjpwYXNzd29yZA==";
        let (jwt_a, jwt_b) = jwt.split_at(25);
        let (b64_a, b64_b) = b64.split_at(10);
        let m = TEXT_MARKER;
        let mut cases: Vec<(String, String)> = vec![
            // header keyword at the line end, token on the next line
            (format!("Authorization: Bearer\n{jwt}"), format!("{m}\n{m}")),
            (format!("Authorization:\nBearer {jwt}"), format!("{m}\n{m}")),
            // token with trailing punctuation: the punctuation and text stay
            (format!("BEARER {jwt}, expired"), format!("{m}, expired")),
            // two spaces (the transport scrub would leave the token)
            (format!("Bearer  {jwt}"), m.to_owned()),
            // scheme word with punctuation or at the line end
            (format!("Bearer\n{jwt}"), format!("{m}\n{m}")),
            (format!("Bearer: {jwt}"), m.to_owned()),
            (
                format!("凭据是 Basic {b64}，为什么401"),
                format!("凭据是 {m}，为什么401"),
            ),
            (format!("(Basic {b64})"), format!("{m})")),
            // header name quoted, full-width colon or with =
            (format!("Authorization=Basic {b64}"), m.to_owned()),
            (
                format!("{{\"Authorization\": \"Basic {b64}\"}}"),
                // a message that is all JSON: scrubbed by key name first
                m.to_owned(),
            ),
            (format!("Authorization：Basic {b64}"), m.to_owned()),
            // the lines around a credential stay
            (
                format!("上一行\nAuthorization: Bearer\n{jwt}\n下一行"),
                format!("上一行\n{m}\n{m}\n下一行"),
            ),
            // the token on the next line after text (the scrub would have put
            // [redacted] after Bearer)
            (
                format!("Authorization: Bearer 如下：\n{jwt}"),
                format!("{m} 如下：\n{m}"),
            ),
            // a token cut by a hard line break
            (
                format!("Authorization: Bearer {jwt_a}\n{jwt_b}"),
                format!("{m}\n{m}"),
            ),
            (
                format!("Authorization: Basic {b64_a}\n{b64_b}"),
                format!("{m}\n{m}"),
            ),
            (format!("凭据 Basic \"{b64}\""), format!("凭据 {m}\"")),
            // text after the credential on its line and on the next line
            (
                format!("请求头 Authorization: Bearer {jwt} 已过期\n第二行照常"),
                format!("请求头 {m} 已过期\n第二行照常"),
            ),
            // `of` is no credential; the transport scrub takes it as the token
            (
                "Bearer of news\n第二行".to_owned(),
                format!("{m} news\n第二行"),
            ),
            // W3l review: the word right after Bearer/Basic in a header is the
            // credential whatever its shape (backquoted, user:password, with
            // `%` or not token-like); only a tail that starts non-ASCII or is
            // closing punctuation stays.
            (
                "我的令牌是 Authorization: Bearer `abcDEF123456789xyz`，AOI 文件在 C:\\x\\aoi.geojson。问题".to_owned(),
                format!("我的令牌是 {m}`，AOI 文件在 {m}。问题"),
            ),
            (format!("Authorization: Bearer\n`{jwt}`"), format!("{m}\n{m}`")),
            ("Authorization: Basic admin:secret".to_owned(), m.to_owned()),
            ("Basic myusername1:Passw0rd!".to_owned(), m.to_owned()),
            ("Bearer ya29.a0AfB%2Fxyz789SECRETtail".to_owned(), m.to_owned()),
            ("Authorization: Bearer s3cr3t".to_owned(), m.to_owned()),
            ("Authorization: Bearer abcdefghijklmnop".to_owned(), m.to_owned()),
            ("Authorization: Bearer abc 123 456".to_owned(), format!("{m} 123 456")),
            ("Bearer s3cr3t".to_owned(), m.to_owned()),
            ("Bearer abcdefghijklmnop".to_owned(), m.to_owned()),
            // Third review P1: a full-width or Chinese opening bracket or
            // quote before the token (Chinese punctuation input mode).
            (
                format!("我的令牌是 Authorization: Bearer 【{jwt}】，帮我看看"),
                format!("我的令牌是 {m}】，帮我看看"),
            ),
            (format!("Bearer\n【{jwt}】"), format!("{m}\n{m}】")),
            // Third redaction guard: no space between the scheme and the token.
            (format!("Authorization: Bearer：{jwt}"), m.to_owned()),
            (format!("Bearer:{jwt}"), m.to_owned()),
            (format!("token:{jwt}"), m.to_owned()),
        ];
        for opening in ["【", "［", "《", "〈", "〔", "＂", "＇", "｀", "«", "·"] {
            cases.push((
                format!("Authorization: Bearer {opening}{jwt}"),
                m.to_owned(),
            ));
        }
        for (text, expected) in &cases {
            let sent = redact_words(text);
            for secret in [
                jwt,
                b64,
                jwt_a,
                jwt_b,
                b64_a,
                b64_b,
                "abcDEF123456789xyz",
                "secret",
                "Passw0rd",
                "SECRET",
                "s3cr3t",
                "abcdefghijklmnop",
            ] {
                assert!(!sent.contains(secret), "{text:?} -> {sent:?}");
            }
            let value = Value::String(sent.clone());
            assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{text:?}");
            assert_eq!(&sent, expected, "{text:?}");
        }
    }

    #[test]
    fn a_json_message_is_scrubbed_by_key_name() {
        // W3l redaction guard-2: a whole message that is JSON is scrubbed by key
        // name first, as main did; the credential pass otherwise breaks the
        // JSON and the key-name scrub never runs.
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl";
        let text = format!(
            "{{\"Authorization\": \"Bearer {jwt}\", \"Cookie\": \"sid=abc123def456\", \"password\": \"hunter2\"}}"
        );
        let sent = redact_words(&text);
        for secret in [jwt, "abc123def456", "hunter2"] {
            assert!(!sent.contains(secret), "{sent}");
        }
        let value = Value::String(sent.clone());
        assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{sent}");
        // Third review P3: a message starting with an ideographic space is
        // still JSON once trimmed, and is scrubbed as such.
        let spaced = redact_words("\u{3000}{\"password\": \"hunter2\"}");
        assert!(!spaced.contains("hunter2"), "{spaced}");
    }

    /// L4 r8 C15/C16: the credential header rule replaced the rest of the
    /// line, so the user's actual question went too (the model saw only
    /// 「我的令牌是 [local-reference]」). The credential and the path go; the
    /// question stays.
    #[test]
    fn the_r8_privacy_probe_keeps_the_question() {
        let text = "我的令牌是 Authorization: Bearer abcDEF123456789xyz，AOI 文件在 C:\\Users\\demo\\Desktop\\eostage-test\\aoi.geojson。先复述一下你从这句话里看到了什么，然后告诉我叠溪最近一景 Sentinel-1 升轨 SLC 是哪一天。";
        assert_eq!(
            redact_words(text),
            "我的令牌是 [local-reference]，AOI 文件在 [local-reference]。先复述一下你从这句话里看到了什么，然后告诉我叠溪最近一景 Sentinel-1 升轨 SLC 是哪一天。"
        );
    }

    /// A tool result reaches `messages` after the loop's `privacy::scrub`,
    /// which turns `Bearer  <jwt>` into `Bearer [redacted] <jwt>`; the value
    /// must not be sent with the token.
    #[test]
    fn a_tool_result_credential_never_leaves() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl";
        let result = privacy::scrub(&json!({"note": format!("Bearer  {jwt}")}));
        let tool = Message {
            role: "tool".into(),
            content: json!({"id": "c1", "result": result}).to_string(),
        };
        let sent = messages(&[tool]);
        assert!(!sent[0].content.contains(jwt), "{}", sent[0].content);
        assert!(
            !nested_value(&json!(format!("Basic dXNlcjpwYXNzd29yZA== x")))
                .to_string()
                .contains("dXNlcjpw")
        );
    }

    /// Second review P1-A (b6937e98 regression): a path stopped at its first
    /// non-ASCII character, so the rest of it was sent without its drive
    /// letter, which the outbound check then no longer refused
    /// (`C:\Users\张三\Desktop\x.tif` sent `张三\Desktop\x.tif`). As in
    /// message_dir, a non-ASCII letter right after a separator, or a word that
    /// still holds a separator, is part of the path.
    #[test]
    fn a_path_with_a_cjk_segment_is_replaced_to_its_end() {
        for (text, sent, gone) in [
            (
                "C:\\Users\\张三\\Desktop\\x.tif 是什么？",
                "[local-reference] 是什么？",
                "张三",
            ),
            (
                "看 C:/Users/张三/Desktop/x.tif 吧",
                "看 [local-reference] 吧",
                "张三",
            ),
            ("D:\\数据\\S1\\scene.zip", "[local-reference]", "scene"),
            ("/home/张三/data", "[local-reference]", "张三"),
        ] {
            let out = redact_words(text);
            assert_eq!(out, sent, "{text}");
            assert!(
                !out.contains(gone) && !out.contains("Desktop"),
                "{text}: {out}"
            );
        }
        // A non-ASCII word that is not part of the path ends it (message_dir's
        // rule: `D:\data里` names `D:\data`).
        assert_eq!(
            redact_words("D:\\data里的文件"),
            "[local-reference]里的文件"
        );
    }

    /// Review P2: words were cut at spaces, so the part of a path after a
    /// space was sent (`C:/Users/Zhang San/x` sent `San/x`).
    #[test]
    fn a_local_path_with_spaces_is_replaced_as_a_whole() {
        for (text, gone) in [
            ("看 C:/Users/Zhang San/x.tif 吧", "San"),
            ("看 C:\\Users\\Zhang San\\x.tif 吧", "San"),
            ("看 file:///home/zhang san/x 吧", "san"),
            ("看 /home/zhang san/x 吧", "san"),
        ] {
            let sent = redact_words(text);
            assert_eq!(sent, "看 [local-reference] 吧", "{text}");
            assert!(!sent.contains(gone));
        }
        assert_eq!(
            redact_words("see C:/Users/Zhang San/x then go\nnext line"),
            "see [local-reference]\nnext line"
        );
        // A web address is not a local path.
        assert_eq!(
            redact_words("见 https://example.org/a b"),
            "见 https://example.org/a b"
        );
    }

    /// Review P1-1: the check accepts some values only by rewriting them (a
    /// single web address loses its query or fragment). Such a value must not
    /// count as safe as it stands: a message with an ASF link kept its text,
    /// and a body refused for it is named by the part that holds it.
    #[test]
    fn a_value_the_check_rewrites_is_not_taken_as_safe() {
        let link = "https://search.asf.alaska.edu/#/?zoom=6";
        let sent = redact_words(&format!("{link}\n[context] {{\"source\":null}}"));
        assert_ne!(sent, "[已省略]");
        assert!(sent.starts_with("https://search.asf.alaska.edu/"), "{sent}");
        assert!(!sent.contains("zoom"), "{sent}");
        assert!(sent.contains("[context]"), "{sent}");
        let body = json!({"model": "m", "messages": [{"role": "user", "content": link}]});
        assert_eq!(field_class(&body), "user_message");
        let tool = json!({"input": [{"type": "function_call_output", "output": link}]});
        assert_eq!(field_class(&tool), "tool_result");
    }

    /// Review of 07866b13 (P0-1): replacing only the refused marker word sent
    /// the credential after it (`Authorization: Bearer <jwt>` became
    /// `[local-reference] [local-reference] <jwt>`), and the transport's own
    /// `Bearer` scrub no longer saw a marker to act on.
    #[test]
    fn an_authorization_header_never_leaves_with_its_credential() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl";
        let basic = "dXNlcjpwYXNzd29yZA==";
        for (label, text, secret) in [
            (
                "bearer",
                format!("请求头 Authorization: Bearer {jwt} 已过期\n第二行照常"),
                jwt,
            ),
            (
                "basic",
                format!("请求头 Authorization: Basic {basic} 已过期\n第二行照常"),
                basic,
            ),
            ("bare-bearer", format!("令牌 Bearer {jwt}"), jwt),
            ("bare-basic", format!("凭据 basic {basic}"), basic),
        ] {
            let sent = redact_words(&text);
            assert!(!sent.contains(secret), "{label}: {sent}");
            assert!(
                sent.contains("请求头") || sent.contains("令牌") || sent.contains("凭据"),
                "{label}: {sent}"
            );
            if text.contains("第二行照常") {
                assert!(
                    sent.contains("\n第二行照常"),
                    "{label}: only that line is replaced: {sent}"
                );
            }
            let value = Value::String(sent.clone());
            assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{label}");
        }
        // An ordinary use of the word keeps its text.
        assert_eq!(redact_words("a basic question"), "a basic question");
    }

    #[test]
    fn unsafe_words_are_replaced_and_safe_text_is_kept() {
        assert_eq!(redact_words("找最新一景"), "找最新一景");
        assert_eq!(
            redact_words("看看 C:/Users/x/a.tif 吧"),
            "看看 [local-reference] 吧"
        );
        assert_eq!(redact_words("/help 一下"), "[local-reference] 一下");
        // A refused marker word goes with the word after it (second review
        // P1-B: that word may be the credential).
        assert_eq!(
            redact_words("I am a BEARER of news"),
            "I am a [local-reference] news"
        );
    }

    /// Review 2026-11-02: a Unix path glued to Chinese text and a `~/`
    /// path left the machine as written. They are replaced whole, on every
    /// platform.
    #[test]
    fn unix_paths_glued_to_chinese_and_tilde_paths_are_replaced() {
        for (text, sent, gone) in [
            (
                "下载到/data/me 里",
                "下载到[local-reference] 里",
                "/data/me",
            ),
            ("下载到/mnt/x", "下载到[local-reference]", "/mnt/x"),
            ("下载到 /data 里", "下载到 [local-reference] 里", "/data"),
            ("下载到/home/张三/下载", "下载到[local-reference]", "张三"),
            (
                "存到/media/backup 里",
                "存到[local-reference] 里",
                "/media/backup",
            ),
            ("下载到/root/x里", "下载到[local-reference]里", "/root/x"),
            (
                "路径：/srv/geo；帮忙",
                "路径：[local-reference]；帮忙",
                "/srv/geo",
            ),
            ("~/data", "[local-reference]", "~/data"),
            ("下载到 ~/out", "下载到 [local-reference]", "~/out"),
            ("./x", "[local-reference]", "./x"),
            (
                r#"存到 "/var/tmp/out""#,
                "存到 \"[local-reference]\"",
                "/var/tmp/out",
            ),
            ("(/etc/config)", "([local-reference])", "/etc/config"),
            ("/data/S1.zip。", "[local-reference]。", "/data/S1.zip"),
            (
                "下载到/var/tmp/out",
                "下载到[local-reference]",
                "/var/tmp/out",
            ),
        ] {
            let out = redact_words(text);
            assert_eq!(out, sent, "{text}");
            assert!(!out.contains(gone), "{text}: {out}");
            let value = Value::String(out.clone());
            assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{text}");
        }
    }

    /// Review 2026-11-02: the words, dates and addresses that merely hold
    /// a `/` stay as written.
    #[test]
    fn slash_separated_words_and_dates_stay() {
        for text in [
            "见 https://a/b 下载",
            "and/or 一下",
            "TCP/IP 配置",
            "km/h 速度",
            "比例 1/2",
            "math.pi/2 计算",
            "日期 2026/11/02",
            "24/7",
            "a/b/c",
            "“/ home”",
            "//server",
            "就一个 / 字符",
            "入射角 38.7°/ 45.0° 区间",
            "入射角 38.7°/45.0° 区间",
            "38.7°/N 45.0°/E",
            "/ 45.0°",
            "/45.0°",
            "支持 查看/下载 与 升轨/降轨 切换",
            "光学/雷达 双源协同",
            "地表形变速率 12.5 cm/yr，波速 3.2 km/s",
            "/s",
            "/yr",
            r"干涉相位公式 \(\Delta\phi = \frac{4\pi}{\lambda} d \sin\theta_i\) 与矩阵 \[\begin{bmatrix} 1 & 0 \\ 0 & 1 \end{bmatrix}\]",
        ] {
            assert_eq!(redact_words(text), text, "{text}");
            let value = Value::String(text.to_owned());
            assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{text}");
        }
        // `他/她` is replaced (2026-11-02 privacy-first ruling: a `/` before
        // a non-ASCII letter reads as a single-segment path, and the false
        // positive is accepted so `下载到/数据` cannot leave the machine).
        assert_eq!(redact_words("他/她"), "他[local-reference]");
    }

    /// Second review, P1: a single-segment CJK path glued to Chinese text
    /// (`下载到/数据`) left the machine as written. Privacy comes first:
    /// every layer takes it.
    #[test]
    fn single_segment_cjk_paths_glued_to_text_are_replaced() {
        for (text, sent, gone) in [
            ("下载到/数据", "下载到[local-reference]", "/数据"),
            ("保存到/下载", "保存到[local-reference]", "/下载"),
            ("在/桌面", "在[local-reference]", "/桌面"),
            ("下载到/数据x", "下载到[local-reference]", "/数据x"),
            ("下载到/数据.zip", "下载到[local-reference]", "/数据.zip"),
        ] {
            let out = redact_words(text);
            assert_eq!(out, sent, "{text}");
            assert!(!out.contains(gone), "{text}: {out}");
            let value = Value::String(out.clone());
            assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{text}");
            // The outbound check also refuses the raw text.
            let raw = Value::String(text.to_owned());
            assert!(crate::run::safe_value(&raw).is_err(), "{text}");
        }
        // The transport's own scrub replaces the whole token (its
        // granularity, as for any path glued to the text before it).
        assert_eq!(
            privacy::scrub_text("下载到/数据 里"),
            "[local-reference] 里"
        );
    }

    /// Second review, P2 (third review: UTF-8 lead bytes included): a
    /// URL-encoded path (`%2Fhome%2Fme%2Fx`, `%2F%E4%B8%8B%E8%BD%BD`) left
    /// the machine as written. `%2F` before an ASCII letter or a `%C2`–`%F4`
    /// lead byte is a path; a digit (`?a=1%2F2`), a space (`%2F%20`) or
    /// another `/` (`%2F%2F`) after the code is not.
    #[test]
    fn url_encoded_paths_are_replaced() {
        for text in [
            "下载到%2Fhome%2Fme%2Fx 里",
            "看 %2Ftmp/out",
            "下载到%2fHome 里",
            "下载到%2F%E4%B8%8B%E8%BD%BD 里",
            "保存到%2f%e4%b8%8b",
        ] {
            let sent = redact_words(text);
            assert!(!sent.contains("home"), "{text}: {sent}");
            assert!(!sent.contains("Home"), "{text}: {sent}");
            assert!(!sent.contains("%2F"), "{text}: {sent}");
            assert!(!sent.contains("%2f"), "{text}: {sent}");
            assert!(!sent.contains("%E4"), "{text}: {sent}");
            assert!(!sent.contains("%e4"), "{text}: {sent}");
            let value = Value::String(sent.clone());
            assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{text}");
            let raw = Value::String(text.to_owned());
            assert!(crate::run::safe_value(&raw).is_err(), "{text}");
        }
        assert_eq!(redact_words("查询 ?a=1%2F2 的结果"), "查询 ?a=1%2F2 的结果");
        let value = Value::String("?a=1%2F2".to_owned());
        assert_eq!(crate::run::safe_value(&value).ok(), Some(value));
        assert_eq!(
            privacy::scrub_text("?a=1%2F2"),
            "?a=1%2F2",
            "a digit after the code is not a path"
        );
        // A space or another `/` after the code is not a path either.
        assert_eq!(
            redact_words("查询 %2F%20x 和 %2F%2Fy"),
            "查询 %2F%20x 和 %2F%2Fy"
        );
        let spaced = Value::String("%2F%20x".to_owned());
        assert_eq!(crate::run::safe_value(&spaced).ok(), Some(spaced));
        let doubled = Value::String("%2F%2Fy".to_owned());
        assert_eq!(crate::run::safe_value(&doubled).ok(), Some(doubled));
        assert_eq!(privacy::scrub_text("%2F%20x"), "%2F%20x");
        assert_eq!(privacy::scrub_text("%2F%2Fy"), "%2F%2Fy");
        // The transport's own scrub replaces the whole token (its
        // granularity, as for any path glued to the text before it).
        assert_eq!(
            privacy::scrub_text("下载到%2Fhome%2Fme%2Fx"),
            "[local-reference]"
        );
        assert_eq!(
            privacy::scrub_text("下载到%2F%E4%B8%8B%E8%BD%BD"),
            "[local-reference]"
        );
        assert_eq!(
            privacy::scrub_text("保存到%2f%e4%b8%8b"),
            "[local-reference]"
        );
    }

    /// Second review, P3: another user's home (`~user/x`) left the machine
    /// as written.
    #[test]
    fn another_users_home_directory_is_replaced() {
        for text in [
            "下载到~zhangsan/out 里",
            "保存到~root.dir/x_data",
            "看 ~user.b-x/a 吧",
        ] {
            let sent = redact_words(text);
            assert!(!sent.contains("~"), "{text}: {sent}");
            assert!(!sent.contains("zhangsan"), "{text}: {sent}");
            assert!(!sent.contains("root.dir"), "{text}: {sent}");
            assert!(!sent.contains("user.b-x"), "{text}: {sent}");
            let value = Value::String(sent.clone());
            assert_eq!(crate::run::safe_value(&value).ok(), Some(value), "{text}");
        }
        assert_eq!(
            redact_words("下载到 ~zhangsan/out 里"),
            "下载到 [local-reference] 里"
        );
        assert_eq!(
            privacy::scrub_text("下载到~zhangsan/out 里"),
            "[local-reference] 里"
        );
        // `~` without a following `/` is not a path.
        assert_eq!(redact_words("大约 ~30 个"), "大约 ~30 个");
    }

    #[test]
    fn a_value_that_reads_as_a_path_once_nested_keeps_its_text() {
        // ":" before a quote or a line break escapes to ":\" / ":\n".
        assert_eq!(nested_value(&json!("错误:\n重试")), json!("错误: \n重试"));
        assert_eq!(nested_value(&json!("a:\"b\"")), json!("a: \"b\""));
        assert_eq!(nested_value(&json!("D:\\out")), json!(VALUE_MARKER));
        assert_eq!(nested_value(&json!("plain")), json!("plain"));
        let nested = nested_value(&json!({"message": "x:\"y\"", "access_token": "z"}));
        assert_eq!(nested, json!({"message": "x: \"y\""}));
        let sent = Value::String(nested.to_string());
        assert!(crate::run::safe_value(&sent).is_ok());
    }

    #[test]
    fn the_refused_part_of_a_body_is_named_by_class() {
        let chat = |message: Value| json!({"model": "m", "messages": [{"role": "system", "content": "s"}, message]});
        assert_eq!(
            field_class(&chat(json!({"role": "user", "content": "C:/x"}))),
            "user_message"
        );
        assert_eq!(
            field_class(&chat(
                json!({"role": "tool", "tool_call_id": "1", "content": "{\"a\":\"D:\\\\x\"}"})
            )),
            "tool_result"
        );
        assert_eq!(
            field_class(&chat(json!({"role": "assistant", "content": null,
                "tool_calls": [{"id": "1", "function": {"name": "n", "arguments": "{\"p\":\"D:\\\\x\"}"}}]}))),
            "tool_call"
        );
        assert_eq!(
            field_class(&chat(json!({"role": "assistant", "content": "见 E:/x"}))),
            "assistant_message"
        );
        assert_eq!(
            field_class(&json!({"model": "m", "metadata": {"note": "C:/x"}, "messages": []})),
            "request_envelope"
        );
        assert_eq!(
            field_class(&json!({"input": [{"type": "function_call_output", "output": "C:/x"}]})),
            "tool_result"
        );
    }

    #[test]
    fn tool_result_nested_json_and_compact_urls_do_not_false_block_outbound() {
        // Reproduces Item 15a:
        // (a) nested JSON containing `:"` after privacy::scrub must NOT re-compact into `:\"`
        // (b) compact JSON tool result containing a clean `https://` URL alongside `?` or `#`
        //     in another field or after `\n` (e.g. SKILL.md) must NOT trip `run::safe_value`.
        let msgs = messages(&[Message {
            role: "tool".into(),
            content: json!({
                "id": "call-1",
                "result": {
                    "ok": true,
                    "detail": "状态:\"运行中\"",
                    "docs_url": "https://asf.alaska.edu/nisar",
                    "question": "为什么缓存命中率低?",
                    "markdown": "参考 https://asf.alaska.edu/nisar\n# 标题"
                }
            })
            .to_string(),
        }]);
        let route = insar_agent_model::transport::ModelRoute::new(
            insar_agent_model::Protocol::Responses,
            "https://chatgpt.com/backend-api/codex",
            "gpt-5-codex",
        )
        .unwrap();
        let (_, body) = route.request(&msgs, &[]).unwrap();
        assert_eq!(
            crate::run::safe_value(&body).ok().as_ref(),
            Some(&body),
            "clean tool result must not falsely trigger outbound_blocked: {body}"
        );

        // Signed/credential URLs inside compact JSON MUST STILL be blocked!
        assert!(crate::run::safe_value(&json!(
            "{\"url\":\"https://asf.alaska.edu/nisar?sig=abc\"}"
        ))
        .is_err());
        assert!(crate::run::safe_value(&json!(
            "{\"url\":\"https://user:pass@asf.alaska.edu/nisar\"}"
        ))
        .is_err());
    }

    #[test]
    fn multimodal_user_message_passes_safe_value_with_valid_png_and_blocks_spoofed_secret() {
        let valid_png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let fake_b64 = "c2stcHJvai1mYWtlMTIzNDU2Nzg5MA==";
        let msgs = messages(&[Message {
            role: "user_multimodal".into(),
            content: json!({
                "text": "帮我看图，密钥 sk-proj-ABC1234567890xyz 和 /etc/passwd 不得外泄",
                "images": [
                    {"mime_type": "image/png", "data_base64": valid_png_b64},
                    {"mime_type": "image/png", "data_base64": fake_b64}
                ]
            })
            .to_string(),
        }]);
        for proto in [
            insar_agent_model::Protocol::ChatCompletions,
            insar_agent_model::Protocol::Responses,
            insar_agent_model::Protocol::AnthropicMessages,
            insar_agent_model::Protocol::GoogleGemini,
        ] {
            let route = insar_agent_model::transport::ModelRoute::new(
                proto,
                "https://example.invalid/v1",
                "m",
            )
            .unwrap();
            let (_, body) = route.request(&msgs, &[]).unwrap();
            assert_eq!(
                crate::run::safe_value(&body).ok().as_ref(),
                Some(&body),
                "verified multimodal image must pass safe_value on {proto:?}: {body}"
            );
            let serialized = body.to_string();
            assert!(serialized.contains(valid_png_b64));
            assert!(!serialized.contains(fake_b64));
            assert!(!serialized.contains("sk-proj-ABC1234567890xyz"));
            assert!(!serialized.contains("/etc/passwd"));
        }
    }
}
