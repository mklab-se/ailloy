//! Ailloy — An AI abstraction layer for Rust.
//!
//! This crate provides a unified interface for interacting with multiple AI providers
//! including OpenAI, Anthropic, Azure OpenAI, Google Vertex AI, Ollama, and local CLI
//! agents (Claude, Codex, Copilot).
//!
//! # Quick start
//!
//! ```no_run
//! use ailloy::{Client, Message};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let client = Client::from_config()?;
//! let response = client.chat(&[Message::user("Hello!")]).await?;
//! println!("{}", response.content);
//! # Ok(())
//! # }
//! ```

pub mod anthropic;
pub mod azure;
pub mod blocking;
pub mod client;
pub mod config;
pub mod conversation;
pub mod error;
pub mod local_agent;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod types;
pub mod vertex;

// Re-export commonly used types at the crate root.
pub use client::{Client, Provider};
pub use conversation::{ChatHistory, Conversation, InMemoryHistory};
pub use error::ClientError;
pub use types::{
    ChatOptions, ChatResponse, ChatStream, EmbeddingResponse, ImageFormat, ImageOptions,
    ImageResponse, Message, Role, StreamEvent, Task, Usage,
};
