# Multi-stage build for liteclaw.
#
# Build:   docker build -t liteclaw .
# Run:     docker run --rm -p 8080:8080 -v "$PWD":/workspace liteclaw serve --host 0.0.0.0 8080
# Compose: docker compose up

# ─── Stage 1: builder ────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

WORKDIR /app

# Copy manifests first for dependency caching.
COPY Cargo.toml Cargo.lock ./
COPY rust-toolchain.toml ./
COPY crates/ ./crates/

# Build the release binary.
RUN cargo build --release && cp target/release/lc /lc

# ─── Stage 2: runtime ────────────────────────────────────────────
# debian-slim + bash: bash is REQUIRED for skill-run (skill scripts are bash).
FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        bash ca-certificates curl python3 poppler-utils && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /lc /usr/local/bin/lc

# Default workspace (compose mounts host dir here).
WORKDIR /workspace

ENTRYPOINT ["lc"]
CMD ["serve", "--host", "0.0.0.0", "8080"]
