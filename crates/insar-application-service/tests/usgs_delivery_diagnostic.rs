//! Explicit opt-in sequential diagnosis through the production USGS clients.
//! No Owner Store, no credential output, no signed URL or response-body logging.
use insar_core::ProviderNetworkPolicy;
use insar_download::resolve_desktop_usgs_login;
use insar_provider::usgs_m2m::{
    UsgsM2mApplicationToken, UsgsM2mDownloadClient, UsgsM2mDownloadRequestOutcome, UsgsM2mError,
    UsgsM2mLoginMaterial, UsgsM2mSceneSearchClient, UsgsM2mSession,
};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn safe_error(error: &UsgsM2mError) -> Value {
    json!({
        "code":error.code_str(),
        "endpoint":error.endpoint().as_str(),
        "http_status":error.http_status(),
        "upstream_error_code":error.remote_code(),
        "upstream_error_message":error.remote_message(),
        "secret_material_redacted":error.remote_message_redacted(),
        "transport_error":error.transport_message(),
        "transport_secret_material_redacted":error.transport_message_redacted(),
        "detail":error.detail()
    })
}

#[test]
#[ignore = "explicit Owner authorization and native saved credentials required"]
fn usgs_sequential_delivery_diagnostic() {
    assert_eq!(std::env::var("EOSTAGE_USGS_DIAG").as_deref(), Ok("1"));
    let output =
        PathBuf::from(std::env::var_os("EOSTAGE_USGS_DIAG_REPORT").expect("unique report"));
    assert!(output.is_absolute());
    let mut report = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)
        .expect("new report only");
    let began = Instant::now();
    let cancelled = || began.elapsed() > Duration::from_secs(100);
    let mut emit = |step: &str, detail: Value| {
        let record = json!({"step":step,"elapsed_ms":began.elapsed().as_millis(),"detail":detail});
        writeln!(report, "{record}").unwrap();
        report.sync_data().unwrap();
        println!("USGS_DIAG {record}");
    };
    let route = std::env::var("EOSTAGE_USGS_DIAG_ROUTE").unwrap_or_else(|_| "system".into());
    let policy = match route.as_str() {
        "system" => ProviderNetworkPolicy::system(),
        "direct" => ProviderNetworkPolicy::direct(),
        _ => panic!("unsupported diagnostic route"),
    };
    emit(
        "route",
        json!({"route":route,"application_settings_changed":false}),
    );
    let session = Arc::new(UsgsM2mSession::new(policy, Duration::from_secs(20)).unwrap());
    let (mut username, mut token) =
        resolve_desktop_usgs_login().expect("native saved USGS credentials unavailable");
    let material = UsgsM2mLoginMaterial::new(
        std::mem::take(&mut *username),
        UsgsM2mApplicationToken::new(std::mem::take(&mut *token)).unwrap(),
    )
    .unwrap();
    let result: Result<(), UsgsM2mError> = (|| {
        let login = session.login(material, &cancelled);
        emit(
            "login",
            match &login {
                Ok(_) => json!({"ok":true}),
                Err(e) => safe_error(e),
            },
        );
        login?;
        let permissions = session.permissions(&cancelled);
        emit(
            "permissions",
            match &permissions {
                Ok(p) => {
                    json!({"ok":true,"user":p.has_user(),"download":p.has_download(),"order":p.has_order()})
                }
                Err(e) => safe_error(e),
            },
        );
        permissions?;
        let product = UsgsM2mSceneSearchClient::new(Arc::clone(&session))
            .exact_product("LC09_L2SP_015036_20230324_20230326_02_T1", &cancelled);
        emit(
            "exact_product",
            match &product {
                Ok(p) => json!({"ok":true,"product":p.display_name,"entity_id":p.product_id}),
                Err(e) => safe_error(e),
            },
        );
        let product = product?;
        let client = UsgsM2mDownloadClient::new(Arc::clone(&session));
        let option = client.full_bundle_option(
            "landsat_ot_c2_l2",
            &product.product_id,
            &product.display_name,
            &cancelled,
        );
        emit(
            "download_options",
            match &option {
                Ok(o) => {
                    json!({"ok":true,"availability":o.availability.as_str(),"expected_bytes":o.expected_size_bytes,"checksum_algorithm":o.checksum_algorithm})
                }
                Err(e) => safe_error(e),
            },
        );
        let option = option?;
        let label = format!("sp-{}", uuid::Uuid::new_v4().simple());
        let requested = client.request_full_bundle(
            &option.dataset_alias,
            &option.entity_id,
            &option.display_id,
            &option.option_id,
            &label,
            &cancelled,
        );
        emit(
            "download_request",
            match &requested {
                Ok(UsgsM2mDownloadRequestOutcome::Available(value)) => {
                    let parsed = reqwest::Url::parse(value.runtime_url()).ok();
                    let path = parsed.as_ref().map(reqwest::Url::path).unwrap_or("");
                    let probe = reqwest::blocking::Client::builder()
                        .no_proxy()
                        .redirect(reqwest::redirect::Policy::limited(4))
                        .timeout(Duration::from_secs(30))
                        .build()
                        .and_then(|client| {
                            client
                                .get(value.runtime_url())
                                .header(reqwest::header::RANGE, "bytes=0-0")
                                .send()
                        });
                    let probe_detail = match probe {
                        Ok(response) => json!({
                            "ok":true,
                            "status":response.status().as_u16(),
                            "final_host":response.url().host_str(),
                            "content_length":response.content_length(),
                            "content_range":response
                                .headers()
                                .get(reqwest::header::CONTENT_RANGE)
                                .and_then(|value| value.to_str().ok()),
                        }),
                        Err(_) => json!({"ok":false,"transport_failed":true}),
                    };
                    json!({
                        "ok":true,
                        "state":"available",
                        "option_expected_bytes":option.expected_size_bytes,
                        "runtime_expected_bytes":value.expected_size_bytes(),
                        "runtime_checksum_algorithm":value.checksum().map(|value| value.0),
                        "payload_probe":probe_detail,
                        "access_url_omitted":true,
                        "url_shape":{
                            "scheme":parsed.as_ref().map(reqwest::Url::scheme),
                            "host":parsed.as_ref().and_then(reqwest::Url::host_str),
                            "port":parsed.as_ref().and_then(reqwest::Url::port_or_known_default),
                            "userinfo":parsed.as_ref().is_some_and(|url| !url.username().is_empty() || url.password().is_some()),
                            "fragment":parsed.as_ref().is_some_and(|url| url.fragment().is_some()),
                            "query_present":parsed.as_ref().is_some_and(|url| url.query().is_some()),
                            "path_starts_data":path.starts_with("/data/"),
                            "path_starts_download":path.starts_with("/download/"),
                            "path_segment_count":path.split('/').filter(|segment| !segment.is_empty()).count(),
                            "tar_suffix":path.to_ascii_lowercase().ends_with(".tar")
                        }
                    })
                }
                Ok(UsgsM2mDownloadRequestOutcome::Preparing(_)) => {
                    json!({"ok":true,"state":"preparing"})
                }
                Err(e) => safe_error(e),
            },
        );
        requested?;
        Ok(())
    })();
    let before_logout = format!("{:?}", session.request_counts());
    let logout = session.logout(&|| false);
    emit(
        "closed",
        json!({"ok":result.is_ok(),"logout_ok":logout.is_ok(),
        "logout_error":logout.as_ref().err().map(safe_error),"counts":before_logout,"payload_bytes":0}),
    );
    if let Err(error) = result {
        panic!(
            "USGS_DIAGNOSTIC_STOPPED:{}:{}",
            error.endpoint().as_str(),
            error.code_str()
        );
    }
}

