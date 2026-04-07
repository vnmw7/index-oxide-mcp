# System: Index Oxide MCP
# File URL: oxidized-index-mcp/Dockerfile
# Purpose: Multi-stage build for production SSE deployment

# Stage 1: Build with latest Rust compiler for best LLVM codegen
FROM rust:1.94-slim-bookworm AS builder
WORKDIR /app

# Install build dependencies for tree-sitter (C compilation) and TLS
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config libssl-dev build-essential && \
    rm -rf /var/lib/apt/lists/*

# Cache dependencies by building with dummy main first
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release 2>/dev/null || true && \
    rm -rf src

# Build actual binary
COPY src/ src/
RUN touch src/main.rs && cargo build --release

# Stage 2: Minimal runtime with Debian Trixie for optimal glibc perf
FROM debian:trixie-slim AS runtime
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/index-oxide-mcp /usr/local/bin/index-oxide-mcp

EXPOSE 8754

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD curl -f http://localhost:8754/health || exit 1

ENTRYPOINT ["index-oxide-mcp", "--transport", "sse"]
