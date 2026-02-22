# ZeroClaw — Runtime Handoff

## System Overview

ZeroClaw is an autonomous agent runtime (Rust, ~72k LOC). A Next.js chat frontend at `https://ay8.app` proxies requests to the runtime via a Cloudflare Tunnel at `https://gateway.ay8.app` → `localhost:8080`.

```
Browser → ay8.app (Cloudflare Worker)
       → Next.js API routes (server-side, adds Bearer token)
       → gateway.ay8.app (Cloudflare Tunnel)
       → localhost:8080 (ZeroClaw gateway, PairingGuard auth)
       → Agent loop with full tool execution → AI provider
```

The browser never talks to the gateway directly. Auth is bearer tokens via PairingGuard (SHA-256 compare). No Cloudflare Zero Trust / Access — that was abandoned.

---

## Repos and Locations

| Item | Path |
|------|------|
| Runtime | `~/zeroclaw-main` |
| Chat frontend | `~/zeroclaw-chat` |
| Gateway config | `~/.zeroclaw/config.toml` |
| Tunnel config | `~/.cloudflared/config.yml` |
| Tunnel ID | `0e0ff8b1-e91a-4861-a762-5031ad8e71c8` |
| NCB MCP config | `~/.claude.json` |

---

## Current State

Everything is committed, deployed, and working end-to-end.

### zeroclaw-main — latest

The gateway webhook runs the full agent loop with tools instead of raw LLM chat. Key files:

- `src/agent/loop_.rs` — `ToolCallRecord` struct, `agent_turn()` and `run_tool_call_loop()` accept optional tool record collection, MCP tools wired into both `run()` and `process_message()`
- `src/gateway/mod.rs` — `agent_turn()` replaces `simple_chat()` in webhook handler, `GET /info` endpoint, `AppState` includes `mcp_manager`, 120s timeout
- `src/channels/mod.rs` — passes `None` for new tool_records param
- `src/mcp/` — **MCP client integration** (see below)

1745 tests pass. Pre-existing flaky `memory::lucid` test (timing-dependent, unrelated).

### MCP Client Integration

ZeroClaw can now connect to MCP (Model Context Protocol) servers and expose their tools to the agent. Each MCP tool becomes a first-class ZeroClaw tool named `mcp__<server>__<tool>`.

**Module structure** (`src/mcp/`):

| File | Purpose |
|------|---------|
| `config.rs` | `McpConfig`, `McpServerConfig` — TOML config types |
| `protocol.rs` | JSON-RPC 2.0 types + MCP protocol structs |
| `transport.rs` | `McpTransport` trait + `StdioTransport` (subprocess) + `SseTransport` (HTTP) |
| `client.rs` | `McpClient` — initialize, tools/list, tools/call, resources |
| `bridge.rs` | `McpBridgedTool` (impl `Tool`), `McpListResourcesTool`, `McpReadResourceTool` |
| `mod.rs` | `McpManager::create_mcp_tools()` — public API |

**Design**: Per-tool bridging with zero new dependencies. Disabled by default (`mcp.enabled = false`). Servers that fail to connect are warned and skipped (graceful degradation). Resource-capable servers also get `list_resources` and `read_resource` synthetic tools.

**Hardening**: Auto-restart on crash (`auto_restart = true` default) — `StdioTransport` holds spawn config and respawns on EOF, retries once. Graceful shutdown via `with_graceful_shutdown` on gateway ctrl+c. Health monitoring via `McpManager::health_status()` exposed in `/info` endpoint as `mcp_servers` array.

**Wired into**: gateway (`src/gateway/mod.rs`), CLI agent (`src/agent/loop_.rs` `run()`), channel processing (`src/agent/loop_.rs` `process_message()`), config schema, onboard wizard.

### zeroclaw-chat — committed (`38ef8f6`)

Features deployed:

- **Token hardened** — `NEXT_PUBLIC_GATEWAY_TOKEN` → `GATEWAY_TOKEN` (server-only)
- **Tool call rendering** — collapsible tool blocks in agent messages (name, success/fail, duration, result)
- **Agent panel** — sidebar shows delegates, tools, runtime channel status via `/api/info`
- **NCB persistence** — awaited writes to NoCodeBackend public data API, history loading on channel switch
- **Multi-channel** — channels populated from runtime `/info` instead of hardcoded
- **Voice input (STT)** — MediaRecorder captures audio, Cloudflare Workers AI Whisper transcribes, auto-sends to agent
- **Voice output (TTS)** — Browser `speechSynthesis` with iOS audio unlock, sentence chunking, auto-speaks agent responses to voice messages
- **Manual TTS** — speaker button on agent messages for on-demand read-aloud

Routes: `/api/chat`, `/api/health`, `/api/info`, `/api/messages`, `/api/pair`, `/api/transcribe`.

Voice flow: tap mic → record → tap again → Workers AI Whisper transcribes → auto-send to agent → agent responds → TTS reads response aloud. No manual send step for voice. Based on the `aismb` repo's VoiceOperator pattern (`getUserMedia` + `MediaRecorder` + server-side transcription).

---

## NCB Database

