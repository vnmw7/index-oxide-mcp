/*
 * System: Index Oxide MCP
 * File URL: inxe-index-mcp/src/cli.rs
 * Purpose: CLI argument definitions for subcommand selection and configuration
 */

use clap::{Parser, Subcommand};

/// Inxe Index MCP: High-throughput codebase indexer for agentic AI workflows.
#[derive(Parser, Debug)]
#[command(name = "inxe-index-mcp", version, about)]
pub struct CliArgs {
    /// API Key for Gemini authentication. If provided, overrides the environment variable.
    #[arg(long, env = "GEMINI_API_KEY", global = true)]
    pub api_key: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the MCP server.
    Serve {
        /// Transport mode: "stdio" for local MCP clients, "streamable-http" for HTTP deployment.
        #[arg(long, default_value = "stdio")]
        transport: TransportMode,
    },
    /// Open the interactive TUI to manage indexes and projects.
    Manage,
}

#[derive(Debug, Clone, clap::ValueEnum, PartialEq)]
pub enum TransportMode {
    /// Standard I/O transport (stdin/stdout) — for local MCP client spawning
    Stdio,
    /// Streamable HTTP transport — for network/Docker deployment
    StreamableHttp,
}
