use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level MCP configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    /// Whether MCP client support is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Named MCP server configurations.
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Transport type: "stdio" (default) or "sse".
    #[serde(default = "default_transport")]
    pub transport: String,
    /// Command to spawn (stdio transport).
    #[serde(default)]
    pub command: Option<String>,
    /// Arguments for the command (stdio transport).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables for the subprocess.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// URL for SSE transport.
    #[serde(default)]
    pub url: Option<String>,
    /// Timeout in seconds for tool calls.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Auto-restart subprocess on crash (stdio only).
    #[serde(default = "default_auto_restart")]
    pub auto_restart: bool,
}

fn default_transport() -> String {
    "stdio".into()
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_auto_restart() -> bool {
    true
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            transport: default_transport(),
            command: None,
            args: Vec::new(),
            env: HashMap::new(),
            url: None,
            timeout_secs: default_timeout_secs(),
            auto_restart: default_auto_restart(),
        }
    }
}
