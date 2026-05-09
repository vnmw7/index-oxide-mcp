# Index Oxide MCP

Codebase indexing using Rust, Qdrant, and an Embedding Model.

Index Oxide MCP is a high-throughput MCP (Model Context Protocol) codebase indexer built for agentic AI workflows. It generates context embeddings before dumping them into a Qdrant vector database.

## Features
- **High-throughput Indexing:** Concurrently walks and indexes large codebases.
- **Tree-sitter Parsing:** Precise syntax tree extraction for multiple languages (Rust, Python, TS, Go).
- **Vector Search:** Semantically retrieves code blocks using Qdrant.
- **MCP Server:** Dual-mode support (Streamable HTTP and Stdio) via the native `rmcp` SDK.

## Setup & Deployment

Index Oxide MCP is designed to run natively as a standalone binary alongside a containerized Qdrant database. You can choose to download a pre-built binary or compile it from source. The server supports dual-transport modes: Stdio (standard I/O) and Streamable HTTP (historically called SSE).

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

If you want to download and run the Qdrant image directly instead of using the repository compose file, pull the official image first:

```sh
docker pull qdrant/qdrant
```

Linux/macOS:

```sh
mkdir -p ./qdrant_storage
docker run -d --name oxi-qdrant \
  --restart unless-stopped \
  -p 6333:6333 \
  -p 6334:6334 \
  -v "$(pwd)/qdrant_storage:/qdrant/storage" \
  qdrant/qdrant
```

Windows PowerShell:

```powershell
New-Item -ItemType Directory -Force .\qdrant_storage | Out-Null
docker run -d --name oxi-qdrant `
  --restart unless-stopped `
  -p 6333:6333 `
  -p 6334:6334 `
  -v "${PWD}/qdrant_storage:/qdrant/storage" `
  qdrant/qdrant
```

If you do not want to use Docker, download Qdrant directly from the official GitHub releases page, extract the archive, and run the `qdrant` executable. The `/latest/download/` links below track the current stable release:

- Linux x86_64: <https://github.com/qdrant/qdrant/releases/latest/download/qdrant-x86_64-unknown-linux-musl.tar.gz>
- Linux ARM64: <https://github.com/qdrant/qdrant/releases/latest/download/qdrant-aarch64-unknown-linux-musl.tar.gz>
- macOS Apple Silicon: <https://github.com/qdrant/qdrant/releases/latest/download/qdrant-aarch64-apple-darwin.tar.gz>
- macOS Intel: <https://github.com/qdrant/qdrant/releases/latest/download/qdrant-x86_64-apple-darwin.tar.gz>
- Windows x86_64: <https://github.com/qdrant/qdrant/releases/latest/download/qdrant-x86_64-pc-windows-msvc.zip>

Linux/macOS after extracting:

```sh
./qdrant
```

Windows PowerShell after extracting:

```powershell
.\qdrant.exe
```

For easier Windows startup, place `qdrant.exe` in the project root and run the included PowerShell launcher:

```powershell
.\start-qdrant.ps1
```

The script runs the equivalent of:

```powershell
$env:QDRANT__STORAGE__STORAGE_PATH=".\qdrant_data"; .\qdrant.exe
```

When using the launcher, Qdrant stores data in `./qdrant_data`. Without the launcher, Qdrant's default local quickstart storage path is `./qdrant_storage`. In both cases, Qdrant exposes REST on `6333` plus gRPC on `6334`, so keep `QDRANT_URL=http://localhost:6334` for Index Oxide MCP.

Qdrant ports used by this project:

- `6334`: gRPC. Index Oxide MCP uses this through `QDRANT_URL`.
- `6333`: HTTP/REST. Useful for health checks and Qdrant's local dashboard/API.

Production note: Qdrant's official documentation recommends managed Qdrant Cloud, Kubernetes, or a carefully operated Docker/Compose deployment for production. If you self-host with Docker/Compose, use persistent SSD/NVMe-backed storage, restrict network access, configure security settings, and plan backup/restore, monitoring, and upgrades.

### 3. Configure Environment Variables

Only `GEMINI_API_KEY` is required. Everything else has defaults.

Supported runtime environment variables:

- `GEMINI_API_KEY`: Required. Google Gemini API key.
- `QDRANT_URL`: Optional. Defaults to `http://localhost:6334`.
- `OXI_SERVER_HOST`: This tells the app where to bind the Streamable HTTP host, though it's entirely optional. If you ignore it, it defaults to catching traffic on 0.0.0.0.
- `OXI_SERVER_PORT`: Optional. Streamable HTTP bind port. Defaults to `8754`.
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
3. Point your client at the binary. `stdio` is the default transport, so no transport argument is required.

*Configuration details for stdio clients can be found in the **Supported MCP Clients** section below.*

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

*Configuration details for SSE clients can be found in the **Supported MCP Clients** section below.*

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

## Supported MCP Clients

Below are the minimum configuration schemas for popular agentic clients. Replace `/absolute/path/to/index-oxide-mcp` with the actual path to your compiled binary, and insert your real `GEMINI_API_KEY`.

