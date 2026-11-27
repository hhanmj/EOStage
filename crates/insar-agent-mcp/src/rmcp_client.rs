//! rmcp 3.7.0 stdio adapter. Only the caller's reviewed absolute executable is allowed.
use std::{future::Future, path::Path, pin::Pin, time::Duration};

use insar_agent_model::{ErrorKind, ModelError};
use rmcp::{
    model::CallToolRequestParams, service::RunningService, transport::TokioChildProcess,
    RoleClient, ServiceExt,
};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{McpClient, RemoteTool};

type ClientService = RunningService<RoleClient, ()>;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Environment variables a spawned server may see. Locations and system
/// settings only; nothing here is a credential.
const INHERITED_ENVIRONMENT: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "PATH",
    "TEMP",
    "TMP",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "NUMBER_OF_PROCESSORS",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "CommonProgramFiles",
    "LANG",
    "LC_ALL",
    "TZ",
    // Certificate bundles a corporate network may require (locations only).
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
];

/// Proxy variables; passed only when the value carries no user name or password.
const PROXY_ENVIRONMENT: &[&str] = &["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"];

pub struct RmcpStdioClient {
    service: Mutex<Option<ClientService>>,
}

impl RmcpStdioClient {
    /// No shell or PATH lookup: executable must have been explicitly reviewed by caller.
    pub async fn spawn(
        executable: &Path,
        args: &[String],
        cancel: &CancellationToken,
    ) -> Result<Self, ModelError> {
        Self::spawn_with_handshake_timeout(executable, args, HANDSHAKE_TIMEOUT, cancel).await
    }

    /// As `spawn`; a server that does not finish the MCP handshake within
    /// `handshake_timeout` is stopped and reported as transient.
    pub async fn spawn_with_handshake_timeout(
        executable: &Path,
        args: &[String],
        handshake_timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Self, ModelError> {
        if !executable.is_absolute() || !executable.is_file() || cancel.is_cancelled() {
            return Err(ModelError::new(ErrorKind::Parameter, None));
        }
        let mut command = tokio::process::Command::new(executable);
        command.args(args);
        // Dropping the client, or giving up on the handshake, ends the server.
        command.kill_on_drop(true);
        if let Some(directory) = executable.parent() {
            command.current_dir(directory);
        }
        // The server is third-party code: it gets the system and profile
        // locations it needs to start, never the desktop process's other
        // variables (API keys, tokens, proxy URLs with credentials).
        command.env_clear();
        command.envs(std::env::vars_os().filter(|(name, value)| {
            let Some(name) = name.to_str() else {
                return false;
            };
            let listed = |names: &[&str]| {
                names
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(name))
            };
            // Proxy settings reach the server only without an account in them.
            listed(INHERITED_ENVIRONMENT)
                || listed(PROXY_ENVIRONMENT)
                    && value.to_str().is_some_and(|value| !value.contains('@'))
        }));
        let child = TokioChildProcess::new(command)
            .map_err(|_| ModelError::new(ErrorKind::Transient, None))?;
        let service = tokio::select! {
            () = cancel.cancelled() => return Err(ModelError::new(ErrorKind::Cancelled, None)),
            result = tokio::time::timeout(handshake_timeout, ().serve(child)) => result
                .map_err(|_| ModelError::new(ErrorKind::Transient, None))?
                .map_err(|_| ModelError::new(ErrorKind::Transient, None))?,
        };
        Ok(Self {
            service: Mutex::new(Some(service)),
        })
    }
}

impl McpClient for RmcpStdioClient {
    fn list_tools<'a>(
        &'a self,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RemoteTool>, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.service.lock().await;
            let service = guard
                .as_ref()
                .ok_or(ModelError::new(ErrorKind::Transient, None))?;
            let tools = tokio::select! {
                () = cancel.cancelled() => return Err(ModelError::new(ErrorKind::Cancelled, None)),
                result = service.peer().list_all_tools() => result.map_err(|_| ModelError::new(ErrorKind::Transient, None))?,
            };
            Ok(tools
                .into_iter()
                .map(|tool| RemoteTool {
                    name: tool.name.into_owned(),
                    description: tool
                        .description
                        .map(|text| text.into_owned())
                        .unwrap_or_default(),
                    input_schema: Value::Object((*tool.input_schema).clone()),
                })
                .collect())
        })
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Value,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let arguments = arguments
                .as_object()
                .cloned()
                .ok_or(ModelError::new(ErrorKind::Parameter, None))?;
            let guard = self.service.lock().await;
            let service = guard
                .as_ref()
                .ok_or(ModelError::new(ErrorKind::Transient, None))?;
            let result = tokio::select! {
                () = cancel.cancelled() => return Err(ModelError::new(ErrorKind::Cancelled, None)),
                result = service.peer().call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments)) => result.map_err(|_| ModelError::new(ErrorKind::Transient, None))?,
            };
            serde_json::to_value(result).map_err(|_| ModelError::new(ErrorKind::Capability, None))
        })
    }

    fn shutdown<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), ModelError>> + Send + 'a>> {
        Box::pin(async move {
            let mut guard = self.service.lock().await;
            if let Some(mut service) = guard.take() {
                service
                    .close()
                    .await
                    .map_err(|_| ModelError::new(ErrorKind::Transient, None))?;
            }
            Ok(())
        })
    }
}
