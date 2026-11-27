//! Opt-in response-envelope diagnosis, separate from delivery acceptance.
//! Uses the native credential reader and explicit proxy/direct HTTP clients.
//! No task Store, download-request, signed URL, raw response or secret is logged.
use reqwest::{
    blocking::{Body, Client},
    header::{HeaderValue, CONTENT_TYPE},
    redirect::Policy,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

const ORIGIN: &str = "https://m2m.cr.usgs.gov/api/api/json/stable/";
const MAX_RESPONSE: u64 = 512 * 1024;
type Check<T> = Result<T, &'static str>;

struct SecretReader {
    bytes: Zeroizing<Vec<u8>>,
    offset: usize,
}
impl Read for SecretReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = buffer.len().min(self.bytes.len() - self.offset);
        buffer[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}
struct Reply {
    status: u16,
    kind: &'static str,
    value: Value,
}
fn erase(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(erase),
        Value::Object(items) => items.values_mut().for_each(erase),
        _ => {}
    }
}
impl Drop for Reply {
    fn drop(&mut self) {
        erase(&mut self.value);
    }
}
impl Reply {
    fn ok(&self) -> bool {
        self.status == 200 && self.value.get("errorCode").is_some_and(Value::is_null)
    }
}
fn post(
    client: &Client,
    endpoint: &str,
    payload: &impl Serialize,
    key: Option<&str>,
) -> Check<Reply> {
    if !matches!(
        endpoint,
        "login-token" | "permissions" | "download-options" | "logout"
    ) {
        return Err("ENDPOINT_NOT_ALLOWED");
    }
    let mut bytes = Zeroizing::new(Vec::new());
    serde_json::to_writer(&mut *bytes, payload).map_err(|_| "REQUEST_ENCODING")?;
    let length = bytes.len() as u64;
    let mut request = client
        .post(format!("{ORIGIN}{endpoint}"))
        .header("Accept", "application/json")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::sized(SecretReader { bytes, offset: 0 }, length));
    if let Some(key) = key {
        let mut header = HeaderValue::from_str(key).map_err(|_| "KEY_HEADER_INVALID")?;
        header.set_sensitive(true);
        request = request.header("X-Auth-Token", header);
    }
    let response = request.send().map_err(|e| {
        if e.is_timeout() {
            "TIMEOUT"
        } else {
            "TRANSPORT_FAILED"
        }
    })?;
    let status = response.status().as_u16();
    let kind = match response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim)
    {
        Some("application/json") => "json",
        Some("text/html") => "html",
        _ => "other",
    };
    if response.content_length().is_some_and(|n| n > MAX_RESPONSE) {
        return Err("RESPONSE_TOO_LARGE");
    }
    let mut bytes = Zeroizing::new(Vec::new());
    response
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "RESPONSE_READ")?;
    if bytes.len() as u64 > MAX_RESPONSE {
        return Err("RESPONSE_TOO_LARGE");
    }
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(Reply {
        status,
        kind,
        value,
    })
}
fn safe_option_text(value: Option<&Value>, maximum: usize) -> Option<&str> {
    value.and_then(Value::as_str).filter(|text| {
        !text.is_empty()
            && text.len() <= maximum
            && text
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    })
}

