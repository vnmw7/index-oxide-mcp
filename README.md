# Index Oxide MCP

Codebase indexing using Rust, Qdrant, and an Embedding Model.

Index Oxide MCP is a high-throughput MCP (Model Context Protocol) codebase indexer built for agentic AI workflows. It generates context embeddings before dumping them into a Qdrant vector database.

## Features
- **High-throughput Indexing:** Concurrently walks and indexes large codebases.
- **Tree-sitter Parsing:** Precise syntax tree extraction for multiple languages (Rust, Python, TS, Go).
- **Vector Search:** Semantically retrieves code blocks using Qdrant.
- **MCP Server:** Dual-mode support (Streamable HTTP and Stdio) via the native `rmcp` SDK.
- **Interactive Index Manager:** Terminal UI for starting index jobs and viewing indexed repositories.

## Setup & Deployment

Index Oxide MCP is designed to run natively as a standalone binary alongside a containerized Qdrant database. You can choose to download a pre-built binary or compile it from source. The binary has two top-level subcommands:

- `serve`: starts the MCP server. This is the command to use from MCP clients and supports Stdio plus Streamable HTTP transports.
- `manage`: opens the interactive terminal index manager for starting index jobs and viewing indexed repositories.

### Prerequisites
???

### 1. Build the Binary

The project compiles as a single binary. You do not need separate builds for the `serve` and `manage` subcommands, and you do not need separate builds for `stdio` and `streamable-http`.

```sh
cargo build --release
```

Binary locations after a successful build:

- Linux/macOS: `./target/release/index-oxide-mcp`
- Windows: `.\target\release\index-oxide-mcp.exe`

### 2. Start the Qdrant Database
Download Qdrant directly from the official GitHub releases page, extract the archive, and run the `qdrant` executable. The `/latest/download/` links below track the current stable release:

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

Index Oxide MCP requires a `GEMINI_API_KEY` to function. You have three ways to provide it (in order of precedence):

1.  **CLI Argument**: Pass it directly when running the binary.
    ```powershell
    .\index-oxide-mcp.exe --api-key "your_api_key_here" serve
    ```
2.  **Environment Variable**: Set it in your shell environment.
    ```powershell
    $env:GEMINI_API_KEY="your_api_key_here"
    ```
3.  **.env File**: Create a file named `.env` in the root directory (where you run the binary).
    ```env
    GEMINI_API_KEY=your_api_key_here
    QDRANT_URL=http://localhost:6334
    ```

Other optional environment variables:
- `QDRANT_URL`: URL of the Qdrant gRPC endpoint (default: `http://localhost:6334`).
- `INXE_EMBEDDING_MODEL`: The Gemini embedding model to use (default: `gemini-embedding-2`).
- `INXE_SERVER_PORT`: Port for Streamable HTTP transport (default: `8754`).

### 4. Choose a Transport Mode

Use the `serve` subcommand when running the MCP server. `serve` supports both transport modes at runtime:

- `stdio`: Best for local MCP clients that launch the server themselves.
- `streamable-http`: Best when you want to run the server as a background HTTP service.

If you run `serve` without `--transport`, the default is `stdio`.

#### Mode A: `stdio` Transport

Use this for local clients such as Claude Desktop, Cursor, or other agentic CLIs that spawn the MCP server as a child process over stdin/stdout.

1. Build the project once with `cargo build --release`, or use a prebuilt binary.
2. Point your client at the compiled binary.
3. Configure the client to pass `serve` as the first argument. `stdio` is the default transport inside `serve`, so no transport argument is required.

*Configuration details for stdio clients can be found in the **Supported MCP Clients** section below.*

Path examples:

- Windows: `D:\\projects\\index-oxide-mcp\\target\\release\\index-oxide-mcp.exe`
- Linux/macOS: `/absolute/path/to/index-oxide-mcp/target/release/index-oxide-mcp`

Notes:

