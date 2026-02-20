# ZeroClaw Chat — Handoff Report

**Date:** 2026-02-20
**From:** Chat Frontend Agent (zeroclaw-chat)
**To:** Main Runtime Agent (zeroclaw-main)
**Status:** Frontend operational, gateway integration working, ready for orchestration layer

---

## What Has Been Completed

### zeroclaw-chat (Next.js 16 Frontend)

**Location:** `/Users/kcdacre8tor/zeroclaw-chat`
**Stack:** Next.js 16.1.6, React 19, Tailwind CSS v4, TypeScript
**Running on:** `http://localhost:3000`

#### Files Created

| File | Purpose |
|------|---------|
| `app/layout.tsx` | Root layout, Inter font, ThemeProvider, mobile viewport |
| `app/page.tsx` | Entry point, renders ChatContainer |
| `app/globals.css` | Dark theme tokens, Tailwind v4 `@theme inline` syntax |
| `lib/types.ts` | TypeScript types matching zeroclaw-main Rust structs exactly |
| `lib/api.ts` | Gateway client — `checkHealth()`, `pair()`, `sendMessage()`, token management |
| `lib/hooks/useConnection.ts` | Polls `/health` every 10s, manages pairing state + bearer token in localStorage |
| `lib/hooks/useChat.ts` | Per-channel message state (Map), sends via `/webhook`, handles agent responses |
| `lib/hooks/useChannels.ts` | Channel list management, active selection, unread counts |
| `app/api/chat/route.ts` | Server-side proxy: POST → gateway `/webhook` (avoids CORS) |
| `app/api/health/route.ts` | Server-side proxy: GET → gateway `/health` |
| `app/api/pair/route.ts` | Server-side proxy: POST → gateway `/pair` |
| `components/ChatContainer.tsx` | Main orchestrator — hooks, mobile drawer sidebar, header, connection status |
| `components/Sidebar.tsx` | Channel list, pairing status button, connection indicators |
| `components/MessageList.tsx` | Role-based message rendering (user/agent/system), typing indicator, auto-scroll |
| `components/MessageInput.tsx` | Auto-resize textarea, Enter to send, safe-area-inset for iPhone |
| `components/PairingDialog.tsx` | Modal for 6-digit pairing code entry |
| `components/ThemeProvider.tsx` | next-themes wrapper |

#### Current Integration Points

- **Health:** `GET /health` → polled every 10s, drives connection indicator in UI
- **Pairing:** `POST /pair` with `X-Pairing-Code` header → bearer token stored in localStorage
- **Chat:** `POST /webhook` with `{"message": "..."}` → `{"response": "...", "model": "..."}`
- **Proxy:** All gateway calls go through Next.js API routes (server-side) to avoid CORS

#### What Works Right Now

1. Frontend renders, connects to gateway, shows connection status
2. Manual pairing via modal dialog (user enters 6-digit code)
3. Send messages through webhook, receive agent responses
4. Per-channel message history (in-memory)
5. Fully responsive — mobile drawer sidebar, safe-area-inset, 100dvh, touch-manipulation

---

## What Needs to Be Built (The Vision)

### Goal: Single Terminal Launch → Auto-Orchestrated Multi-Agent System

The user wants to:
1. Open **one terminal**, run **one command**
2. That command starts the gateway + a **primary orchestrator agent**
3. The **chat UI** is the control plane — the user talks to the orchestrator
4. The orchestrator **spawns additional agents and terminals** on demand
5. **Pairing is automatic** — no manual pin entry

### Architecture Needed

```
┌─────────────────────────────────────────────────┐
│  User's Terminal (single launch point)          │
│  $ zeroclaw daemon --port 8080                  │
│                                                  │
│  ┌─────────────┐  ┌──────────────────────────┐  │
│  │   Gateway    │  │  Orchestrator Agent      │  │
│  │   :8080      │◄─┤  (primary, always-on)    │  │
│  │              │  │                          │  │
│  │  /webhook ───┼──┤  Receives all UI msgs    │  │
│  │  /pair    ───┼──┤  Spawns sub-agents       │  │
│  │  /health  ───┼──┤  Manages terminals       │  │
│  └─────────────┘  └──────┬───────────────────┘  │
│                          │ DelegateTool          │
│                    ┌─────┴─────┐                 │
│                    ▼           ▼                  │
│              ┌──────────┐ ┌──────────┐           │
│              │ Agent A   │ │ Agent B   │          │
│              │ (coder)   │ │ (tester)  │          │
│              └──────────┘ └──────────┘           │
└─────────────────────────────────────────────────┘
         ▲
         │ HTTP (auto-paired)
         │
┌─────────────────────┐
│  zeroclaw-chat UI   │
│  localhost:3000     │
│                     │
│  No manual pairing  │
│  Auto-connects      │
│  Spawn agents from  │
│  chat commands      │
└─────────────────────┘
```

### Specific Changes Needed in zeroclaw-main

#### 1. Auto-Pairing / Pre-Shared Token Mode

**Problem:** Currently, gateway prints a 6-digit code to the terminal and the user must manually enter it in the UI. This breaks the "under the hood" requirement.

**Options (pick one):**

**A. Pre-shared token via config (simplest)**
```toml
[gateway]
port = 8080
require_pairing = true
paired_tokens = ["a-pre-shared-secret-token"]
```
- Frontend `.env.local` gets: `GATEWAY_BEARER_TOKEN=a-pre-shared-secret-token`
- `lib/api.ts` reads token from env, skips pairing flow entirely
- Works today with zero runtime changes

