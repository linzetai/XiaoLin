# ─── Build stage ─────────────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

WORKDIR /build

# Cache dependencies: copy manifests first, then build a dummy to cache deps
COPY Cargo.toml Cargo.lock ./
COPY crates crates

RUN cargo build --release --bin xiaolin \
    && strip target/release/xiaolin

# ─── Runtime stage ───────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        sqlite3 \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -r xiaolin && useradd -r -g xiaolin -m xiaolin

WORKDIR /app

COPY --from=builder /build/target/release/xiaolin /usr/local/bin/xiaolin
COPY config/ /app/config/

RUN mkdir -p /app/data /app/logs && chown -R xiaolin:xiaolin /app

USER xiaolin

ENV RUST_LOG=info
ENV XIAOLIN_STATE_DIR=/app

EXPOSE 18789

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD xiaolin health || exit 1

ENTRYPOINT ["xiaolin"]
CMD ["serve"]
