//! Core message types for AI interactions.

use std::pin::Pin;

use futures_util::Stream;
use serde::{Deserialize, Serialize};

/// The role of a message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Options for controlling chat generation.
#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

impl ChatOptions {
    pub fn builder() -> ChatOptionsBuilder {
        ChatOptionsBuilder::default()
    }
}

/// Builder for [`ChatOptions`].
#[derive(Debug, Default)]
pub struct ChatOptionsBuilder {
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

impl ChatOptionsBuilder {
    pub fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn build(self) -> ChatOptions {
        ChatOptions {
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        }
    }
}

/// A response from an AI provider.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<Usage>,
}

/// Token usage information.
#[derive(Debug, Clone)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// An event from a streaming chat response.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A text delta (partial token).
    Delta(String),
    /// The stream is complete, with the final assembled response.
    Done(ChatResponse),
}

/// A stream of chat events.
pub type ChatStream = Pin<Box<dyn Stream<Item = anyhow::Result<StreamEvent>> + Send>>;

/// Response from an image generation request.
#[derive(Debug, Clone)]
pub struct ImageResponse {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub revised_prompt: Option<String>,
}

/// Supported image output formats.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

/// Options for image generation.
#[derive(Debug, Clone, Default)]
pub struct ImageOptions {
    pub size: Option<(u32, u32)>,
    pub quality: Option<String>,
    pub style: Option<String>,
}

impl ImageOptions {
    pub fn builder() -> ImageOptionsBuilder {
        ImageOptionsBuilder::default()
    }
}

/// Builder for [`ImageOptions`].
#[derive(Debug, Default)]
pub struct ImageOptionsBuilder {
    size: Option<(u32, u32)>,
    quality: Option<String>,
    style: Option<String>,
}

impl ImageOptionsBuilder {
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width, height));
        self
    }

    pub fn quality(mut self, quality: impl Into<String>) -> Self {
        self.quality = Some(quality.into());
        self
    }

    pub fn style(mut self, style: impl Into<String>) -> Self {
        self.style = Some(style.into());
        self
    }

    pub fn build(self) -> ImageOptions {
        ImageOptions {
            size: self.size,
            quality: self.quality,
            style: self.style,
        }
    }
}

/// Response from an embedding request.
#[derive(Debug, Clone)]
pub struct EmbeddingResponse {
    pub vector: Vec<f32>,
    pub model: String,
    pub usage: Option<Usage>,
}

/// Task types for provider routing.
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    Chat,
    ImageGeneration,
    Embedding,
    Transcription,
}

impl Task {
    /// Returns the config key for this task (used in `defaults` map).
    pub fn config_key(&self) -> &str {
        match self {
            Self::Chat => "chat",
            Self::ImageGeneration => "image",
            Self::Embedding => "embedding",
            Self::Transcription => "transcription",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_constructors() {
        let msg = Message::user("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "hello");

        let msg = Message::system("you are helpful");
        assert_eq!(msg.role, Role::System);

        let msg = Message::assistant("response");
        assert_eq!(msg.role, Role::Assistant);
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::user("hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"hello\""));
    }

    #[test]
    fn test_role_roundtrip() {
        let msg = Message::system("test");
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, Role::System);
        assert_eq!(parsed.content, "test");
    }

    #[test]
    fn test_chat_options_builder() {
        let opts = ChatOptions::builder()
            .temperature(0.7)
            .max_tokens(100)
            .build();
        assert_eq!(opts.temperature, Some(0.7));
        assert_eq!(opts.max_tokens, Some(100));
    }

    #[test]
    fn test_chat_options_default() {
        let opts = ChatOptions::default();
        assert!(opts.temperature.is_none());
        assert!(opts.max_tokens.is_none());
    }

    #[test]
    fn test_image_options_builder() {
        let opts = ImageOptions::builder()
            .size(1024, 1024)
            .quality("hd")
            .style("natural")
            .build();
        assert_eq!(opts.size, Some((1024, 1024)));
        assert_eq!(opts.quality.unwrap(), "hd");
        assert_eq!(opts.style.unwrap(), "natural");
    }

    #[test]
    fn test_task_config_key() {
        assert_eq!(Task::Chat.config_key(), "chat");
        assert_eq!(Task::ImageGeneration.config_key(), "image");
        assert_eq!(Task::Embedding.config_key(), "embedding");
        assert_eq!(Task::Transcription.config_key(), "transcription");
    }
}
