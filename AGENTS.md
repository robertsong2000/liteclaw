# liteclaw

## What is this?

`liteclaw` ("小龙虾") is a lightweight, single-binary Rust toolkit — a local
AI coding assistant with a built-in Web UI, Agent Loop, and Defender security
kernel. ~3MB binary, zero runtime dependencies.

## Three run modes

- `lc serve [--host 0.0.0.0] [PORT]` — Web UI (chat + agent loop + login + history)
- `lc mcp` — MCP server (stdio JSON-RPC, for Claude Code / Cursor)
- `lc read | grep | glob | audit | skills | skill-run` — standalone CLI tools

## Architecture

7 crates:
- `core` — Claw trait, Ctx, Defender security kernel, Sandbox
- `fs` — read / grep / edit
- `model` — streaming OpenAI-compatible client (reqwest + rustls)
- `skills` — SKILL.md parsing + discovery + execution (78+ skills)
- `agent` — Agent Loop + 11 tools + hooks + backup + MCP client
- `web` — axum HTTP server + embedded frontend + auth + history
- `cli` — main, clap dispatch, MCP server entry

## Agent tools (11)

The model can autonomously call these in a conversation:
- `read(path)` / `grep(pattern,path)` / `glob(pattern,path)` — read & search
- `audit(path)` — security scan
- `fetch(url)` — web download (SSRF-protected)
- `edit(path,old,new)` / `write(path,content)` — modify files
- `bash(command)` — shell execution (Defender blocks dangerous commands)
- `skill_list()` / `skill_run(id,args)` — discover & run skills
- `undo()` — rollback the most recent write/edit

## Hooks lifecycle

Pluggable interceptors around tool execution:
- `PreToolUse` — before a tool runs (DefenderHook blocks danger, BackupHook
  snapshots files)
- `PostToolUse` — after a tool runs (LogHook records the call)
- Default chain: Defender → Backup → Log

## Security model

Three layers, all built-in:
1. **Defender** — rule-based pre-check (prompt injection, command injection,
   SSRF, path traversal, credential exfil). Score ≥90 = block.
2. **Sandbox** — write whitelist (`--allow-write`), default read-only, `..`
   traversal rejected.
3. **rustls TLS** — no openssl, pure-Rust TLS for model API calls.

## Conventions

- **Comments in English**, conversation in Chinese.
- One responsibility per file. A "claw" is usable standalone AND composable
  via stdin/stdout/--json.
- Lean-first: prefer pure-Rust crates (rustls over openssl, `ignore` over full
  ripgrep). Every dependency must justify its binary size cost.
- Security is a pre-check, not an afterthought.

## Build & test

```bash
cargo build --release          # → target/release/lc (~3MB)
cargo test                     # unit + integration tests
cargo clippy --all-targets     # zero warnings enforced
./target/release/lc --help
./target/release/lc serve 9999 # Web UI at http://localhost:9999
```

## Config files

- `~/.liteclaw/config.json` — model config (base_url, api_key, model)
- `~/.liteclaw/auth.json` — login credentials (default: renault/renault123)
- `~/.liteclaw/history.json` — conversation sessions
- `~/.liteclaw/backups/` — file snapshots before write/edit (for undo)
- `~/.liteclaw/mcp.json` — external MCP server config

## Deployment

- **Docker**: `docker compose up -d` → http://localhost:8080
- **CI**: GitHub Actions (fmt + clippy + test + release + size assertion <5MB)