Data API: `https://app.nocodebackend.com/api/data`
Instance: `36905_zeroclaw_chat`
Path format: `/create/<table>`, `/read/<table>`, `/search/<table>` with `?Instance=36905_zeroclaw_chat`

RLS policies set to `public_readwrite` on `conversations` and `messages` — no session cookies needed.

| Table | Fields | RLS |
|-------|--------|-----|
| `conversations` | `channel`, `user_email`, `title`, `created_at`, `updated_at` | `public_readwrite` |
| `messages` | `conversation_id`, `role`, `content`, `model`, `client_message_id`, `created_at` | `public_readwrite` |
| `user_sessions` | `email`, `cf_access_sub`, `last_seen`, `created_at` | private (default) |

MCP token (for MCP tools only, NOT for REST API): `ncb_5555d9c08f06607289b6bc7296b228436103afcee5ec30a5`

---

## Config

### Gateway (`~/.zeroclaw/config.toml`)

```toml
[gateway]
port = 8080
host = "127.0.0.1"
require_pairing = true
allow_public_bind = false
paired_tokens = ["zc_local_dev_2026", "78e80f32166e97b07b2814e70e808071f5496276c5dd22261b13976695efaa1f"]
```

### MCP servers (`~/.zeroclaw/config.toml`)

```toml
[mcp]
enabled = true

[mcp.servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/me"]

[mcp.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_..." }
```

### Chat frontend (`~/zeroclaw-chat/.env.local`)

```env
GATEWAY_URL=http://localhost:8080
GATEWAY_TOKEN=zc_local_dev_2026
```

Note: `NCB_API_TOKEN` is no longer needed — public RLS policies allow unauthenticated data API access.

---

## Commands

```bash
# Runtime (MUST use run.sh for OAuth token)
cd ~/zeroclaw-main && ./run.sh daemon --port 8080

# Tunnel
cloudflared tunnel run zeroclaw-gateway

# Frontend (local)
cd ~/zeroclaw-chat && npm run dev

# Frontend (deploy)
cd ~/zeroclaw-chat && npx opennextjs-cloudflare build && npx opennextjs-cloudflare deploy
```

---

## Voice Integration

### Architecture

```
Tap mic → getUserMedia (mic permission) → MediaRecorder.start(1000)
Tap again → MediaRecorder.stop() → audio Blob
  → POST /api/transcribe (FormData with audio file)
  → Cloudflare Workers AI (@cf/openai/whisper)
  → transcribed text → auto-send via onSendMessage()
  → agent responds → TTS auto-speaks response
```

### Key files

| File | Role |
|------|------|
| `lib/hooks/useSpeechRecognition.ts` | MediaRecorder + `/api/transcribe` hook. Accepts `onTranscription` callback for auto-send. |
| `lib/hooks/useSpeechSynthesis.ts` | Browser `speechSynthesis` TTS. iOS audio unlock, voice persistence, sentence chunking. |
| `lib/voice-utils.ts` | `sanitizeForSpeech()` strips markdown. `chunkText()` splits at sentence boundaries (~200 chars). |
| `app/api/transcribe/route.ts` | Receives audio FormData, runs `env.AI.run('@cf/openai/whisper', ...)`. |
| `wrangler.jsonc` | `"ai": { "binding": "AI" }` for Workers AI access. |

### Why not Web Speech API SpeechRecognition

Safari's `webkitSpeechRecognition` requires an OS-level "Speech Recognition" permission (System Preferences > Privacy & Security > Speech Recognition) — separate from mic permission. Users get `not-allowed` errors even with mic granted. `getUserMedia` + `MediaRecorder` only needs mic permission, which Safari handles reliably.

### Safari-specific

- `MediaRecorder.start(1000)` — timeslice required or Safari produces blobs with broken metadata
- iOS audio unlock: play silent MP3 + empty `SpeechSynthesisUtterance` at volume 0 during user gesture
- TTS chunking at ~200 char sentence boundaries to avoid Safari's ~15s playback cutoff
- Audio format detection: Safari uses `audio/mp4`, Chrome uses `audio/webm;codecs=opus`

### Bindings

- `env.AI` — Cloudflare Workers AI binding (no API key needed, declared in `wrangler.jsonc`)
- Access via `getCloudflareContext()` from `@opennextjs/cloudflare`

---

## Rules

- `GATEWAY_TOKEN` is server-only. Never use `NEXT_PUBLIC_` prefix for tokens.
- NCB failures never block chat. Writes are awaited but wrapped in try/catch.
- **Must await NCB writes** — Cloudflare Workers kill async work after response is sent. Fire-and-forget (`promise.then().catch()`) does NOT work.
- Use `@opennextjs/cloudflare` for deployment. Do not add `export const runtime = 'edge'` to routes.
- Structured JSON responses, not SSE streaming. Tool calls return after the agent loop completes.
- **Always use `run.sh`** to start the gateway — it extracts the OAuth token from macOS Keychain.
- NCB data API paths: `/create/<table>`, `/read/<table>` etc. Always include `?Instance=36905_zeroclaw_chat`.
- **Do not use browser `SpeechRecognition` API** — use `MediaRecorder` + Workers AI Whisper instead (Safari compatibility).
- Voice reference implementation: `~/aismb` repo (`github.com/elev8tion/aismb`) — VoiceOperator component.
