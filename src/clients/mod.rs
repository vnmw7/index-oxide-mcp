pub mod embedder;
mod gemini;
mod ollama;
mod qdrant;

pub use gemini::client::{BatchEmbedResult, EmbedInput, GeminiClient};
pub use ollama::client::OllamaClient;
pub use qdrant::client::InxeQdrantClient;
