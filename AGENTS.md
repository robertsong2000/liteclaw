# liteclaw

## What is this?

`liteclaw` ("小龙虾") is a lightweight, single-binary Rust toolkit — the lean
execution endpoint of the claw ecosystem. Each subcommand (`lc <claw>`) is an
independently usable "claw" (tool) that can also be composed via pipes.

## Architecture in one paragraph

A `Claw` trait (`liteclaw-core`) defines the contract for every tool. A shared
`Ctx` injects the Defender security kernel + sandbox + I/O into every claw, so
security scanning is built-in, not bolted-on. Crates: `core` (kernel), `fs`
(file tools), `model` (LLM client), `cli` (dispatch).

## Conventions

- **Comments in English**, conversation in Chinese. Keep the codebase
  internationally shareable.
- One responsibility per file. A "claw" must be usable standalone AND
  composable via stdin/stdout/--json.
- Lean-first: avoid heavy deps. Prefer pure-Rust crates (rustls over openssl,
  `ignore` over full ripgrep). Every added dependency must justify its binary
  size cost.
- Security is a pre-check, not an afterthought: mutating claws run input
  through the Defender before acting.

## Build & test

```bash
cargo build --release          # single binary at target/release/lc
cargo test                     # unit + integration (tests/smoke.rs)
./target/release/lc --help
```

## Status

MVP: `lc read | grep | edit | audit` + Defender kernel. `lc chat`/ACP/MCP are
scaffolded for future milestones — see DESIGN.md.
