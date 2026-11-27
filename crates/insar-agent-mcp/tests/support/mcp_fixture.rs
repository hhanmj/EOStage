//! Local-only MCP protocol fixture. This executable is never bundled with the desktop.
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        ServerCapabilities, ServerConfig, Tool,
    },
    transport::io::stdio,
    ServerHandler, ServiceExt,
};
use serde_json::Value;

#[derive(Clone)]
struct Echo;
impl ServerHandler for Echo {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult {
            tools: vec![
                Tool::new_with_raw("echo", Some("local fixture".into()), serde_json::Map::new()),
                Tool::new_with_raw(
                    "has_env",
                    Some("reports whether a variable is set".into()),
                    serde_json::Map::new(),
                ),
                Tool::new_with_raw(
                    "exec_command",
                    Some("unregistered".into()),
                    serde_json::Map::new(),
                ),
            ],
            next_cursor: None,
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let argument = |name: &str| {
            request
                .arguments
                .as_ref()
                .and_then(|args| args.get(name).and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_default()
        };
        let text = if request.name == "has_env" {
            let set = std::env::var_os(argument("name")).is_some();
            if set { "set" } else { "unset" }.to_owned()
        } else {
            argument("text")
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().any(|arg| arg == "--never-initialize") {
        // Holds stdio open without ever answering the handshake.
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        return;
    }
    if let Ok(service) = Echo.serve(stdio()).await {
        let _ = service.waiting().await;
    }
}
