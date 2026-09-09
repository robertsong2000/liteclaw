# Multi-stage build for liteclaw.
#
# Build:   docker build -t liteclaw .
# Run:     docker run --rm -p 8080:8080 -v "$PWD":/workspace liteclaw serve --host 0.0.0.0 8080
# Compose: docker compose up

# ─── Stage 1: builder ────────────────────────────────────────────
# 1-bookworm (not the old 1.82): Cargo.lock resolves transitive crates
# (e.g. rand_pcg 0.10.x) whose manifests cargo <1.85 cannot parse. Stay on
# bookworm to match the runtime stage's glibc.
FROM rust:1-bookworm AS builder

WORKDIR /app

# Copy manifests first for dependency caching.
# (rust-toolchain.toml is deliberately NOT copied: it requests channel
# "stable", which makes rustup phone home to static.rust-lang.org during
# the build. The base image already ships the exact toolchain.)
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Cargo registry via rsproxy mirror: direct crates.io is intermittently
# connection-reset from this network (builds died at "Updating crates.io
# index" even with cargo's own retries).
RUN printf '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n\n[source.rsproxy-sparse]\nregistry = "sparse+https://rsproxy.cn/index/"\n\n[net]\nretry = 5\n' \
    > /usr/local/cargo/config.toml

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