Use one of two integration styles:

- `stdio`: the client starts the Index Oxide MCP binary and must receive `GEMINI_API_KEY` in its local server environment.
- `sse` service mode: start Index Oxide MCP yourself with `--transport sse`, then point the client at `http://localhost:8754/mcp`.

The project CLI still names the HTTP service mode `sse`, but the current implementation exposes RMCP streamable HTTP at `/mcp`. If a client distinguishes old SSE from Streamable HTTP, choose `http`, `remote`, or `Streamable HTTP` for `http://localhost:8754/mcp`.

### Claude Desktop

Stdio config for `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "index-oxide": {
      "command": "/absolute/path/to/index-oxide-mcp",
      "env": {
        "GEMINI_API_KEY": "your_gemini_api_key_here"
      }
    }
  }
}
```

SSE service config through `mcp-remote`:

```json
{
  "mcpServers": {
    "index-oxide": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "http://localhost:8754/mcp"]
    }
  }
}
```

### Cursor

Stdio config for `.cursor/mcp.json` or `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "index-oxide": {
      "type": "stdio",
      "command": "/absolute/path/to/index-oxide-mcp",
      "env": {
        "GEMINI_API_KEY": "your_gemini_api_key_here"
      }
    }
  }
}
```

SSE service config for `.cursor/mcp.json` or `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "index-oxide": {
      "url": "http://localhost:8754/mcp"
    }
  }
}
```

### Gemini CLI

Stdio config for `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "index-oxide": {
      "command": "/absolute/path/to/index-oxide-mcp",
      "env": {
        "GEMINI_API_KEY": "your_gemini_api_key_here"
      }
    }
  }
}
```

SSE service config for `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "index-oxide": {
      "httpUrl": "http://localhost:8754/mcp"
    }
  }
}
```

Gemini CLI can also write these settings for you. Use exactly one of these commands:

Direct `stdio` mode, where Gemini starts the Index Oxide MCP binary and therefore must inject the required server environment:

```sh
gemini mcp add --env GEMINI_API_KEY=your_gemini_api_key_here index-oxide /absolute/path/to/index-oxide-mcp
```

HTTP service mode, where Index Oxide MCP is already running with `--transport sse`; the client only needs the `/mcp` URL:

```sh
gemini mcp add --transport http index-oxide http://localhost:8754/mcp
```

`gemini mcp add` defaults to project scope. Add `--scope user` only when you intentionally want to write the server to your global Gemini config. Do not pass `--env` to the HTTP service command unless the remote MCP server specifically requires client-side headers or auth; Index Oxide reads `GEMINI_API_KEY` from the process that runs `index-oxide-mcp --transport sse`.

### OpenAI Codex

Stdio config for `~/.codex/config.toml` or your project-scoped TOML:

```toml
[mcp_servers.index-oxide]
command = "/absolute/path/to/index-oxide-mcp"

[mcp_servers.index-oxide.env]
GEMINI_API_KEY = "your_gemini_api_key_here"
```

SSE service config for `~/.codex/config.toml` or your project-scoped TOML:

```toml
[mcp_servers.index-oxide]
url = "http://localhost:8754/mcp"
```

Codex CLI can also write these settings for you:

```sh
codex mcp add index-oxide --env GEMINI_API_KEY=your_gemini_api_key_here -- /absolute/path/to/index-oxide-mcp
codex mcp add index-oxide --url http://localhost:8754/mcp
```

### OpenCode

Stdio config for `opencode.json`:

```json
{
  "mcp": {
    "index-oxide": {
      "type": "local",
      "command": ["/absolute/path/to/index-oxide-mcp"],
      "environment": {
        "GEMINI_API_KEY": "your_gemini_api_key_here"
      }
    }
  }
}
```

SSE service config for `opencode.json`:

```json
{
  "mcp": {
    "index-oxide": {
      "type": "remote",
      "url": "http://localhost:8754/mcp"
    }
  }
}
```

### Kilo Code

Stdio config for `kilo.jsonc` or `.kilo/kilo.jsonc`:

```jsonc
{
  "mcp": {
    "index-oxide": {
      "type": "local",
      "command": ["/absolute/path/to/index-oxide-mcp"],
      "environment": {
        "GEMINI_API_KEY": "your_gemini_api_key_here"
      }
    }
  }
}
```

SSE service config matching the `mcp-remote` bridge shape commonly written by Kilo Code:

```jsonc
{
  "mcp": {
    "index-oxide": {
      "type": "local",
      "command": [
        "npx",
        "-y",
        "mcp-remote",
        "http://localhost:8754/mcp"
      ]
    }
  }
}
```

If your installed Kilo Code version supports direct remote MCP entries in the UI, the equivalent direct remote form is:

```jsonc
{
  "mcp": {
    "index-oxide": {
      "type": "remote",
      "url": "http://localhost:8754/mcp"
    }
  }
}
```

## Live / Production MCP Use

Use this flow when you want to run the built MCP binary against a persistent Qdrant instance instead of running ad hoc development commands.

