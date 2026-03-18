//! Ailloy — Vendor-flexible AI integration for Rust tools.
//!
//! Ailloy is built for developers shipping tools to other users when those users may
//! have access to different AI vendors or environments. Integrate once in Rust, then
//! let each user configure the AI path they already have.
//!
//! Supported options include OpenAI, Anthropic, Azure OpenAI, Microsoft Foundry,
//! Google Vertex AI, Ollama, and local CLI agents (Claude, Codex, Copilot).
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
#[cfg(feature = "config-tui")]
pub mod azure_discover;
pub mod blocking;
pub mod client;
pub mod config;
#[cfg(feature = "config-tui")]
pub mod config_tui;
pub mod conversation;
pub mod discover;
pub mod error;
pub mod foundry;
pub mod local_agent;
pub mod ollama;
pub mod openai;
pub mod terminal;
pub mod types;
pub mod vertex;

// Re-export commonly used types at the crate root.
pub use client::{Client, Provider};
pub use config::{AiNode, Auth, Capability};
pub use conversation::{ChatHistory, Conversation, InMemoryHistory};
pub use error::ClientError;
pub use types::{
    ChatOptions, ChatResponse, ChatStream, ImageFormat, ImageOptions, ImageResponse, Message, Role,
    StreamEvent, Task, Usage,
};