fn projection(step: &str, reply: &Reply) -> Value {
    // Only closed protocol names leave memory. Unknown data is counted, not echoed.
    let code = match reply.value.get("errorCode").and_then(Value::as_str) {
        Some("AUTH_UNAUTHORIZED") => Some("AUTH_UNAUTHORIZED"),
        Some("AUTH_INVALID") => Some("AUTH_INVALID"),
        Some("AUTH_KEY_INVALID") => Some("AUTH_KEY_INVALID"),
        Some("AUTH_EXPIRED") => Some("AUTH_EXPIRED"),
        Some("AUTH_REQUIRED") => Some("AUTH_REQUIRED"),
        Some("PERMISSION_DENIED") => Some("PERMISSION_DENIED"),
        Some("FORBIDDEN") => Some("FORBIDDEN"),
        Some("DATASET_NOT_FOUND") => Some("DATASET_NOT_FOUND"),
        Some("INPUT_ERROR") => Some("INPUT_ERROR"),
        Some("INPUT_VALIDATION_ERROR") => Some("INPUT_VALIDATION_ERROR"),
        Some(_) => Some("UNRECOGNIZED_CODE_REDACTED"),
        None => None,
    };
    let message = Zeroizing::new(
        reply
            .value
            .get("errorMessage")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase(),
    );
    let mut result = json!({"http_status":reply.status,"response_type":reply.kind,
        "json_object":reply.value.is_object(),"envelope_ok":reply.ok(),"business_error_code":code,
        "message_mentions_permission":message.contains("permission"),
        "message_mentions_download":message.contains("download"),
        "message_mentions_request_access":message.contains("request access"),
        "message_says_not_authorized":message.contains("not authorized") || message.contains("not authorised"),
        "raw_body_retained":false});
    if step == "permissions" {
        let values = reply.value.get("data").and_then(Value::as_array);
        let names: Vec<&str> = values
            .into_iter()
            .flatten()
            .filter_map(|value| match value.as_str() {
                Some("user") => Some("user"),
                Some("download") => Some("download"),
                Some("order") => Some("order"),
                _ => None,
            })
            .collect();
        result["permission_names"] = json!(names);
        result["permission_count"] = json!(values.map(Vec::len));
        result["unrecognized_permission_count"] = json!(values.map(|v| v.len() - names.len()));
    }
    if step == "download_options" {
        let rows = reply.value.get("data").and_then(Value::as_array);
        let options = rows
            .into_iter()
            .flatten()
            .take(128)
            .filter_map(Value::as_object)
            .map(|record| {
                json!({
                    "id": safe_option_text(record.get("id"), 96),
                    "product_name": safe_option_text(record.get("productName"), 160),
                    "product_code": safe_option_text(record.get("productCode"), 64),
                    "download_system": safe_option_text(record.get("downloadSystem"), 64),
                    "available": record.get("available").and_then(Value::as_bool),
                    "bulk_available": record.get("bulkAvailable").and_then(Value::as_bool),
                    "filesize": record.get("filesize").and_then(Value::as_u64),
                    "download_name_present": record.get("downloadName").is_some_and(|value| !value.is_null()),
                    "checksum_algorithms": record
                        .get("checksum")
                        .and_then(Value::as_array)
                        .map(|values| values.iter().filter_map(|value| {
                            safe_option_text(value.as_object()?.get("id"), 32)
                        }).collect::<Vec<_>>()),
                    "secondary_count": record
                        .get("secondaryDownloads")
                        .and_then(Value::as_array)
                        .map(Vec::len),
                    "file_groups_present": record.get("fileGroups").is_some_and(|value| !value.is_null()),
                })
            })
            .collect::<Vec<_>>();
        result["option_count"] = json!(rows.map(Vec::len));
        result["options"] = json!(options);
    }
    result
}
#[derive(Serialize)]
struct Login<'a> {
    username: &'a str,
    token: &'a str,
}

