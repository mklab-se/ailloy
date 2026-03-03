//! Blocking (synchronous) client.
//!
//! Provides a sync wrapper around the async client for applications that don't
//! use an async runtime. Uses its own internal tokio runtime, similar to
//! `reqwest::blocking`.
//!
//! **Note:** Cannot be called from within an existing async runtime.
//!
//! # Example
//!
//! ```no_run
//! use ailloy::blocking::Client;
//! use ailloy::Message;
//!
//! let client = Client::from_config().unwrap();
//! let response = client.chat(&[Message::user("Hello!")]).unwrap();
//! println!("{}", response.content);
//! ```

use anyhow::Result;

use crate::types::{
    ChatOptions, ChatResponse, EmbeddingResponse, ImageOptions, ImageResponse, Message,
    StreamEvent, Task,
};

/// A synchronous client wrapping the async [`crate::Client`].
pub struct Client {
    inner: crate::Client,
    runtime: tokio::runtime::Runtime,
}

impl Client {
    /// Create a client from the default config (uses `defaults.chat` provider).
    pub fn from_config() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let inner = crate::Client::from_config()?;
        Ok(Self { inner, runtime })
    }

    /// Create a client using a specific named provider from config.
    pub fn with_provider(provider_name: &str) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let inner = crate::Client::with_provider(provider_name)?;
        Ok(Self { inner, runtime })
    }

    /// Create a client for a specific task type.
    pub fn for_task(task: Task) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let inner = crate::Client::for_task(task)?;
        Ok(Self { inner, runtime })
    }

    /// Create a client wrapping an existing async client.
    pub fn from_async(inner: crate::Client) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(Self { inner, runtime })
    }

    /// Send a simple chat request.
    pub fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        self.runtime.block_on(self.inner.chat(messages))
    }

    /// Send a chat request with options.
    pub fn chat_with(&self, messages: &[Message], options: &ChatOptions) -> Result<ChatResponse> {
        self.runtime
            .block_on(self.inner.chat_with(messages, options))
    }

    /// Send a streaming chat request.
    ///
    /// Returns an iterator over stream events.
    pub fn chat_stream(&self, messages: &[Message]) -> Result<BlockingChatStream<'_>> {
        let stream = self.runtime.block_on(self.inner.chat_stream(messages))?;
        Ok(BlockingChatStream {
            stream,
            runtime: &self.runtime,
        })
    }

    /// Generate an image from a text prompt.
    pub fn generate_image(&self, prompt: &str) -> Result<ImageResponse> {
        self.runtime.block_on(self.inner.generate_image(prompt))
    }

    /// Generate an image with options.
    pub fn generate_image_with(
        &self,
        prompt: &str,
        options: &ImageOptions,
    ) -> Result<ImageResponse> {
        self.runtime
            .block_on(self.inner.generate_image_with(prompt, options))
    }

    /// Generate an embedding vector from text.
    pub fn embed(&self, input: &str) -> Result<EmbeddingResponse> {
        self.runtime.block_on(self.inner.embed(input))
    }

    /// Get the provider name.
    pub fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }
}

/// An iterator-based stream for blocking mode.
pub struct BlockingChatStream<'a> {
    stream: crate::types::ChatStream,
    runtime: &'a tokio::runtime::Runtime,
}

impl Iterator for BlockingChatStream<'_> {
    type Item = Result<StreamEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        use futures_util::StreamExt;
        self.runtime.block_on(self.stream.next())
    }
}
