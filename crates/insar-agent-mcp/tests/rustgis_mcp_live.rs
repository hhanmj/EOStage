use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use insar_agent_mcp::{rmcp_client::RmcpStdioClient, ApprovedTools, McpClient};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn temp_test_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("eostage-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn find_real_rustgis_cli() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("RUSTGIS_CLI_PATH") {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Some(p);
        }
    }
    for fixed in [
        "/opt/homebrew/bin/rustgis-cli",
        "/usr/local/bin/rustgis-cli",
        "/opt/homebrew/bin/rustgis",
    ] {
        let p = Path::new(fixed);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    None
}

#[tokio::test]
async fn live_rustgis_mcp_stdio_lists_tools_buffers_and_saves_project() {
    let Some(exe) = find_real_rustgis_cli() else {
        eprintln!("SKIPPED: real rustgis-cli not installed on this host");
        return;
    };

    let tmp_dir = temp_test_dir("rustgis-mcp-live");
    let input_geojson = tmp_dir.join("hefei_seed.geojson");
    let saved_rgis = tmp_dir.join("hefei_buffer_live.rgis");
    fs::write(
        &input_geojson,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"Hefei"},"geometry":{"type":"Point","coordinates":[117.2830,31.8612]}}]}"#,
    )
    .expect("write seed geojson");

    let cancel = CancellationToken::new();
    let args = vec![
        "mcp".to_string(),
        "--yes".to_string(),
        "--no-plugins".to_string(),
    ];
    let client = tokio::time::timeout(
        Duration::from_secs(15),
        RmcpStdioClient::spawn(&exe, &args, &cancel),
    )
    .await
    .expect("mcp handshake must not time out")
    .expect("mcp spawn succeeds against real rustgis-cli");

    let tools = client
        .list_tools(&cancel)
        .await
        .expect("list_tools succeeds");
    assert!(
        tools.iter().any(|t| t.name == "run_tool") && tools.iter().any(|t| t.name == "run_command"),
        "expected run_tool and run_command in RustGIS MCP tools, got: {:?}",
        tools.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    let add_res = client
        .call_tool(
            "run_command",
            &json!({
                "command": "layer.addData",
                "params": { "path": input_geojson.to_string_lossy() }
            }),
            &cancel,
        )
        .await
        .expect("layer.addData via MCP");
    assert_ne!(
        add_res.get("isError").and_then(|v| v.as_bool()),
        Some(true),
        "layer.addData returned MCP error: {add_res}"
    );

    let buffer_res = client
        .call_tool(
            "run_tool",
            &json!({
                "tool": "buffer",
                "params": {
                    "input": "hefei_seed",
                    "distance": 500,
                    "units": "Meters"
                }
            }),
            &cancel,
        )
        .await
        .expect("buffer GP tool via MCP");
    assert_ne!(
        buffer_res.get("isError").and_then(|v| v.as_bool()),
        Some(true),
        "buffer returned MCP error: {buffer_res}"
    );

    let layers_res = client
        .call_tool("list_layers", &json!({}), &cancel)
        .await
        .expect("list_layers via MCP");
    let layers_text = layers_res.to_string();
    assert!(
        layers_text.contains("hefei_seed"),
        "expected loaded layer in list_layers output: {layers_text}"
    );

    let save_res = client
        .call_tool(
            "run_command",
            &json!({
                "command": "project.saveAs",
                "params": { "path": saved_rgis.to_string_lossy() }
            }),
            &cancel,
        )
        .await
        .expect("project.saveAs via MCP");
    assert_ne!(
        save_res.get("isError").and_then(|v| v.as_bool()),
        Some(true),
        "project.saveAs returned MCP error: {save_res}"
    );

    let saved_len = fs::metadata(&saved_rgis)
        .expect("saved .rgis file must exist on disk")
        .len();
    assert!(saved_len > 0, "saved .rgis project must be non-empty");

    // Also verify ApprovedTools boundary lists tools safely through the same client.
    let approved = ApprovedTools::new(
        client,
        vec![
            "list_layers".into(),
            "run_command".into(),
            "run_tool".into(),
        ],
        Duration::from_secs(10),
    );
    let approved_tools = approved
        .list_tools(&cancel)
        .await
        .expect("ApprovedTools::list_tools");
    assert_eq!(approved_tools.len(), 3);
    approved.shutdown().await.expect("clean MCP shutdown");

    let _ = fs::remove_dir_all(&tmp_dir);
}
