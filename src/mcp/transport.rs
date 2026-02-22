use super::protocol::{JsonRpcRequest, JsonRpcResponse};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
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

struct StdioInner {
    child: Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
}

/// Spawn a child process and return its inner handles.
fn spawn_child(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<StdioInner> {
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

    Ok(StdioInner {
        child,
        stdin,
        reader,
    })
}

/// Send a request over stdio and read the matching response.
async fn stdio_send(
    inner: &mut StdioInner,
    alive: &AtomicBool,
    request: &JsonRpcRequest,
) -> Result<JsonRpcResponse> {
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
            alive.store(false, Ordering::Relaxed);
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

/// Kill a stdio child, giving it a grace period.
async fn kill_child(inner: &mut StdioInner) {
    drop(inner.stdin.shutdown().await);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), inner.child.wait()).await;
    let _ = inner.child.kill().await;
}

// ── Resilient Stdio Transport ───────────────────────────────────

/// Stdio transport that auto-restarts the child process on crash.
///
/// Holds the spawn config so it can re-spawn. When `auto_restart` is false,
/// behaves identically to a basic stdio transport (fails permanently on crash).
pub struct StdioTransport {
    inner: Mutex<StdioInner>,
    alive: Arc<AtomicBool>,
    // Spawn config (retained for auto-restart)
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    auto_restart: bool,
}

impl StdioTransport {
    /// Spawn the MCP server subprocess.
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        auto_restart: bool,
    ) -> Result<Self> {
        let child_inner = spawn_child(command, args, env)?;

        Ok(Self {
            inner: Mutex::new(child_inner),
            alive: Arc::new(AtomicBool::new(true)),
            command: command.to_string(),
            args: args.to_vec(),
            env: env.clone(),
            auto_restart,
        })
    }

    /// Attempt to restart the child process. Returns Ok(true) if restart succeeded.
    async fn try_restart(&self) -> Result<bool> {
        if !self.auto_restart {
            return Ok(false);
        }

        tracing::info!(command = %self.command, "MCP server crashed — attempting restart");

        let mut inner = self.inner.lock().await;
        // Kill old process cleanly
        kill_child(&mut inner).await;

        // Spawn fresh process
        match spawn_child(&self.command, &self.args, &self.env) {
            Ok(new_inner) => {
                *inner = new_inner;
                self.alive.store(true, Ordering::Relaxed);
                tracing::info!(command = %self.command, "MCP server restarted successfully");
                Ok(true)
            }
            Err(e) => {
                tracing::error!(command = %self.command, error = %e, "MCP server restart failed");
                Err(e)
            }
        }
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        // First attempt
        {
            let mut inner = self.inner.lock().await;
            match stdio_send(&mut inner, &self.alive, request).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if !self.auto_restart {
                        return Err(e);
                    }
                    tracing::warn!(error = %e, "MCP stdio send failed — will attempt restart");
                }
            }
        }

        // Auto-restart and retry once
        self.try_restart().await?;

        // Re-initialize after restart (caller must handle this via McpClient)
        // For now, retry the send directly — the client's initialize will re-run on next call
        let mut inner = self.inner.lock().await;
        stdio_send(&mut inner, &self.alive, request).await
    }

    async fn shutdown(&self) -> Result<()> {
        self.alive.store(false, Ordering::Relaxed);
        let mut inner = self.inner.lock().await;
        kill_child(&mut inner).await;
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
