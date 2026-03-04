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

impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageFormat::Png => write!(f, "PNG"),
            ImageFormat::Jpeg => write!(f, "JPEG"),
            ImageFormat::Webp => write!(f, "WebP"),
        }
    }
}

/// Read image dimensions from raw PNG/JPEG/WebP data.
///
/// Parses the header bytes to extract width and height without
/// requiring a full image decoding library.
pub fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    // PNG: 8-byte signature, then IHDR chunk (4 length + 4 "IHDR" + 4 width + 4 height)
    if data.len() >= 24 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return Some((width, height));
    }

    // JPEG: starts with 0xFF 0xD8, scan for SOF0 (0xFF 0xC0) or SOF2 (0xFF 0xC2)
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        let mut i = 2;
        while i + 1 < data.len() {
            if data[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = data[i + 1];
            // SOF0 or SOF2 (baseline/progressive)
            if (marker == 0xC0 || marker == 0xC2) && i + 9 <= data.len() {
                let height = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                let width = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                return Some((width, height));
            }
            // Skip to next marker using segment length
            if i + 3 < data.len() {
                let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
                i += 2 + len;
            } else {
                break;
            }
        }
    }

    // WebP: "RIFF" + size + "WEBP", then VP8 chunk
    if data.len() >= 30
        && &data[..4] == b"RIFF"
        && &data[8..12] == b"WEBP"
        && &data[12..16] == b"VP8 "
    {
        // Simple lossy VP8: dimensions at bytes 26-29
        let width = u16::from_le_bytes([data[26], data[27]]) as u32 & 0x3FFF;
        let height = u16::from_le_bytes([data[28], data[29]]) as u32 & 0x3FFF;
        return Some((width, height));
    }

    None
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

    /// Convert this task to a [`Capability`](crate::config::Capability), if applicable.
    pub fn to_capability(&self) -> Option<crate::config::Capability> {
        match self {
            Self::Chat => Some(crate::config::Capability::Chat),
            Self::ImageGeneration => Some(crate::config::Capability::Image),
            Self::Embedding => Some(crate::config::Capability::Embedding),
            Self::Transcription => None,
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

    #[test]
    fn test_image_format_display() {
        assert_eq!(format!("{}", ImageFormat::Png), "PNG");
        assert_eq!(format!("{}", ImageFormat::Jpeg), "JPEG");
        assert_eq!(format!("{}", ImageFormat::Webp), "WebP");
    }

    #[test]
    fn test_image_dimensions_png() {
        // Minimal PNG header: signature + IHDR chunk
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        // IHDR chunk: length (13) + "IHDR" + width (1536) + height (1024)
        png.extend_from_slice(&[0, 0, 0, 13]); // chunk length
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1536u32.to_be_bytes()); // width
        png.extend_from_slice(&1024u32.to_be_bytes()); // height
        assert_eq!(image_dimensions(&png), Some((1536, 1024)));
    }

    #[test]
    fn test_image_dimensions_jpeg() {
        // Minimal JPEG with SOI + SOF0 marker
        let mut jpeg = vec![0xFF, 0xD8]; // SOI
        // APP0 marker (to skip past)
        jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]); // marker + length=16
        jpeg.extend_from_slice(&[0; 14]); // padding for APP0 body
        // SOF0 marker
        jpeg.extend_from_slice(&[0xFF, 0xC0]);
        jpeg.extend_from_slice(&[0x00, 0x11]); // length=17
        jpeg.push(0x08); // precision
        jpeg.extend_from_slice(&768u16.to_be_bytes()); // height
        jpeg.extend_from_slice(&1024u16.to_be_bytes()); // width
        assert_eq!(image_dimensions(&jpeg), Some((1024, 768)));
    }

    #[test]
    fn test_image_dimensions_too_short() {
        assert_eq!(image_dimensions(&[]), None);
        assert_eq!(image_dimensions(&[0x89, b'P']), None);
    }
}