### 1. Build or Download the MCP Binary

For source builds:

```sh
cargo build --release
```

Use the release binary from one of these paths:

- Windows: `D:\projects\index-oxide-mcp\target\release\index-oxide-mcp.exe`
- Linux/macOS: `/absolute/path/to/index-oxide-mcp/target/release/index-oxide-mcp`

If you are using a published release artifact instead, place the downloaded binary in a stable path such as `/opt/index-oxide-mcp/index-oxide-mcp` or `C:\Tools\index-oxide-mcp\index-oxide-mcp.exe` and point your MCP client to that exact file.

### 2. Run Qdrant as a Persistent Service

Recommended repository-local option:

```sh
docker-compose up -d qdrant
```

Direct Docker option after pulling the image:

```sh
docker pull qdrant/qdrant
docker run -d --name oxi-qdrant --restart unless-stopped -p 6333:6333 -p 6334:6334 -v "$(pwd)/qdrant_storage:/qdrant/storage" qdrant/qdrant
```

Windows PowerShell direct Docker option:

```powershell
docker pull qdrant/qdrant
New-Item -ItemType Directory -Force .\qdrant_storage | Out-Null
docker run -d --name oxi-qdrant --restart unless-stopped -p 6333:6333 -p 6334:6334 -v "${PWD}/qdrant_storage:/qdrant/storage" qdrant/qdrant
```

Verify Qdrant is reachable:

```sh
curl http://localhost:6333/health
```

The MCP server should still use `QDRANT_URL=http://localhost:6334` because this project talks to Qdrant through gRPC.

### 3. Use the Built MCP Binary with a Local Client

For live local use, `stdio` is the recommended production client integration because the MCP client owns the child process lifecycle and communicates over stdin/stdout.

Windows client config example:

```json
{
  "mcpServers": {
    "index-oxide": {
      "command": "D:\\projects\\index-oxide-mcp\\target\\release\\index-oxide-mcp.exe",
      "env": {
        "GEMINI_API_KEY": "your_gemini_api_key_here"
      }
    }
  }
}
```

Linux/macOS client config example:

```json
{
  "mcpServers": {
    "index-oxide": {
      "command": "/absolute/path/to/index-oxide-mcp/target/release/index-oxide-mcp",
      "env": {
        "GEMINI_API_KEY": "your_gemini_api_key_here"
      }
    }
  }
}
```

After restarting the MCP client, use the `search_codebase` tool with a request like:

```json
{
  "query": "where is the Qdrant collection created",
  "repo": "index-oxide-mcp",
  "limit": 5
}
```

Optional filters supported by the current search tool include `language`, `path_prefix`, `symbol_kind`, `repo`, and `limit`.

### 4. Run the Built MCP Binary as an HTTP Service

Use `sse` mode when a client needs a URL-based MCP endpoint or when the server should run as a long-lived background service.

Linux/macOS:

```sh
export GEMINI_API_KEY="your_gemini_api_key_here"
export QDRANT_URL="http://localhost:6334"
export OXI_SERVER_HOST="127.0.0.1"
export OXI_SERVER_PORT="8754"
./target/release/index-oxide-mcp --transport sse
```

Windows PowerShell:

```powershell
$env:GEMINI_API_KEY="your_gemini_api_key_here"
$env:QDRANT_URL="http://localhost:6334"
$env:OXI_SERVER_HOST="127.0.0.1"
$env:OXI_SERVER_PORT="8754"
.\target\release\index-oxide-mcp.exe --transport sse
```

Validate the running service:

```sh
curl http://localhost:8754/health
```

Connect URL-based MCP clients through:

```text
http://localhost:8754/mcp
```

Use the direct remote client config from the **Supported MCP Clients** section when your client supports Streamable HTTP. For stdio-only clients, or for Kilo Code setups that use the same bridge shape shown above, bridge the running HTTP service with `mcp-remote`:

```json
{
  "mcpServers": {
    "index-oxide": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "http://localhost:8754/mcp"]
    }
  }
}
```

### 5. Production Operating Checklist

- Keep the release binary path stable so MCP client configs do not drift.
- Keep `GEMINI_API_KEY` out of source control; inject it through the MCP client, service manager, or secret manager.
- Bind `OXI_SERVER_HOST=127.0.0.1` unless remote clients must connect. If remote access is required, put authentication, TLS, and firewall rules in front of the service.
- Persist Qdrant storage with a Docker volume or host directory and back it up.
- Monitor Qdrant HTTP health on `6333` and Index Oxide MCP health on `8754` when using HTTP mode.
- Keep Qdrant and the MCP binary on the same trusted network. Do not expose Qdrant ports publicly without Qdrant security controls.
- Current live MCP tool surface: `search_codebase`. Confirm indexed data exists in Qdrant before expecting search results.
- Official Qdrant installation reference: <https://qdrant.tech/documentation/operations/installation/>

## Development

See [CONTRIBUTING.md](./CONTRIBUTING.md) for how to build, test, and contribute to the repository.
