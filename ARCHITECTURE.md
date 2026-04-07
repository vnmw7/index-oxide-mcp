# Architecture: Index Oxide MCP

## Overview

Index Oxide MCP is a high-performance code indexing and semantic search server implementing the Model Context Protocol (MCP). It is designed to run natively as a standalone binary alongside a containerized Qdrant database, providing a scalable and portable solution for code intelligence.

## System Architecture

The system consists of two primary components:

1.  **Oxi-MCP Server (Rust)**: The core logic engine that runs natively. It handles MCP requests, manages indexing pipelines, directly accesses the local filesystem, and coordinates with external AI services.
2.  **Oxi-Qdrant (Vector DB)**: A dedicated high-performance vector database used to store and retrieve code embeddings, orchestrated via Docker.

```mermaid
graph TD
    Client[MCP Client / IDE] <-->|SSE / HTTP| Server[Oxi-MCP Server]
    Server <-->|gRPC| Qdrant[Qdrant Vector DB (Docker)]
    Server -->|HTTPS| Gemini[Google Gemini API]
    Server <-->|Native FS| SourceCode[Local Source Code]
```

## Core Components

### 1. Dual-Transport Layer
The server supports two primary transport modes for communication with MCP clients:
- **Stdio Transport**: Standard stdin/stdout communication for local integrations, which is typically the primary way IDEs like Cursor or Claude Desktop start and interact with the MCP server natively.
- **SSE (Server-Sent Events) Transport**: A robust HTTP-based transport layer using `axum`. This is useful for standalone shared indexing servers or setups where the client requires connecting over an HTTP network boundary.

### 2. Vector Database (Qdrant)
- **Communication**: The MCP server communicates with Qdrant via high-speed **gRPC** (Port 6334).
- **Persistence**: Data is persisted in a Docker volume (`qdrant_data`), ensuring index durability across restarts.
- **Organization**: Each repository is indexed into its own collection (`oxi_{sanitized_name}`), allowing for isolated management and clearing of indices.

### 3. Asynchronous Processing Pipeline
Indexing is performed via a non-blocking background pipeline to ensure MCP responsiveness:
1.  **Discovery**: Recursively crawls the workspace natively, respecting `.gitignore` and glob patterns.
2.  **Parsing**: Utilizes `tree-sitter` for language-aware code chunking (preserving semantic boundaries like functions and classes).
3.  **Embedding**: Batches code chunks and generates high-dimensional vectors via the **Google Gemini API**.
4.  **Indexing**: Streams embeddings and metadata into Qdrant.

### 4. Job Management
The `JobRegistry` provides real-time tracking of indexing operations. Each job is assigned a UUID, and clients can poll the status or cancel long-running indexing tasks without affecting the main server loop.

## Deployment Configuration

The environment is simplified to a native executable alongside a minimal `docker-compose.yml`, which handles:
- **Database Orchestration**: Automatic startup and configuration of the Qdrant database.
- **Persistence**: Management of the Docker volumes for the vector database.