/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/cli.rs
 * Purpose: CLI argument definitions for transport selection and configuration
 */

use clap::Parser;

/// Index Oxide MCP: High-throughput codebase indexer for agentic AI workflows.
#[derive(Parser, Debug)]
#[command(name = "index-oxide-mcp", version, about)]
pub struct CliArgs {
    /// Transport mode: "stdio" for local MCP clients, "sse" for HTTP/SSE deployment.
    #[arg(long, default_value = "stdio")]
    pub transport: TransportMode,
}

#[derive(Debug, Clone, clap::ValueEnum, PartialEq)]
pub enum TransportMode {
    /// Standard I/O transport (stdin/stdout) — for local MCP client spawning
    Stdio,
    /// Streamable HTTP transport (SSE) — for network/Docker deployment
    Sse,
}