**B. Local-only trust (no pairing for localhost)**
- Add `trust_localhost = true` to GatewayConfig
- Skip bearer token check when request comes from 127.0.0.1
- Cleanest UX, moderate gateway change

**C. Startup token exchange (auto-pair on boot)**
- Daemon writes pairing code to a known file (e.g., `~/.zeroclaw/pairing-code`)
- Frontend reads it and auto-pairs on first health check
- No user interaction, but requires filesystem coordination

#### 2. Agent Spawn Endpoint

**New gateway endpoint:** `POST /agents/spawn`

```json
// Request
{
  "name": "coder-agent",
  "system_prompt": "You are a coding assistant...",
  "provider": "anthropic",
  "model": "claude-sonnet-4-20250514",
  "tools": ["filesystem", "shell", "http"]
}

// Response
{
  "agent_id": "agent-abc123",
  "status": "running"
}
```

**Implementation path:**
- The `DelegateTool` already exists (`src/tools/delegate.rs`) with `DelegateAgentConfig`
- Extend it to be controllable via HTTP, not just via agent-to-agent delegation
- Add agent registry: `HashMap<String, AgentHandle>` in gateway state
- Each spawned agent gets its own tokio task with a message channel

#### 3. Agent Management Endpoints

```
POST   /agents/spawn     — Create and start a new agent
GET    /agents            — List running agents
GET    /agents/:id        — Get agent status/logs
POST   /agents/:id/send   — Send a message to a specific agent
DELETE /agents/:id        — Stop an agent
```

#### 4. Terminal/Process Spawning

For agents that need their own terminal (e.g., running `npm run dev`, `cargo build`):

```
POST /terminals/spawn
{
  "command": "npm run dev",
  "cwd": "/Users/kcdacre8tor/zeroclaw-chat",
  "name": "frontend-dev"
}

// Response
{ "terminal_id": "term-xyz", "pid": 12345 }

GET  /terminals           — List running terminals
GET  /terminals/:id/logs  — Stream terminal output
DELETE /terminals/:id     — Kill terminal process
```

**Implementation:** Use `tokio::process::Command` with stdout/stderr captured into a ring buffer.

#### 5. Frontend Changes (zeroclaw-chat side)

Once the above endpoints exist, the chat UI needs:

- **Auto-connect on boot** — skip PairingDialog if token is pre-shared or localhost-trusted
- **Agent panel** in sidebar — show running agents with status indicators
- **Terminal panel** — tabbed terminal output viewer (read-only log stream)
- **Chat commands** — orchestrator agent interprets natural language:
  - "spin up a coding agent" → POST /agents/spawn
  - "start the frontend server" → POST /terminals/spawn
  - "show me agent logs" → GET /agents/:id with streaming

---

## Existing Capabilities to Leverage

| Capability | Location | Notes |
|-----------|----------|-------|
| DelegateTool | `src/tools/delegate.rs` | Agent-to-agent delegation, max_depth=3 |
| Component Supervision | `src/daemon/mod.rs` | Exponential backoff restart for crashed components |
| PairingGuard | `src/security/pairing.rs` | SHA-256 token hashing, brute-force lockout |
| Rate Limiter | `src/gateway/mod.rs` | Per-client sliding window, configurable per-minute |
| Idempotency Store | `src/gateway/mod.rs` | Prevents duplicate processing, configurable TTL |
| Channel Trait | `src/channels/traits.rs` | `Channel::listen()` + `Channel::send()` pattern |
| Memory System | `src/memory/` | Auto-recall + autosave on all message flows |
| Health Reporting | `src/health/` | Component-level status tracking, `/health` endpoint |

---

## Config Changes for Immediate Use

To get auto-pairing working right now without any code changes:

**`~/.zeroclaw/config.toml`** (or wherever your config lives):
```toml
[gateway]
port = 8080
host = "127.0.0.1"
require_pairing = true
# Pre-shared token — paste this same value into zeroclaw-chat/.env.local
paired_tokens = ["zeroclaw-local-dev-token-2026"]
```

**`/Users/kcdacre8tor/zeroclaw-chat/.env.local`**:
```
NEXT_PUBLIC_GATEWAY_URL=http://localhost:8080
GATEWAY_URL=http://localhost:8080
GATEWAY_BEARER_TOKEN=zeroclaw-local-dev-token-2026
```

Then update `zeroclaw-chat/lib/api.ts` to read `GATEWAY_BEARER_TOKEN` from env and use it automatically instead of going through the pairing flow.

---

## Priority Order

1. **Auto-pairing** (pre-shared token) — unblocks seamless UX immediately
2. **Agent spawn endpoint** — enables orchestrator to create sub-agents via HTTP
3. **Terminal spawn endpoint** — enables orchestrator to launch processes
4. **Agent management endpoints** — list, status, stop
5. **Frontend agent/terminal panels** — UI for monitoring spawned agents

---

## Running Services

| Service | Port | Command | Status |
|---------|------|---------|--------|
| Next.js Frontend | 3000 | `npm run dev` (in zeroclaw-chat) | Running |
| ZeroClaw Gateway | 8080 | `cargo run -- gateway --port 8080` | Running |

Both are currently live and communicating. The Next.js proxy routes successfully forward to the gateway.
