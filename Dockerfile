#
# Multi-stage build for liteclaw.
#
# Stage 1 (builder): Rust toolchain, compiles the release binary against glibc.
# Stage 2 (runtime): debian-slim + bash. bash is REQUIRED — skill-run executes
#   skill scripts (mostly bash), so a shell-less image (distroless/scratch)
#   would break the skill system. glibc is chosen over musl/alpine for the
#   widest crate compatibility and zero-quirk compilation.
#
# Build:   docker build -t liteclaw .
# Run:     docker run --rm -v "$PWD":/workspace liteclaw read README.md
# Compose: docker compose run --rm lc read README.md

# ─── Stage 1: builder ────────────────────────────────────────────
FROM rust:1.74-bookworm AS builder

# Compile-time deps go here if ever needed (e.g. perl for some build scripts).
# glibc targets need nothing extra today.

WORKDIR /app

# Cache deps independently of source: copy manifests first, then a dummy build
# so `cargo build` populates the registry/target cache. Subsequent source-only
# changes rebuild fast.
COPY Cargo.toml Cargo.lock ./
COPY rust-toolchain.toml ./
COPY crates/ ./crates/

# Build the release binary. --workspace ensures all member crates compile;
# the CLI binary (`lc`) is what we ship.
RUN cargo build --release && cp target/release/lc /lc

# ─── Stage 2: runtime ────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# bash: required by `lc skill-run` (skill scripts are overwhelmingly bash —
# using sh would break process substitution, a known footgun documented in
# DESIGN.md §12). ca-certificates: needed by the model client (rustls) for
# TLS verification against cloud API endpoints. curl: convenience for health
# checks / debugging inside the container.
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /lc /usr/local/bin/lc

# Default workspace: compose mounts the host cwd here.
WORKDIR /workspace

# ENTRYPOINT = the binary itself, so `docker run ... liteclaw read x`
# works the same as `lc read x`. CMD gives a sane default (--help).
ENTRYPOINT ["lc"]
CMD ["--help"]
