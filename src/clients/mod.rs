pub mod embedder;
mod gemini;
mod ollama;
mod qdrant;

pub use gemini::client::{GeminiClient, EmbedInput, BatchEmbedResult};
pub use ollama::client::OllamaClient;
pub use qdrant::client::InxeQdrantClient;
