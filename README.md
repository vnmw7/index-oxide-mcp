# Index Oxide MCP

Codebase indexing using Rust, Qdrant, and an Embedding Model.

Index Oxide MCP is a high-throughput MCP (Model Context Protocol) codebase indexer built for agentic AI workflows. It generates context embeddings before dumping them into a Qdrant vector database.

## Features
- **High-throughput Indexing:** Concurrently walks and indexes large codebases.
- **Tree-sitter Parsing:** Precise syntax tree extraction for multiple languages (Rust, Python, TS, Go).
- **Vector Search:** Semantically retrieves code blocks using Qdrant.
- **MCP Server:** Dual-mode support (SSE Web Server and stdio) via the native `rmcp` SDK.

## Setup & Deployment

Index Oxide MCP is designed to run natively as a standalone binary alongside a containerized Qdrant database. You can choose to download a pre-built binary or compile it from source. The server supports dual-transport modes: Stdio (standard I/O) and SSE (HTTP).

### Prerequisites
- **Docker**: Required to run the Qdrant vector database.
- **Gemini API key**: Required. Set via `GEMINI_API_KEY`.
- **Index Oxide MCP Binary**: Either download the pre-built binary for your OS from the releases page or install [Rust](https://rustup.rs/) to compile from source.

### 1. Build the Binary

The server is compiled as a single binary. You do not need separate builds for `stdio` and `sse`.

```sh
cargo build --release
```

Binary locations after a successful build:

- Linux/macOS: `./target/release/index-oxide-mcp`
- Windows: `.\target\release\index-oxide-mcp.exe`

### 2. Start the Database
The easiest way to start the Qdrant database is using Docker Compose. Run the following command in the project root:

```sh
docker-compose up -d
```

By default, the server expects Qdrant at `http://localhost:6334`, which matches [`docker-compose.yml`](./docker-compose.yml).

### 3. Configure Environment Variables

Only `GEMINI_API_KEY` is required. Everything else has defaults.

Supported runtime environment variables:

- `GEMINI_API_KEY`: Required. Google Gemini API key.
- `QDRANT_URL`: Optional. Defaults to `http://localhost:6334`.
- `OXI_SERVER_HOST`: This tells the app where to bind the SSE host, though it's entirely optional. If you ignore it, it defaults to catching traffic on 0.0.0.0.
- `OXI_SERVER_PORT`: Optional. SSE bind port. Defaults to `8754`.
- `OXI_EMBEDDING_MODEL`: Optional. Defaults to `gemini-embedding-2-preview`.
- `OXI_EMBEDDING_DIMENSIONS`: Optional. Defaults to `3072`.

### 4. Choose a Transport Mode

The same binary supports both transport modes at runtime:

- `stdio`: Best for local MCP clients that launch the server themselves.
- `sse`: Best when you want to run the server as a background HTTP service.

If you do not pass `--transport`, the default is `stdio`.

#### Mode A: `stdio` Transport

Use this for local clients such as Claude Desktop, Cursor, or other agentic CLIs that spawn the MCP server as a child process over stdin/stdout.

1. Build the project once with `cargo build --release`, or use a prebuilt binary.
2. Point your client at the compiled binary.
3. Pass `--transport stdio` in the client config.

Example MCP client config:

```json
{
  "mcpServers": {
    "index-oxide": {
      "command": "/absolute/path/to/index-oxide-mcp",
      "args": ["--transport", "stdio"],
      "env": {
        "GEMINI_API_KEY": "your_gemini_api_key_here",
        "QDRANT_URL": "http://localhost:6334"
      }
    }
  }
}
```

Path examples:

- Windows: `D:\\projects\\index-oxide-mcp\\target\\release\\index-oxide-mcp.exe`
- Linux/macOS: `/absolute/path/to/index-oxide-mcp/target/release/index-oxide-mcp`

Notes:

- Keep logs on `stderr`; MCP traffic uses `stdin`/`stdout` in this mode.
- `stdio` is the recommended default for local desktop agents.

#### Mode B: `sse` Transport

Use this when you want to run Index Oxide MCP as a standalone HTTP service and connect to it over the network or from a client that prefers URL-based MCP servers.

Start the server from your terminal.

Linux/macOS:

```sh
export GEMINI_API_KEY="your_api_key_here"
export QDRANT_URL="http://localhost:6334"
./target/release/index-oxide-mcp --transport sse
```

One-line alternative:

```sh
GEMINI_API_KEY="your_api_key_here" QDRANT_URL="http://localhost:6334" ./target/release/index-oxide-mcp --transport sse
```

Windows PowerShell:

```powershell
$env:GEMINI_API_KEY="your_api_key_here"
$env:QDRANT_URL="http://localhost:6334"
.\target\release\index-oxide-mcp.exe --transport sse
```

One-line alternative:

```powershell
$env:GEMINI_API_KEY="your_api_key_here"; $env:QDRANT_URL="http://localhost:6334"; .\target\release\index-oxide-mcp.exe --transport sse
```

Default SSE endpoints:

- MCP endpoint: `http://localhost:8754/mcp`
- Health endpoint: `http://localhost:8754/health`

Example MCP client config for SSE mode:

```json
{
  "mcpServers": {
    "index-oxide": {
      "url": "http://localhost:8754/mcp"
    }
  }
}
```

You can override the listen address with:

```sh
OXI_SERVER_HOST=127.0.0.1 OXI_SERVER_PORT=8754 ./target/release/index-oxide-mcp --transport sse
```

### 5. Quick Start Summary

For most local users:

1. Run `docker-compose up -d`
2. Run `cargo build --release`
3. Add the `stdio` config to your MCP client
4. Set `GEMINI_API_KEY` in the client config
5. Restart your MCP client

## Development

See [CONTRIBUTING.md](./CONTRIBUTING.md) for how to build, test, and contribute to the repository.
