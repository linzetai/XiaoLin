# ─── Build stage ─────────────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

WORKDIR /build

# Cache dependencies: copy manifests first, then build a dummy to cache deps
COPY Cargo.toml Cargo.lock ./
COPY crates crates

RUN cargo build --release --bin fastclaw \
    && strip target/release/fastclaw

# ─── Runtime stage ───────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        sqlite3 \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -r fastclaw && useradd -r -g fastclaw -m fastclaw

WORKDIR /app

COPY --from=builder /build/target/release/fastclaw /usr/local/bin/fastclaw
COPY config/ /app/config/

RUN mkdir -p /app/data /app/logs && chown -R fastclaw:fastclaw /app

USER fastclaw

ENV RUST_LOG=info
ENV FASTCLAW_STATE_DIR=/app

EXPOSE 18789

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD fastclaw health || exit 1

ENTRYPOINT ["fastclaw"]
CMD ["serve"]
