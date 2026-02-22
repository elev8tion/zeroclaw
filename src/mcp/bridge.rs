use super::client::McpClient;
use crate::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// A bridged MCP tool exposed as a ZeroClaw `Tool` implementation.
///
/// Each MCP server tool becomes one of these, named `mcp__<server>__<tool>`.
pub struct McpBridgedTool {
    /// Qualified name: mcp__<server>__<tool>
    qualified_name: String,
    /// Tool description from MCP server
    description: String,
    /// JSON Schema for input parameters
    input_schema: Value,
    /// Shared client for the MCP server
    client: Arc<McpClient>,
    /// Original tool name on the MCP server
    mcp_tool_name: String,
}

impl McpBridgedTool {
    pub fn new(
        server_name: &str,
        mcp_tool_name: String,
        description: Option<String>,
        input_schema: Option<Value>,
        client: Arc<McpClient>,
    ) -> Self {
        let qualified_name = format!("mcp__{server_name}__{mcp_tool_name}");
        let description = description
            .unwrap_or_else(|| format!("MCP tool '{mcp_tool_name}' from server '{server_name}'"));
        let input_schema =
            input_schema.unwrap_or_else(|| json!({ "type": "object", "properties": {} }));

        Self {
            qualified_name,
            description,
            input_schema,
            client,
            mcp_tool_name,
        }
    }
}

#[async_trait]
impl Tool for McpBridgedTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.input_schema.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        match self.client.call_tool(&self.mcp_tool_name, args).await {
            Ok(result) => {
                // Concatenate all text content items
                let output: String = result
                    .content
                    .iter()
                    .filter_map(|c| c.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("\n");

                if result.is_error {
                    Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(output),
                    })
                } else {
                    Ok(ToolResult {
                        success: true,
                        output,
                        error: None,
                    })
                }
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("MCP call failed: {e}")),
            }),
        }
    }
}

/// Synthetic tool: list resources available on an MCP server.
pub struct McpListResourcesTool {
    qualified_name: String,
    description: String,
    client: Arc<McpClient>,
}

impl McpListResourcesTool {
    pub fn new(server_name: &str, client: Arc<McpClient>) -> Self {
        Self {
            qualified_name: format!("mcp__{server_name}__list_resources"),
            description: format!("List available resources on MCP server '{server_name}'"),
            client,
        }
    }
}

#[async_trait]
impl Tool for McpListResourcesTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        match self.client.list_resources().await {
            Ok(result) => {
                let output = serde_json::to_string_pretty(&result.resources)?;
                Ok(ToolResult {
                    success: true,
                    output,
                    error: None,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to list resources: {e}")),
            }),
        }
    }
}

/// Synthetic tool: read a specific resource from an MCP server.
pub struct McpReadResourceTool {
    qualified_name: String,
    description: String,
    client: Arc<McpClient>,
}

impl McpReadResourceTool {
    pub fn new(server_name: &str, client: Arc<McpClient>) -> Self {
        Self {
            qualified_name: format!("mcp__{server_name}__read_resource"),
            description: format!("Read a resource by URI from MCP server '{server_name}'"),
            client,
        }
    }
}

#[async_trait]
impl Tool for McpReadResourceTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "uri": {
                    "type": "string",
                    "description": "The URI of the resource to read"
                }
            },
            "required": ["uri"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let uri = args.get("uri").and_then(Value::as_str).unwrap_or_default();

        if uri.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Missing required parameter: uri".into()),
            });
        }

        match self.client.read_resource(uri).await {
            Ok(result) => {
                let output: String = result
                    .contents
                    .iter()
                    .filter_map(|c| c.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("\n");

                Ok(ToolResult {
                    success: true,
                    output,
                    error: None,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to read resource: {e}")),
            }),
        }
    }
}
