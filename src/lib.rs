//! Ailloy — An AI abstraction layer for Rust.
//!
//! This crate provides a unified interface for interacting with multiple AI providers
//! including OpenAI, Ollama, and local CLI agents (Claude, Codex, Copilot).
//!
//! # Quick start
//!
//! ```no_run
//! use ailloy::config::Config;
//! use ailloy::provider::create_provider;
//! use ailloy::types::Message;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = Config::load()?;
//! let provider = create_provider(&config)?;
//! let response = provider.chat(&[Message::user("Hello!")]).await?;
//! println!("{}", response.content);
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod error;
pub mod local_agent;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod types;
