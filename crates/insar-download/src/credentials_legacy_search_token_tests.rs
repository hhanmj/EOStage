//! Loopback-fixture coverage for the legacy-CMR search token seam: a
//! fixture `.netrc` (never the developer's real one) must resolve through
//! `EarthdataCredentialSource::Auto` to a Basic Earthdata login and exchange
//! for the bearer token the restricted legacy SAR collections' CMR search
//! attaches. No network access beyond 127.0.0.1; no live EDL endpoints.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use super::{
    material_to_transfer_auth_for_test, resolve_earthdata_credential_with, CredentialMaterial,
    EarthdataCredentialSource, TransferAuth,
};

/// Loopback EDL stand-in (same shape as the earthdata session fixtures):
/// `/token` answers the bearer JSON, `/authorize` sets the `asf-urs` cookie
/// the session establishment requires, and every request's `Authorization`
/// header is recorded. The token value itself is never printed.
fn edl_token_fixture() -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let mut authorizations = Vec::new();
        let mut idle_since = std::time::Instant::now();
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    idle_since = std::time::Instant::now();
                    stream.set_nonblocking(false).unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut request = String::new();
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if line.to_ascii_lowercase().starts_with("authorization:") {
                            authorizations.push(line.trim().to_owned());
                        }
                        request.push_str(&line.to_ascii_lowercase());
                    }
                    let path = request
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_owned();
                    let mut stream = reader.into_inner();
                    if path == "/token" {
                        let body = br#"{"access_token":"fixture-search-token"}"#;
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(body);
                    } else if path == "/authorize" {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    } else {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                        );
                    }
                    let _ = stream.flush();
                }
                Err(_) => {
                    if idle_since.elapsed() > Duration::from_millis(400) {
                        return authorizations;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    });
    (base, server)
}

struct TempNetrc(std::path::PathBuf);

impl TempNetrc {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempNetrc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn tempfile_netrc(contents: &str) -> TempNetrc {
    let path = std::env::temp_dir().join(format!(
        "insar-netrc-fixture-{}-{}.netrc",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, contents).unwrap();
    TempNetrc(path)
}

/// A fixture `.netrc` resolves through the Auto source to a Basic Earthdata
/// login, and that login exchanges for the search bearer token through the
/// real session machinery — proving the netrc path desktop users rely on
/// feeds the legacy CMR route's token seam.
#[test]
fn netrc_login_resolves_to_basic_and_exchanges_for_a_search_bearer() {
    let netrc =
        tempfile_netrc("machine urs.earthdata.nasa.gov login fixture-user password fixture-pass\n");
    let credential = resolve_earthdata_credential_with(
        EarthdataCredentialSource::Auto,
        |_| None,
        &[netrc.path().to_path_buf()],
    )
    .expect("fixture netrc resolves to a Basic credential");
    match &credential {
        TransferAuth::Basic { .. } => {}
        other => panic!("expected a Basic login from the netrc fixture, got {other:?}"),
    }

    let mut transfer = crate::http_transfer::HttpTransfer::new(crate::TransferOptions {
        max_attempts: 1,
        backoff: Duration::ZERO,
        timeout: Duration::from_secs(2),
        ..crate::TransferOptions::default()
    })
    .unwrap();
    let (base, server) = edl_token_fixture();
    transfer
        .set_resolved_earthdata_session_endpoints_for_test(
            &format!("{base}/token"),
            &format!("{base}/authorize"),
            &format!("{base}/"),
        )
        .unwrap();
    transfer
        .ensure_earthdata_session_with_credential(&credential)
        .expect("the loopback EDL stand-in establishes the session");
    let token = transfer.earthdata_session_token().unwrap();
    assert_eq!(token, "fixture-search-token");

    let authorizations = server.join().unwrap();
    assert!(
        authorizations
            .iter()
            .any(|header| header.to_ascii_lowercase().starts_with("authorization: basic ")),
        "the token exchange must present the fixture netrc Basic login; recorded headers: {authorizations:?}"
    );
}

/// A stored bearer token short-circuits the exchange entirely: the bearer
/// arm of the resolver returns the stored value without any session request.
#[test]
fn stored_bearer_token_is_returned_without_a_session_request() {
    let material = CredentialMaterial::bearer("stored-search-token").unwrap();
    match material_to_transfer_auth_for_test(material).unwrap() {
        TransferAuth::Bearer(_) => {}
        other => panic!("expected bearer, got {other:?}"),
    }
}
