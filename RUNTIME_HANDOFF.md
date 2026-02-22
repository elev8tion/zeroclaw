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

### zeroclaw-main — uncommitted

The gateway webhook now runs the full agent loop with tools instead of raw LLM chat. Three files changed (+221/-80 lines):

- `src/agent/loop_.rs` — `ToolCallRecord` struct, `agent_turn()` and `run_tool_call_loop()` accept optional tool record collection
- `src/gateway/mod.rs` — `agent_turn()` replaces `simple_chat()` in webhook handler, `GET /info` endpoint, 5 new `AppState` fields, 120s timeout
- `src/channels/mod.rs` — passes `None` for new tool_records param

1740 tests pass. 3 pre-existing `memory::lucid` failures (unrelated).

### zeroclaw-chat — uncommitted

12 modified files, 5 new files (+301/-83 lines):

- **Token hardened** — `NEXT_PUBLIC_GATEWAY_TOKEN` → `GATEWAY_TOKEN` (server-only)
- **Tool call rendering** — collapsible tool blocks in agent messages (name, success/fail, duration, result)
- **Agent panel** — sidebar shows delegates, tools, runtime channel status via `/api/info`
- **NCB persistence** — fire-and-forget message storage, history loading on channel switch
- **Multi-channel** — channels populated from runtime `/info` instead of hardcoded

Build passes. Routes: `/api/chat`, `/api/health`, `/api/info`, `/api/messages`, `/api/pair`.

---

## NCB Database

Three tables in NoCodeBackend, ready to use. More can be added any time.

| Table | Fields |
|-------|--------|
| `conversations` | `channel`, `title`, `created_at`, `updated_at` |
| `messages` | `conversation_id`, `role`, `content`, `model`, `client_message_id`, `created_at` |
| `user_sessions` | `email`, `cf_access_sub`, `last_seen`, `created_at` |

Token: `ncb_5555d9c08f06607289b6bc7296b228436103afcee5ec30a5`

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

### Chat frontend (`~/zeroclaw-chat/.env.local`)

```env
GATEWAY_URL=http://localhost:8080
GATEWAY_TOKEN=zc_local_dev_2026
NCB_API_TOKEN=ncb_5555d9c08f06607289b6bc7296b228436103afcee5ec30a5
```

---

## Commands

```bash
# Runtime
cd ~/zeroclaw-main && cargo run --release -- daemon --port 8080

# Tunnel
cloudflared tunnel run zeroclaw-gateway

# Frontend
cd ~/zeroclaw-chat && npm run dev       # local
cd ~/zeroclaw-chat && npm run deploy    # deploy to Cloudflare
```

---

## What Needs To Happen

1. Commit changes in both repos
2. Rebuild and restart the runtime (`cargo build --release`)
3. Store Cloudflare secrets before deploying frontend:
   ```bash
   cd ~/zeroclaw-chat
   npx wrangler secret put GATEWAY_TOKEN
   npx wrangler secret put NCB_API_TOKEN
   ```
4. Deploy frontend: `npm run deploy`
5. Test end-to-end — send a tool-using message from ay8.app

---

## Rules

- `GATEWAY_TOKEN` is server-only. Never use `NEXT_PUBLIC_` prefix for tokens.
- NCB failures never block chat. All NCB writes are fire-and-forget.
- Use `@opennextjs/cloudflare` for deployment. Do not add `export const runtime = 'edge'` to routes.
- Structured JSON responses, not SSE streaming. Tool calls return after the agent loop completes.
