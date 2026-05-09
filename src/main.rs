#![allow(dead_code)]
/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/main.rs
 * Purpose: Entry point - initializes tracing, loads config, starts MCP server on stdio transport
 */

mod cli;
mod config;
mod errors;
mod gemini;
mod jobs;
mod mcp_server;
mod models;
mod pipeline;
mod qdrant;
mod search;
mod util;

use crate::config::OxiConfig;
use crate::gemini::client::GeminiClient;
use crate::jobs::registry::JobRegistry;
use crate::mcp_server::OxiServer;
use crate::qdrant::client::OxiQdrantClient;
use axum::Router;
use clap::Parser;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};
use rmcp::ServiceExt;
use std::sync::Arc;
use tracing::info;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    // Files are saved in the same directory as the executable
    let log_dir = std::env::current_exe()?
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("index-oxide-mcp")
        .filename_suffix("log")
        .build(log_dir)
        .expect("Failed to initialize rolling file appender");

    // Use non-blocking writer for performance
    // The _guard must remain in scope for the duration of the program to ensure logs are flushed
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // Separate layers for stderr (human-readable) and file (JSON)
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_filter(filter.clone());

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .json()
        .with_ansi(false)
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    info!("oxidized-index-mcp starting");

    // Load configuration from environment
    let config = OxiConfig::from_env()?;
    info!(
        model = %config.gemini.model,
        dimensions = config.embedding.dimensions,
        qdrant_url = %config.qdrant.url,
        "Configuration loaded"
    );

    // Initialize Gemini client
    let gemini = Arc::new(GeminiClient::new(
        config.gemini.clone(),
        config.embedding.dimensions,
    ));

    // Initialize Qdrant client
    let qdrant = Arc::new(OxiQdrantClient::new(
        &config.qdrant,
        config.embedding.dimensions,
    )?);

    // Initialize job registry
    let jobs = Arc::new(JobRegistry::new());

    // Wrap core components in Arc for sharing
    let config_arc = Arc::new(config);
    let gemini_arc = gemini;
    let qdrant_arc = qdrant;
    let jobs_arc = jobs;

    let args = cli::CliArgs::parse();

    match args.transport {
        cli::TransportMode::Stdio => {
            // Create MCP server
            let server = OxiServer::new(
                Arc::clone(&config_arc),
                Arc::clone(&gemini_arc),
                Arc::clone(&qdrant_arc),
                Arc::clone(&jobs_arc),
            );

            info!("Starting MCP server on stdio transport");

            // Start MCP server on stdio
            let transport = (tokio::io::stdin(), tokio::io::stdout());
            let running_server = server
                .serve(transport)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to start MCP server: {}", e))?;

            // Wait for the server to finish (runs until the transport closes)
            running_server
                .waiting()
                .await
                .map_err(|e| anyhow::anyhow!("MCP server error: {}", e))?;
        }
        cli::TransportMode::StreamableHttp => {
            let mcp_service = StreamableHttpService::new(
                {
                    let config_arc = Arc::clone(&config_arc);
                    let gemini = Arc::clone(&gemini_arc);
                    let qdrant = Arc::clone(&qdrant_arc);
                    let jobs = Arc::clone(&jobs_arc);
                    move || {
                        Ok(OxiServer::new(
                            Arc::clone(&config_arc),
                            Arc::clone(&gemini),
                            Arc::clone(&qdrant),
                            Arc::clone(&jobs),
                        ))
                    }
                },
                LocalSessionManager::default().into(),
                Default::default(),
            );

            let app = Router::new()
                .nest_service("/mcp", mcp_service)
                .route("/health", axum::routing::get(|| async { "ok" }));

            let addr = format!("{}:{}", config_arc.server.host, config_arc.server.port);
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            info!(address = %addr, "MCP Streamable HTTP server listening");
            axum::serve(listener, app).await?;
        }
    }

    info!("oxidized-index-mcp shutting down");
    Ok(())
}
