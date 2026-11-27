#![cfg(feature = "internal-test-server")]

use std::{path::PathBuf, time::Duration};

use insar_agent_mcp::{rmcp_client::RmcpStdioClient, ApprovedTools};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "internal-test-server")]
#[tokio::test]
async fn reviewed_test_binary_stdio_mcp_round_trip() {
    // Cargo sets this path only when the explicitly gated local test executable is built.
    let path = PathBuf::from(env!("CARGO_BIN_EXE_mcp_fixture"));
    let cancel = CancellationToken::new();
    let client = tokio::time::timeout(
        Duration::from_secs(15),
        RmcpStdioClient::spawn(&path, &[], &cancel),
    )
    .await
    .unwrap()
    .unwrap();
    let approved = ApprovedTools::new(client, vec!["echo".into()], Duration::from_secs(3));
    let tools = approved.list_tools(&cancel).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    let reply = approved
        .call_tool("echo", &json!({"text":"fixture-output"}), &cancel)
        .await
        .unwrap();
    assert!(reply.to_string().contains("fixture-output"));
    approved.shutdown().await.unwrap();
}

#[cfg(feature = "internal-test-server")]
#[tokio::test]
async fn the_server_process_does_not_inherit_the_desktop_environment() {
    // A credential-like variable of the parent; only the reviewed allowlist may pass.
    std::env::set_var("EOSTAGE_FIXTURE_PARENT_ONLY", "fixture-parent-value");
    let path = PathBuf::from(env!("CARGO_BIN_EXE_mcp_fixture"));
    let cancel = CancellationToken::new();
    let client = tokio::time::timeout(
        Duration::from_secs(15),
        RmcpStdioClient::spawn(&path, &[], &cancel),
    )
    .await
    .unwrap()
    .unwrap();
    let approved = ApprovedTools::new(client, vec!["has_env".into()], Duration::from_secs(3));
    let probe = |name: &'static str| {
        let approved = &approved;
        let cancel = &cancel;
        async move {
            approved
                .call_tool("has_env", &json!({"name": name}), cancel)
                .await
                .unwrap()
                .to_string()
        }
    };
    assert!(probe("EOSTAGE_FIXTURE_PARENT_ONLY").await.contains("unset"));
    assert!(probe("SystemRoot").await.contains("\"set\""));
    approved.shutdown().await.unwrap();
}

#[cfg(feature = "internal-test-server")]
#[tokio::test]
async fn a_server_that_never_initializes_times_out_instead_of_hanging() {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_mcp_fixture"));
    let started = std::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(10),
        RmcpStdioClient::spawn_with_handshake_timeout(
            &path,
            &["--never-initialize".to_owned()],
            Duration::from_millis(500),
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("spawn must give up on its own")
    .err()
    .expect("a silent server is not a connection");
    assert_eq!(error.kind, insar_agent_model::ErrorKind::Transient);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(feature = "internal-test-server")]
#[tokio::test]
async fn proxy_and_certificate_settings_pass_only_without_an_account_in_them() {
    std::env::set_var("ALL_PROXY", "http://fixture-user:fixture-pass@127.0.0.1:9");
    std::env::set_var("NO_PROXY", "localhost,127.0.0.1");
    std::env::set_var("SSL_CERT_FILE", "fixture-ca-bundle.pem");
    let path = PathBuf::from(env!("CARGO_BIN_EXE_mcp_fixture"));
    let cancel = CancellationToken::new();
    let spawned = tokio::time::timeout(
        Duration::from_secs(15),
        RmcpStdioClient::spawn(&path, &[], &cancel),
    )
    .await;
    std::env::remove_var("ALL_PROXY");
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("SSL_CERT_FILE");
    let approved = ApprovedTools::new(
        spawned.unwrap().unwrap(),
        vec!["has_env".into()],
        Duration::from_secs(3),
    );
    let probe = |name: &'static str| {
        let approved = &approved;
        let cancel = &cancel;
        async move {
            approved
                .call_tool("has_env", &json!({"name": name}), cancel)
                .await
                .unwrap()
                .to_string()
        }
    };
    assert!(probe("ALL_PROXY").await.contains("unset"));
    assert!(probe("NO_PROXY").await.contains("\"set\""));
    assert!(probe("SSL_CERT_FILE").await.contains("\"set\""));
    approved.shutdown().await.unwrap();
}
