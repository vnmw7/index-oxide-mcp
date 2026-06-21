#![allow(dead_code)]
/*
 * System: Index Oxide MCP
 * File URL: index-oxide-mcp/src/main.rs
 * Purpose: Entry point - initializes tracing, loads config, and routes to MCP server or TUI manager
 */

mod cli;
mod clients;
mod config;
mod errors;
mod jobs;
mod manage;
mod mcp_server;
mod models;
mod pipeline;
mod search;

use crate::clients::GeminiClient;
use crate::clients::InxeQdrantClient;
use crate::clients::embedder::EmbedderClient;
use crate::config::{ActiveEmbedder, InxeConfig};
use crate::jobs::registry::JobRegistry;
use crate::mcp_server::InxeServer;
use axum::Router;
use clap::Parser;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::{
    StreamableHttpService, session::local::LocalSessionManager,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if it exists
    dotenvy::dotenv().ok();

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

    info!("index-oxide-mcp starting");

    let args = cli::CliArgs::parse();

    // If API key is provided via CLI, set it in the environment
    if let Some(key) = args.api_key {
        unsafe {
            std::env::set_var("GEMINI_API_KEY", key);
        }
    }

    // Load configuration from environment
    let config = InxeConfig::from_env()?;
    info!(
        model = %config.active_model_name(),
        dimensions = config.embedding.dimensions,
        qdrant_url = %config.qdrant.url,
        "Configuration loaded"
    );

    // Initialize Embedding client
    let embedder = Arc::new(RwLock::new(match config.active_embedder {
        ActiveEmbedder::Gemini => EmbedderClient::Gemini(GeminiClient::new(
            config.gemini.clone(),
            config.embedding.dimensions,
        )),
        ActiveEmbedder::Ollama => {
            EmbedderClient::Ollama(crate::clients::OllamaClient::new(config.ollama.clone()))
        }
    }));

    // Initialize Qdrant client
    let qdrant = Arc::new(InxeQdrantClient::new(
        &config.qdrant,
        config.embedding.dimensions,
    )?);

    // Initialize job registry
    let jobs = Arc::new(JobRegistry::new());

    // Wrap core components in Arc for sharing
    let config_arc = Arc::new(config);
    let embedder_arc = embedder;
    let qdrant_arc = qdrant;
    let jobs_arc = jobs;

    match args.command {
        cli::Commands::Serve { transport } => match transport {
            cli::TransportMode::Stdio => {
                // Create MCP server
                let server = InxeServer::new(
                    Arc::clone(&config_arc),
                    Arc::clone(&embedder_arc),
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
                        let embedder = Arc::clone(&embedder_arc);
                        let qdrant = Arc::clone(&qdrant_arc);
                        let jobs = Arc::clone(&jobs_arc);
                        move || {
                            Ok(InxeServer::new(
                                Arc::clone(&config_arc),
                                Arc::clone(&embedder),
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
        },
        cli::Commands::Manage => {
            info!("Starting TUI manager");
            manage::run_tui(config_arc, embedder_arc, qdrant_arc, jobs_arc).await?;
        }
    }

    info!("index-oxide-mcp shutting down");
    Ok(())
}
