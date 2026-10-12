FROM rust:1-slim AS builder

WORKDIR /app

# Workspace now spans multiple crates (crates/armadai-*), so the old
# single-crate "stub src/main.rs to cache deps" trick no longer applies
# cleanly. Copy the whole workspace and build the bin package directly.
# (Base image floated to `rust:1-slim` (latest stable): pinning to a fixed
# patch version lets it fall behind current deps.)
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release -p armadai

# Debian release matched to `rust:1-slim`'s base (trixie) — the old
# bookworm-slim runtime failed at startup with a GLIBC version mismatch
# against binaries built by the newer builder image.
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/armadai /usr/local/bin/armadai

ENTRYPOINT ["armadai"]
