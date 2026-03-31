# ─── Stage 1: Builder ────────────────────────────────────────────────────────
FROM rust:1.80-bookworm AS builder

WORKDIR /app

# Cache dependencies by copying manifests first
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/truenorth-core/Cargo.toml crates/truenorth-core/Cargo.toml
COPY crates/truenorth-llm/Cargo.toml crates/truenorth-llm/Cargo.toml
COPY crates/truenorth-memory/Cargo.toml crates/truenorth-memory/Cargo.toml
COPY crates/truenorth-tools/Cargo.toml crates/truenorth-tools/Cargo.toml
COPY crates/truenorth-skills/Cargo.toml crates/truenorth-skills/Cargo.toml
COPY crates/truenorth-visual/Cargo.toml crates/truenorth-visual/Cargo.toml
COPY crates/truenorth-orchestrator/Cargo.toml crates/truenorth-orchestrator/Cargo.toml
COPY crates/truenorth-web/Cargo.toml crates/truenorth-web/Cargo.toml
COPY crates/truenorth-cli/Cargo.toml crates/truenorth-cli/Cargo.toml

# Create dummy src files to build deps only
RUN for crate in core llm memory tools skills visual orchestrator web cli; do \
      mkdir -p crates/truenorth-$crate/src && \
      echo "// placeholder" > crates/truenorth-$crate/src/lib.rs; \
    done

# Pre-build dependencies
RUN cargo build --release --workspace 2>/dev/null || true

# Copy real source
COPY crates/ crates/
COPY config.toml.example .env.example ./

# Build the actual binary
RUN cargo build --release -p truenorth-cli

# ─── Stage 2: Runtime ────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -s /bin/bash truenorth

COPY --from=builder /app/target/release/truenorth /usr/local/bin/truenorth

# Default data directory
RUN mkdir -p /data/truenorth && chown truenorth:truenorth /data/truenorth

USER truenorth
WORKDIR /data/truenorth

ENV TRUENORTH_DATA_DIR=/data/truenorth
EXPOSE 8080

ENTRYPOINT ["truenorth"]
CMD ["serve", "--host", "0.0.0.0", "--port", "8080"]
