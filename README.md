# Index Oxide MCP

Codebase indexing using Rust, Qdrant, and Gemini.

Index Oxide MCP is a high-throughput MCP (Model Context Protocol) codebase indexer built for agentic AI workflows. It parses source code using Tree-sitter, generates context embeddings via Gemini, and indexes them into an integrated Qdrant vector database for fast, semantic retrieval.

## Features
- **High-throughput Indexing:** Concurrently walks and indexes large codebases.
- **Tree-sitter Parsing:** Precise syntax tree extraction for multiple languages (Rust, Python, TS, Go).
- **Vector Search:** Semantically retrieves code blocks using Qdrant.
- **MCP Server:** Dual-mode support (SSE Web Server and stdio) via the native `rmcp` SDK.

## Setup & Deployment

Index Oxide MCP is designed to run natively as a standalone binary alongside a containerized Qdrant database. You can choose to download a pre-built binary or compile it from source. The server supports dual-transport modes: Stdio (standard I/O) and SSE (HTTP).

### Prerequisites
- **Docker**: Required to run the Qdrant vector database.
- **Index Oxide MCP Binary**: Either download the pre-built binary for your OS from the [GitHub Releases](https://github.com/username/repo_name/releases) page OR install [Rust](https://rustup.rs/) (v1.94+) to compile from source.

### 1. Start the Database
The easiest way to start the Qdrant database is using Docker Compose. Run the following command in the project root:
```sh
docker-compose up -d
```

### 2. Connect your MCP Client

You can run the MCP server in one of two modes depending on your client's requirements:

#### Mode A: Stdio Transport (Local Executable)
In this mode, the AI agent (like Cursor or Claude Desktop) directly launches the server as a child process and communicates via standard input/output.

1. Ensure the downloaded binary (e.g., `index-oxide-mcp-windows.exe`) is executable and accessible, or build it locally using `cargo build --release`.
2. Add the following JSON configuration block to your MCP client's configuration file:

```json
{
  "mcpServers": {
    "index-oxide": {
      "command": "/path/to/index-oxide-mcp",
      "args": ["--transport", "stdio"],
      "env": {
        "GEMINI_API_KEY": "your_gemini_api_key_here",
        "QDRANT_URL": "http://localhost:6334"
      }
    }
  }
}
```

#### Mode B: SSE HTTP Transport (Background Service)
In this mode, you run the MCP server in the background, and the AI agent connects to it via an HTTP endpoint. This is useful for shared indexing servers or specific client requirements.

1. Run the server natively in your terminal with the required environment variables:
```sh
# Set environment variables (Linux/macOS)
export GEMINI_API_KEY="your_api_key_here"
export QDRANT_URL="http://localhost:6334"

# Run the server
./index-oxide-mcp --transport sse
```

2. Add the following JSON configuration block to your MCP client:

```json
{
  "mcpServers": {
    "index-oxide": {
      "url": "http://localhost:8754/mcp"
    }
  }
}
```

## Development

See [CONTRIBUTING.md](./CONTRIBUTING.md) for how to build, test, and contribute to the repository.