- Keep logs on `stderr`; MCP traffic uses `stdin`/`stdout` in this mode.
- `stdio` is the recommended default for local desktop agents.
- The direct terminal equivalent is `./target/release/index-oxide-mcp serve` on Linux/macOS or `.\target\release\index-oxide-mcp.exe serve` on Windows.

#### Mode B: `streamable-http` Transport

Use this when you want to run Index Oxide MCP as a standalone HTTP service and connect to it over the network or from a client that prefers URL-based MCP servers.

Start the server from your terminal.

```powershell
$env:GEMINI_API_KEY="your_api_key_here"; $env:QDRANT_URL="http://localhost:6334"; .\index-oxide-mcp.exe serve --transport streamable-http
```

Default Streamable HTTP endpoints:

- MCP endpoint: `http://localhost:8754/mcp`
- Health endpoint: `http://localhost:8754/health`

*Configuration details for Streamable HTTP clients can be found in the **Supported MCP Clients** section below.*

### Open CLI: `manage` Interactive TUI

Use `manage` when you want a slash-command terminal UI for index operations instead of connecting through an MCP client. The TUI uses the same `GEMINI_API_KEY`, `QDRANT_URL`, and embedding configuration as the MCP server.

Windows PowerShell:

```powershell
.\index-oxide-mcp.exe --api-key "your_api_key_here" manage 
```

In the TUI, type `/help` to show the available commands. Entering a repository path without a slash still starts an index job for compatibility.

Core commands:

- `/index <path>`: start indexing a repository path. Quote paths that contain spaces.
- `/refresh`: reload indexed repositories from Qdrant.
- `/delete` or `/delete selected`: request deletion of the selected collection.
- `/delete <collection>`: request deletion of a named collection.
- `/confirm`: confirm a pending deletion.
- `/cancel`: cancel a pending deletion or clear command state.
- `/model`: toggle between configured Gemini and Ollama embedders.
- `/model gemini` or `/model ollama`: switch to a specific embedder.
- `/clear`: clear the log panel.
- `/quit`: exit the TUI.

Keyboard shortcuts:

- `Up` / `Down`: select an indexed repository.
- `Delete`: request deletion of the selected repository when the command input is empty.
- `Tab`: toggle the active embedder when the command input is empty.
- `Ctrl+Q`: quit.
- `Esc`: clear input, cancel a pending deletion, or clear repository selection.

## Testing and Debugging with MCP Inspector

Use the official MCP Inspector when you want an interactive test UI for connection checks, capability negotiation, tool listing, request payload testing, and protocol-level debugging. It runs through `npx` without adding a project dependency.

Official reference: <https://modelcontextprotocol.io/docs/tools/inspector>

Windows PowerShell:

```powershell
npx -y @modelcontextprotocol/inspector
```

## Supported MCP Clients

Below are the minimum configuration schemas for popular agentic clients. Replace `/absolute/path/to/index-oxide-mcp` with the actual path to your compiled binary, and insert your real `GEMINI_API_KEY`.

Use one of two integration styles:

- `stdio`: the client starts the Index Oxide MCP binary with the `serve` argument and must receive `GEMINI_API_KEY` in its local server environment.
- `streamable-http` service mode: start Index Oxide MCP yourself with `serve --transport streamable-http`, then point the client at `http://localhost:8754/mcp`.

The HTTP service mode exposes RMCP Streamable HTTP at `/mcp`. If a client offers multiple HTTP-like transport choices, choose `http`, `remote`, or `Streamable HTTP` for `http://localhost:8754/mcp`.

### Gemini CLI

Streamable HTTP service config for `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "index-oxide": {
      "httpUrl": "http://localhost:8754/mcp"
    }
  }
}
```

### Kilo Code

Streamable HTTP service config matching the `mcp-remote` bridge shape commonly written by Kilo Code:

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

## Development

See [CONTRIBUTING.md](./CONTRIBUTING.md) for how to build, test, and contribute to the repository.
