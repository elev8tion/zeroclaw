use super::protocol::{JsonRpcRequest, JsonRpcResponse};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Transport abstraction for MCP communication.
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a JSON-RPC request and receive the response.
    async fn send(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse>;
    /// Gracefully shut down the transport.
    async fn shutdown(&self) -> Result<()>;
    /// Check if the transport is still alive.
    fn is_alive(&self) -> bool;
}

// ── Stdio Transport ─────────────────────────────────────────────

/// Stdio-based MCP transport: spawns a child process and communicates via stdin/stdout.
pub struct StdioTransport {
    /// Serialized access to stdin/stdout for request-response pairing.
    inner: Mutex<StdioInner>,
    alive: Arc<AtomicBool>,
}

struct StdioInner {
    child: Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
}

impl StdioTransport {
    /// Spawn the MCP server subprocess.
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn MCP server: {command}"))?;

        let stdin = child.stdin.take().context("No stdin on MCP child")?;
        let stdout = child.stdout.take().context("No stdout on MCP child")?;
        let reader = BufReader::new(stdout);

        Ok(Self {
            inner: Mutex::new(StdioInner {
                child,
                stdin,
                reader,
            }),
            alive: Arc::new(AtomicBool::new(true)),
        })
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        let mut inner = self.inner.lock().await;

        // Serialize request as single line
        let mut line = serde_json::to_string(request)?;
        line.push('\n');

        inner
            .stdin
            .write_all(line.as_bytes())
            .await
            .context("Failed to write to MCP stdin")?;
        inner
            .stdin
            .flush()
            .await
            .context("Failed to flush MCP stdin")?;

        // Read response lines, skipping empty lines and JSON-RPC notifications (no id)
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = inner
                .reader
                .read_line(&mut buf)
                .await
                .context("Failed to read from MCP stdout")?;
            if n == 0 {
                self.alive.store(false, Ordering::Relaxed);
                bail!("MCP server closed stdout (EOF)");
            }

            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Try to parse as JSON-RPC response
            match serde_json::from_str::<JsonRpcResponse>(trimmed) {
                Ok(resp) => {
                    // Skip notifications (responses without id that match our request)
                    if resp.id == Some(request.id) {
                        return Ok(resp);
                    }
                    // Notification or mismatched id — skip and keep reading
                }
                Err(_) => {
                    // Not valid JSON-RPC, skip (could be log output)
                }
            }
        }
    }

    async fn shutdown(&self) -> Result<()> {
        self.alive.store(false, Ordering::Relaxed);
        let mut inner = self.inner.lock().await;
        // Drop stdin to signal EOF
        drop(inner.stdin.shutdown().await);
        // Give the process a moment to exit, then kill
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), inner.child.wait()).await;
        let _ = inner.child.kill().await;
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

// ── SSE Transport ───────────────────────────────────────────────

/// SSE-based MCP transport: sends JSON-RPC over HTTP POST, receives via SSE.
pub struct SseTransport {
    url: String,
    client: reqwest::Client,
    alive: AtomicBool,
}

impl SseTransport {
    pub fn new(url: &str, timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_default();

        Self {
            url: url.to_string(),
            client,
            alive: AtomicBool::new(true),
        }
    }
}

#[async_trait]
impl McpTransport for SseTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        let resp = self
            .client
            .post(&self.url)
            .json(request)
            .send()
            .await
            .context("SSE transport: POST failed")?;

        if !resp.status().is_success() {
            bail!("SSE transport: HTTP {} from {}", resp.status(), self.url);
        }

        let body = resp.text().await?;
        // Parse the response — SSE servers may return JSON-RPC directly or as SSE events
        // Try direct JSON-RPC first
        if let Ok(rpc) = serde_json::from_str::<JsonRpcResponse>(&body) {
            return Ok(rpc);
        }

        // Try parsing SSE event format: look for "data:" lines
        for line in body.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if let Ok(rpc) = serde_json::from_str::<JsonRpcResponse>(data) {
                    return Ok(rpc);
                }
            }
        }

        bail!("SSE transport: no valid JSON-RPC response in body")
    }

    async fn shutdown(&self) -> Result<()> {
        self.alive.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}