#[test]
#[ignore = "explicit Owner authorization, isolated report and native saved credentials required"]
fn usgs_route_response_envelope_probe() {
    assert_eq!(std::env::var("EOSTAGE_USGS_WIRE_PROBE").as_deref(), Ok("1"));
    let route = std::env::var("EOSTAGE_USGS_WIRE_ROUTE").expect("explicit route");
    let path = std::path::PathBuf::from(
        std::env::var_os("EOSTAGE_USGS_WIRE_REPORT").expect("unique report"),
    );
    assert!(path.is_absolute());
    let mut report = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .expect("new report only");
    let started = Instant::now();
    let mut emit = |step: &str, detail: Value| {
        let value = json!({"step":step,"elapsed_ms":started.elapsed().as_millis(),"detail":detail});
        writeln!(report, "{value}").expect("safe evidence write");
        report.sync_data().expect("safe evidence sync");
        println!("USGS_WIRE {value}");
    };
    let builder = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .referer(false);
    let builder = match route.as_str() {
        "direct" => builder,
        "proxy-7897" => builder
            .proxy(reqwest::Proxy::all("http://127.0.0.1:7897").expect("fixed loopback proxy")),
        _ => panic!("unsupported route"),
    };
    let client = builder.build().expect("client configuration");
    emit(
        "route",
        json!({"route":route,"explicit_client_route":true,"settings_changed":false}),
    );
    let mut api_key: Option<Zeroizing<String>> = None;
    let outcome: Check<()> = (|| {
        let (username, token) = insar_download::resolve_desktop_usgs_login()
            .map_err(|_| "NATIVE_CREDENTIAL_UNAVAILABLE")?;
        let mut reply = post(
            &client,
            "login-token",
            &Login {
                username: &username,
                token: &token,
            },
            None,
        )?;
        emit("login", projection("login", &reply));
        if !reply.ok() {
            return Err("LOGIN_REJECTED");
        }
        let key = match reply.value.get_mut("data").map(Value::take) {
            Some(Value::String(key)) => Zeroizing::new(key),
            _ => return Err("LOGIN_ENVELOPE_INVALID"),
        };
        if key.is_empty() || key.len() > 16 * 1024 || key.chars().any(char::is_control) {
            return Err("KEY_INVALID");
        }
        api_key = Some(key);
        drop(username);
        drop(token);
        drop(reply);
        let key = api_key.as_deref().map(String::as_str);
        let permissions = post(&client, "permissions", &Value::Null, key)?;
        emit("permissions", projection("permissions", &permissions));
        if !permissions.ok() {
            return Err("PERMISSIONS_REJECTED");
        }
        // Same entity/product route as production, with an optional explicit
        // Owner-selected entity for a read-only compatibility diagnosis.
        let entity_id = std::env::var("EOSTAGE_USGS_WIRE_ENTITY_ID")
            .unwrap_or_else(|_| "LC90150362023083LGN00".to_owned());
        if entity_id.is_empty()
            || entity_id.len() > 64
            || !entity_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err("ENTITY_ID_INVALID");
        }
        let options = post(
            &client,
            "download-options",
            &json!({"datasetName":"landsat_ot_c2_l2","entityIds":[entity_id]}),
            key,
        )?;
        emit("download_options", projection("download_options", &options));
        if !options.ok() {
            return Err("DOWNLOAD_OPTIONS_REJECTED");
        }
        Ok(())
    })();
    let mut logout_ok = false;
    if let Some(key) = api_key.as_deref() {
        match post(&client, "logout", &Value::Null, Some(key)) {
            Ok(reply) => {
                logout_ok = reply.ok();
                emit("logout", projection("logout", &reply));
            }
            Err(code) => emit("logout", json!({"transport_code":code})),
        }
    }
    drop(api_key);
    emit(
        "closed",
        json!({"outcome":outcome.as_ref().err(),"logout_http_confirmed":logout_ok,
        "payload_bytes":0,"download_requests":0,"key_dropped":true}),
    );
    assert!(
        outcome.is_ok(),
        "USGS_RESPONSE_PROBE_STOPPED:{}",
        outcome.unwrap_err()
    );
}

#[test]
fn response_projection_never_echoes_unknown_codes_or_secret_fields() {
    let reply = Reply {
        status: 403,
        kind: "json",
        value: json!({"errorCode":"DO_NOT_ECHO_TOKEN",
        "errorMessage":"DO_NOT_ECHO_ACCOUNT does not have download permission",
        "data":["user","DO_NOT_ECHO_UNKNOWN"],"requestId":"DO_NOT_ECHO_ID"}),
    };
    let result = projection("permissions", &reply);
    let encoded = result.to_string();
    assert!(!encoded.contains("DO_NOT_ECHO"));
    assert_eq!(result["permission_names"], json!(["user"]));
    assert_eq!(result["unrecognized_permission_count"], 1);
    assert_eq!(result["message_mentions_permission"], true);
    assert_eq!(result["business_error_code"], "UNRECOGNIZED_CODE_REDACTED");
    let download_options = Reply {
        status: 200,
        kind: "json",
        value: json!({
            "errorCode": null,
            "errorMessage": null,
            "data": [{
                "id": "5e83d14fec7cae84",
                "productName": "Landsat Collection 2 Level-2 Product Bundle",
                "productCode": "D694",
                "downloadSystem": "ls_zip",
                "available": false,
                "bulkAvailable": true,
                "secondaryDownloads": [{"url":"DO_NOT_ECHO_URL"}],
                "fileGroups": null,
                "url": "DO_NOT_ECHO_URL",
                "secret": "DO_NOT_ECHO_SECRET"
            }]
        }),
    };
    let option_projection = projection("download_options", &download_options);
    let option_encoded = option_projection.to_string();
    assert_eq!(option_projection["option_count"], 1);
    assert_eq!(option_projection["options"][0]["id"], "5e83d14fec7cae84");
    assert_eq!(option_projection["options"][0]["available"], false);
    assert_eq!(option_projection["options"][0]["bulk_available"], true);
    assert_eq!(option_projection["options"][0]["filesize"], Value::Null);
    assert_eq!(
        option_projection["options"][0]["download_name_present"],
        false
    );
    assert_eq!(
        option_projection["options"][0]["checksum_algorithms"],
        Value::Null
    );
    assert_eq!(option_projection["options"][0]["secondary_count"], 1);
    assert!(!option_encoded.contains("DO_NOT_ECHO"));
}
