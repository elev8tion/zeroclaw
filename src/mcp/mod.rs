pub mod bridge;
pub mod client;
pub mod config;
pub mod protocol;
pub mod transport;

use bridge::{McpBridgedTool, McpListResourcesTool, McpReadResourceTool};
use client::McpClient;
use config::McpConfig;
use transport::{SseTransport, StdioTransport};

use crate::tools::Tool;
use anyhow::Result;
use std::sync::Arc;

/// Manages all MCP server connections and their bridged tools.
pub struct McpManager {
    clients: Vec<Arc<McpClient>>,
}

impl McpManager {
    /// Connect to all configured MCP servers, discover their tools, and return
    /// bridged `Tool` implementations ready for the agent registry.
    ///
    /// Servers that fail to connect are logged and skipped — partial success is OK.
    pub async fn create_mcp_tools(config: &McpConfig) -> Result<(Self, Vec<Box<dyn Tool>>)> {
        if !config.enabled || config.servers.is_empty() {
            return Ok((Self { clients: vec![] }, vec![]));
        }

        let mut clients = Vec::new();
        let mut tools: Vec<Box<dyn Tool>> = Vec::new();

        for (server_name, server_config) in &config.servers {
            match connect_server(server_name, server_config).await {
                Ok((client, server_tools)) => {
                    let tool_count = server_tools.len();
                    tools.extend(server_tools);
                    clients.push(client);
                    tracing::info!(
                        server = %server_name,
                        tools = tool_count,
                        "MCP server connected"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        server = %server_name,
                        error = %e,
                        "MCP server failed to connect — skipping"
                    );
                }
            }
        }

        if !tools.is_empty() {
            tracing::info!(
                servers = clients.len(),
                total_tools = tools.len(),
                "MCP tools registered"
            );
        }

        Ok((Self { clients }, tools))
    }

    /// Gracefully shut down all MCP server connections.
    pub async fn shutdown(&self) {
        for client in &self.clients {
            if let Err(e) = client.shutdown().await {
                tracing::warn!(
                    server = %client.server_name,
                    error = %e,
                    "MCP server shutdown error"
                );
            }
        }
    }
}

/// Connect to a single MCP server and discover its tools.
async fn connect_server(
    server_name: &str,
    config: &config::McpServerConfig,
) -> Result<(Arc<McpClient>, Vec<Box<dyn Tool>>)> {
    // Create transport
    let transport: Box<dyn transport::McpTransport> = match config.transport.as_str() {
        "sse" => {
            let url = config
                .url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("SSE transport requires 'url'"))?;
            Box::new(SseTransport::new(url, config.timeout_secs))
        }
        _ => {
            // Default: stdio
            let command = config
                .command
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Stdio transport requires 'command'"))?;
            Box::new(StdioTransport::spawn(command, &config.args, &config.env)?)
        }
    };

    // Create client and initialize
    let mut client = McpClient::new(server_name.to_string(), transport, config.timeout_secs);
    client.initialize().await?;

    let client = Arc::new(client);
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    // Discover and bridge tools
    let mcp_tools = client.list_tools().await?;
    for tool_def in mcp_tools {
        tools.push(Box::new(McpBridgedTool::new(
            server_name,
            tool_def.name,
            tool_def.description,
            tool_def.input_schema,
            Arc::clone(&client),
        )));
    }

    // Add resource tools if the server supports resources
    if client.has_resources() {
        tools.push(Box::new(McpListResourcesTool::new(
            server_name,
            Arc::clone(&client),
        )));
        tools.push(Box::new(McpReadResourceTool::new(
            server_name,
            Arc::clone(&client),
        )));
    }

    Ok((client, tools))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_returns_empty() {
        let config = McpConfig::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (manager, tools) = rt.block_on(McpManager::create_mcp_tools(&config)).unwrap();
        assert!(tools.is_empty());
        assert!(manager.clients.is_empty());
    }

    #[test]
    fn enabled_but_no_servers_returns_empty() {
        let config = McpConfig {
            enabled: true,
            servers: std::collections::HashMap::new(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (manager, tools) = rt.block_on(McpManager::create_mcp_tools(&config)).unwrap();
        assert!(tools.is_empty());
        assert!(manager.clients.is_empty());
    }
}
