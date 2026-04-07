# Contributing to Index Oxide

First off, thank you for considering contributing to Index Oxide MCP!

## Process

1. **Fork the repository** to your own GitHub account.
2. **Create a branch** for your feature or fix.
3. **Commit your changes**. Ensure your code is well-tested and adheres to the project's formatting by running:
    ```sh
    cargo test
    cargo fmt
    cargo clippy
    ```
4. **Open a Pull Request**. Ensure you provide a clear description of your changes.

## Development Setup

To get started with local development:

1. Ensure you have [Rust](https://rustup.rs/) installed.
2. Ensure you have Docker to run the backend dependencies (like Qdrant) if required:
    ```sh
    docker-compose up -d
    ```
3. Run `cargo build` to build the repository.

### Guidelines
* Check the `/src` directory for functional domains. Do not bloat `main.rs` over simple initialization.
* Maintain minimum test coverage for newly added components.
