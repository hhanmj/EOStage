//! L5 recheck 2026-10-19: requests whose server certificate cannot be verified
//! are classified as such. A real rustls server on 127.0.0.1 presents a
//! self-signed certificate (tests/fixtures/tls), which every client rejects
//! with "invalid peer certificate: UnknownIssuer", the failure every NISAR
//! Range request of the release candidate met when it ran as a low-integrity
//! process that could not open the current user's certificate store.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use insar_core::{DownloadOutcome, ProxyPolicy};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::http_transfer::{ExactRangeSendOutcome, ExactRangeTransportCause};
use crate::transfer_backend::TransferBackend;
use crate::{
    HttpTransfer, ResolvedAsset, ResolvedMethod, TransferAuth, TransferOptions, TransferRequest,
};

const CERTIFICATE: &[u8] = include_bytes!("../tests/fixtures/tls/loopback-self-signed.cert.der");
const PRIVATE_KEY: &[u8] = include_bytes!("../tests/fixtures/tls/loopback-self-signed.key.der");

/// Accepts `connections` connections and drives each TLS handshake until it
/// fails; returns how many the client aborted.
fn self_signed_tls_server(connections: usize) -> (u16, JoinHandle<usize>) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("default TLS versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(CERTIFICATE.to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(PRIVATE_KEY.to_vec())),
        )
        .expect("fixture certificate and key");
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("loopback address").port();
    let handle = thread::spawn(move || {
        let mut aborted = 0;
        for _ in 0..connections {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let mut connection =
                rustls::ServerConnection::new(Arc::clone(&config)).expect("server connection");
            while connection.is_handshaking() {
                if connection.complete_io(&mut stream).is_err() {
                    aborted += 1;
                    break;
                }
            }
        }
        aborted
    });
    (port, handle)
}

fn transfer() -> HttpTransfer {
    HttpTransfer::new_with_proxy_policy(
        TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(10),
            ..TransferOptions::default()
        },
        ProxyPolicy::Direct,
    )
    .expect("transfer")
}

#[test]
fn an_exact_range_request_to_an_unverifiable_certificate_is_a_tls_certificate_failure() {
    let (port, server) = self_signed_tls_server(1);
    let asset = ResolvedAsset::new(
        &format!("https://127.0.0.1:{port}/source.h5"),
        ResolvedMethod::Get,
    )
    .expect("loopback asset");

    let outcome = transfer().send_resolved_exact_range(&asset, 0, 4, "\"fixture\"", &|| false);

    match outcome {
        ExactRangeSendOutcome::RetryableExhausted { diagnostic, .. } => {
            assert_eq!(diagnostic.cause, ExactRangeTransportCause::TlsCertificate);
            assert_eq!(diagnostic.cause.as_str(), "tls_certificate");
        }
        _ => panic!("the certificate failure was not reported as a transport failure"),
    }
    assert_eq!(
        server.join().expect("server thread"),
        1,
        "the client aborted the handshake"
    );
}

#[test]
fn a_transfer_to_an_unverifiable_certificate_fails_naming_the_certificate_and_keeps_retrying_possible(
) {
    let (port, server) = self_signed_tls_server(1);
    let root = std::env::temp_dir().join(format!("insar-tls-certificate-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("output root");
    let destination: PathBuf = root.join("product.bin");
    let request = TransferRequest::new(
        "tls-task",
        "tls-item",
        &format!("https://127.0.0.1:{port}/product.bin"),
        destination,
        Some(4),
    )
    .expect("transfer request");

    let result = transfer().download_legacy(
        &request,
        &TransferAuth::None,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    let failure = result.failure.expect("a failed transfer names its failure");
    assert_eq!(failure.code, "DL005");
    assert!(failure.retryable);
    assert!(
        failure
            .reason_zh
            .starts_with("无法验证服务器证书（本机证书库无法读取或网络被拦截），断点已保留。")
            && failure.reason_zh.contains("source=tls_certificate"),
        "{}",
        failure.reason_zh
    );
    assert_eq!(
        server.join().expect("server thread"),
        1,
        "the client aborted the handshake"
    );
    let _ = std::fs::remove_dir_all(&root);
}
