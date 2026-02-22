use super::protocol::{
    InitializeResult, JsonRpcRequest, McpToolDef, ResourceReadResult, ResourcesListResult,
    ToolCallResult,
};
use super::transport::McpTransport;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// MCP protocol version we advertise.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Client for a single MCP server.
pub struct McpClient {
    pub server_name: String,
    transport: Box<dyn McpTransport>,
    next_id: AtomicU64,
    timeout: Duration,
    has_resources: bool,
}

impl McpClient {
    /// Create a new client wrapping the given transport.
    pub fn new(server_name: String, transport: Box<dyn McpTransport>, timeout_secs: u64) -> Self {
        Self {
            server_name,
            transport,
            next_id: AtomicU64::new(1),
            timeout: Duration::from_secs(timeout_secs),
            has_resources: false,
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Perform the MCP `initialize` handshake.
    pub async fn initialize(&mut self) -> Result<InitializeResult> {
        let req = JsonRpcRequest::new(
            self.next_id(),
            "initialize",
            Some(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "zeroclaw",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        );

        let resp = tokio::time::timeout(self.timeout, self.transport.send(&req))
            .await
            .context("MCP initialize timed out")?
            .context("MCP initialize failed")?;

        if let Some(err) = resp.error {
            bail!("MCP initialize error: {err}");
        }

        let result: InitializeResult =
            serde_json::from_value(resp.result.context("MCP initialize: empty result")?)?;

        // Track whether server supports resources
        self.has_resources = result.capabilities.resources.is_some();

        // Send initialized notification (no response expected, but we must send it)
        let notif =
            JsonRpcRequest::new(self.next_id(), "notifications/initialized", Some(json!({})));
        // Fire and forget — some servers don't respond to notifications
        let _ = tokio::time::timeout(Duration::from_secs(2), self.transport.send(&notif)).await;

        Ok(result)
    }

    /// List tools available on this MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>> {
        let req = JsonRpcRequest::new(self.next_id(), "tools/list", None);

        let resp = tokio::time::timeout(self.timeout, self.transport.send(&req))
            .await
            .context("MCP tools/list timed out")?
            .context("MCP tools/list failed")?;

        if let Some(err) = resp.error {
            bail!("MCP tools/list error: {err}");
        }

        let result = resp.result.context("MCP tools/list: empty result")?;

        // tools/list returns { tools: [...] }
        let tools_val = result
            .get("tools")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![]));

        let tools: Vec<McpToolDef> = serde_json::from_value(tools_val)?;
        Ok(tools)
    }

    /// Call a tool on this MCP server.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolCallResult> {
        let req = JsonRpcRequest::new(
            self.next_id(),
            "tools/call",
            Some(json!({
                "name": name,
                "arguments": arguments,
            })),
        );

        let resp = tokio::time::timeout(self.timeout, self.transport.send(&req))
            .await
            .context("MCP tools/call timed out")?
            .context("MCP tools/call failed")?;

        if let Some(err) = resp.error {
            bail!("MCP tools/call error: {err}");
        }

        let result: ToolCallResult =
            serde_json::from_value(resp.result.context("MCP tools/call: empty result")?)?;

        Ok(result)
    }

    /// Whether this server advertises resource support.
    pub fn has_resources(&self) -> bool {
        self.has_resources
    }

    /// List resources available on this MCP server.
    pub async fn list_resources(&self) -> Result<ResourcesListResult> {
        let req = JsonRpcRequest::new(self.next_id(), "resources/list", None);

        let resp = tokio::time::timeout(self.timeout, self.transport.send(&req))
            .await
            .context("MCP resources/list timed out")?
            .context("MCP resources/list failed")?;

        if let Some(err) = resp.error {
            bail!("MCP resources/list error: {err}");
        }

        let result: ResourcesListResult =
            serde_json::from_value(resp.result.context("MCP resources/list: empty result")?)?;

        Ok(result)
    }

    /// Read a specific resource by URI.
    pub async fn read_resource(&self, uri: &str) -> Result<ResourceReadResult> {
        let req = JsonRpcRequest::new(
            self.next_id(),
            "resources/read",
            Some(json!({ "uri": uri })),
        );

        let resp = tokio::time::timeout(self.timeout, self.transport.send(&req))
            .await
            .context("MCP resources/read timed out")?
            .context("MCP resources/read failed")?;

        if let Some(err) = resp.error {
            bail!("MCP resources/read error: {err}");
        }

        let result: ResourceReadResult =
            serde_json::from_value(resp.result.context("MCP resources/read: empty result")?)?;

        Ok(result)
    }

    /// Gracefully shut down the transport.
    pub async fn shutdown(&self) -> Result<()> {
        self.transport.shutdown().await
    }

    /// Check if the underlying transport is alive.
    pub fn is_alive(&self) -> bool {
        self.transport.is_alive()
    }
}