#[test]
#[ignore = "explicit Owner authorization and native saved credentials required"]
fn usgs_two_scene_readonly_probe() {
    assert_eq!(std::env::var("EOSTAGE_USGS_DIAG").as_deref(), Ok("1"));
    let output =
        PathBuf::from(std::env::var_os("EOSTAGE_USGS_DIAG_REPORT").expect("unique report"));
    assert!(output.is_absolute());
    let mut report = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)
        .expect("new report only");
    let began = Instant::now();
    let cancelled = || began.elapsed() > Duration::from_secs(80);
    let mut emit = |step: &str, detail: Value| {
        let record = json!({"step":step,"elapsed_ms":began.elapsed().as_millis(),"detail":detail});
        writeln!(report, "{record}").unwrap();
        report.sync_data().unwrap();
        println!("USGS_READONLY {record}");
    };
    let session = Arc::new(
        UsgsM2mSession::new(ProviderNetworkPolicy::direct(), Duration::from_secs(20)).unwrap(),
    );
    let (mut username, mut token) =
        resolve_desktop_usgs_login().expect("native saved USGS credentials unavailable");
    let material = UsgsM2mLoginMaterial::new(
        std::mem::take(&mut *username),
        UsgsM2mApplicationToken::new(std::mem::take(&mut *token)).unwrap(),
    )
    .unwrap();
    let result: Result<(), UsgsM2mError> = (|| {
        session.login(material, &cancelled)?;
        let permissions = session.permissions(&cancelled)?;
        emit(
            "permissions",
            json!({"user":permissions.has_user(),"download":permissions.has_download(),"order":permissions.has_order()}),
        );
        let client = UsgsM2mDownloadClient::new(Arc::clone(&session));
        let display_ids = std::env::var("EOSTAGE_USGS_DIAG_SCENES")
            .ok()
            .map(|value| {
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|values| !values.is_empty())
            .unwrap_or_else(|| {
                vec![
                    "LC09_L2SP_015035_20230324_20230326_02_T1".to_owned(),
                    "LC09_L2SP_015036_20230324_20230326_02_T1".to_owned(),
                ]
            });
        assert!((1..=8).contains(&display_ids.len()));
        assert!(display_ids.iter().all(|value| {
            value.len() <= 96
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        }));
        for display_id in &display_ids {
            let product = UsgsM2mSceneSearchClient::new(Arc::clone(&session))
                .exact_product(display_id, &cancelled);
            emit(
                "exact_product",
                match &product {
                    Ok(product) => {
                        json!({"ok":true,"display_id":display_id,"entity_id":product.product_id})
                    }
                    Err(error) => {
                        json!({"ok":false,"display_id":display_id,"error":safe_error(error)})
                    }
                },
            );
            let product = product?;
            let option = client.full_bundle_option(
                "landsat_ot_c2_l2",
                &product.product_id,
                &product.display_name,
                &cancelled,
            );
            emit(
                "download_options",
                match &option {
                    Ok(option) => json!({"ok":true,"display_id":display_id,
                        "availability":option.availability.as_str(),
                        "expected_bytes":option.expected_size_bytes,
                        "checksum_algorithm":option.checksum_algorithm}),
                    Err(error) => {
                        json!({"ok":false,"display_id":display_id,"error":safe_error(error)})
                    }
                },
            );
            option?;
        }
        Ok(())
    })();
    let logout = session.logout(&|| false);
    emit(
        "closed",
        json!({"ok":result.is_ok(),"logout_ok":logout.is_ok(),"payload_bytes":0,"download_requests":0}),
    );
    if let Err(error) = result {
        panic!(
            "USGS_READONLY_PROBE_STOPPED:{}:{}",
            error.endpoint().as_str(),
            error.code_str()
        );
    }
}
