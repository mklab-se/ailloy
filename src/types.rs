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

/// Base64 (STANDARD) ⇄ `Vec<u8>` serde helper for binary attachment data.
///
/// Used via `#[serde(with = "base64_bytes")]` on [`ContentPart`] byte fields so
/// that image/file bytes travel as a base64 string in JSON/YAML.
mod base64_bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(deserializer)?;
        STANDARD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// A single content part within a multi-part [`MessageContent`].
///
/// Serializes to a `type`-tagged object (`{"type":"text","text":"…"}`,
/// `{"type":"image","data":"<base64>","media_type":"image/png"}`, etc.).
/// Binary `data` fields are base64-encoded on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentPart {
    /// A run of plain text.
    Text { text: String },
    /// An inline image (png/jpeg/gif/webp) with its media type.
    Image {
        #[serde(with = "base64_bytes")]
        data: Vec<u8>,
        media_type: String,
    },
    /// An inline file attachment (pdf, text documents, …) with media type and
    /// original filename.
    File {
        #[serde(with = "base64_bytes")]
        data: Vec<u8>,
        media_type: String,
        filename: String,
    },
}

/// The content of a [`Message`]: either plain text or an ordered list of
/// [`ContentPart`]s (text interleaved with image/file attachments).
///
/// The representation is `#[serde(untagged)]`, so a text-only message
/// serializes to (and deserializes from) exactly the bare-string form used in
/// ailloy 1.x — `"content": "hello"`. Multi-part content serializes to a JSON
/// array of tagged part objects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text (wire-compatible with the 1.x `String` content field).
    Text(String),
    /// An ordered list of content parts (text + attachments).
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    /// The textual content: for `Text`, a clone of the string; for `Parts`,
    /// the text parts joined with `"\n"` (attachments contribute nothing).
    pub fn text(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Borrow the text directly, but only when this is plain `Text`; returns
    /// `None` for multi-part content.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            MessageContent::Text(s) => Some(s.as_str()),
            MessageContent::Parts(_) => None,
        }
    }

    /// True if this content carries any non-text part (image or file).
    pub fn has_attachments(&self) -> bool {
        match self {
            MessageContent::Text(_) => false,
            MessageContent::Parts(parts) => {
                parts.iter().any(|p| !matches!(p, ContentPart::Text { .. }))
            }
        }
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        MessageContent::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        MessageContent::Text(s.to_string())
    }
}

impl From<&String> for MessageContent {
    fn from(s: &String) -> Self {
        MessageContent::Text(s.clone())
    }
}

impl From<Vec<ContentPart>> for MessageContent {
    fn from(parts: Vec<ContentPart>) -> Self {
        MessageContent::Parts(parts)
    }
}

impl std::fmt::Display for MessageContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.text())
    }
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

impl Message {
    pub fn system(content: impl Into<MessageContent>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<MessageContent>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<MessageContent>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }

    /// Build a user message whose content is a text prompt followed by one
    /// attachment part per file, in argument order.
    ///
    /// Attachment media types are inferred from the file extension:
    /// - Images (`Image` parts): `png`→image/png, `jpg`/`jpeg`→image/jpeg,
    ///   `gif`→image/gif, `webp`→image/webp
    /// - `pdf` → `File` part with media type `application/pdf`
    /// - Text documents (`File` parts): `txt`→text/plain, `md`→text/markdown,
    ///   `csv`→text/csv, `json`→application/json, `yaml`/`yml`→application/yaml,
    ///   `xml`→application/xml, `html`→text/html
    ///
    /// Any other extension yields an actionable error listing supported types.
    /// A missing/unreadable file yields an actionable error naming the path.
    ///
    /// Note: file reads are synchronous ([`std::fs::read`]); call from a
    /// blocking context or a spawned blocking task if that matters.
    pub fn user_with_attachments(
        text: impl Into<String>,
        files: &[std::path::PathBuf],
    ) -> anyhow::Result<Message> {
        use anyhow::Context as _;

        // Which kind of part an extension maps to, resolved before reading so
        // that an unsupported extension fails deterministically even if the
        // file is absent.
        enum Kind {
            Image(&'static str),
            File(&'static str),
        }

        let mut parts = vec![ContentPart::Text { text: text.into() }];

        for path in files {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();

            let kind = match ext.as_str() {
                "png" => Kind::Image("image/png"),
                "jpg" | "jpeg" => Kind::Image("image/jpeg"),
                "gif" => Kind::Image("image/gif"),
                "webp" => Kind::Image("image/webp"),
                "pdf" => Kind::File("application/pdf"),
                "txt" => Kind::File("text/plain"),
                "md" => Kind::File("text/markdown"),
                "csv" => Kind::File("text/csv"),
                "json" => Kind::File("application/json"),
                "yaml" | "yml" => Kind::File("application/yaml"),
                "xml" => Kind::File("application/xml"),
                "html" => Kind::File("text/html"),
                other => anyhow::bail!(
                    "unsupported attachment type '{}' for '{}' — supported: images (png, jpg, jpeg, gif, webp), pdf, and text files (txt, md, csv, json, yaml, yml, xml, html)",
                    if other.is_empty() {
                        "(no extension)"
                    } else {
                        other
                    },
                    path.display()
                ),
            };

            let data = std::fs::read(path).with_context(|| {
                format!(
                    "Failed to read attachment '{}' — check the path exists and is readable.",
                    path.display()
                )
            })?;

            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("attachment")
                .to_string();

            let part = match kind {
                Kind::Image(media_type) => ContentPart::Image {
                    data,
                    media_type: media_type.to_string(),
                },
                Kind::File(media_type) => ContentPart::File {
                    data,
                    media_type: media_type.to_string(),
                    filename,
                },
            };
            parts.push(part);
        }

        Ok(Message {
            role: Role::User,
            content: MessageContent::Parts(parts),
        })
    }
}

/// Build the OpenAI Chat Completions `messages` array from our [`Message`]s.
///
/// A text-only message stays wire-compatible with ailloy 1.x — its `content`
/// is a bare JSON string, never an array. A message carrying attachment parts
/// becomes a content array of typed blocks:
/// - text → `{"type":"text","text":…}`
/// - image → `{"type":"image_url","image_url":{"url":"data:<media>;base64,<b64>"}}`
/// - file → `{"type":"file","file":{"filename":…,"file_data":"data:<media>;base64,<b64>"}}`
///
/// Shared by every OpenAI-compatible surface (OpenAI, Azure OpenAI, Foundry).
pub(crate) fn to_openai_wire(messages: &[Message]) -> serde_json::Value {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    fn role_str(role: &Role) -> &'static str {
        match role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    let arr: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| match &m.content {
            MessageContent::Text(s) => serde_json::json!({
                "role": role_str(&m.role),
                "content": s,
            }),
            MessageContent::Parts(parts) => {
                let blocks: Vec<serde_json::Value> = parts
                    .iter()
                    .map(|p| match p {
                        ContentPart::Text { text } => serde_json::json!({
                            "type": "text",
                            "text": text,
                        }),
                        ContentPart::Image { data, media_type } => serde_json::json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", media_type, STANDARD.encode(data)),
                            },
                        }),
                        ContentPart::File {
                            data,
                            media_type,
                            filename,
                        } => serde_json::json!({
                            "type": "file",
                            "file": {
                                "filename": filename,
                                "file_data": format!("data:{};base64,{}", media_type, STANDARD.encode(data)),
                            },
                        }),
                    })
                    .collect();
                serde_json::json!({
                    "role": role_str(&m.role),
                    "content": blocks,
                })
            }
        })
        .collect();
    serde_json::Value::Array(arr)
}

/// Options for controlling chat generation.
#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Constrain the response to JSON (providers that support it natively;
    /// see [`ResponseFormat`]).
    pub response_format: Option<ResponseFormat>,
}

/// Structured-output constraint for a chat request.
#[derive(Debug, Clone)]
pub enum ResponseFormat {
    /// The model must emit a single valid JSON object.
    JsonObject,
    /// The model must emit JSON matching the given JSON Schema.
    JsonSchema {
        /// Schema name (required by OpenAI-family providers).
        name: String,
        /// The JSON Schema document.
        schema: serde_json::Value,
        /// Enforce the schema strictly where the provider supports it.
        strict: bool,
    },
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
    response_format: Option<ResponseFormat>,
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

    /// Constrain the response to a single JSON object.
    pub fn json(mut self) -> Self {
        self.response_format = Some(ResponseFormat::JsonObject);
        self
    }

    /// Constrain the response to JSON matching `schema` (strict).
    pub fn json_schema(mut self, name: impl Into<String>, schema: serde_json::Value) -> Self {
        self.response_format = Some(ResponseFormat::JsonSchema {
            name: name.into(),
            schema,
            strict: true,
        });
        self
    }

    pub fn build(self) -> ChatOptions {
        ChatOptions {
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            response_format: self.response_format,
        }
    }
}

impl ResponseFormat {
    /// OpenAI-family `response_format` request value.
    pub(crate) fn to_openai_value(&self) -> serde_json::Value {
        match self {
            ResponseFormat::JsonObject => serde_json::json!({"type": "json_object"}),
            ResponseFormat::JsonSchema {
                name,
                schema,
                strict,
            } => {
                // OpenAI-family strict mode requires `additionalProperties: false`
                // on every object node. Auto-patch a clone so callers don't have
                // to hand-edit their schemas (the original `self` stays intact).
                let schema = if *strict {
                    let mut patched = schema.clone();
                    patch_strict_additional_properties(&mut patched);
                    patched
                } else {
                    schema.clone()
                };
                serde_json::json!({
                    "type": "json_schema",
                    "json_schema": {"name": name, "schema": schema, "strict": strict}
                })
            }
        }
    }

    /// Ollama `format` request value (native JSON / schema-constrained output).
    pub(crate) fn to_ollama_value(&self) -> serde_json::Value {
        match self {
            ResponseFormat::JsonObject => serde_json::json!("json"),
            ResponseFormat::JsonSchema { schema, .. } => schema.clone(),
        }
    }

    /// Prompt suffix for providers without a native structured-output knob
    /// (currently Anthropic, where prompted JSON is the documented pattern
    /// for simple cases).
    pub(crate) fn nudge_text(&self) -> String {
        match self {
            ResponseFormat::JsonObject => {
                "\n\nRespond with a single valid JSON object and nothing else — no prose, no code fences.".to_string()
            }
            ResponseFormat::JsonSchema { schema, .. } => format!(
                "\n\nRespond with a single valid JSON document matching this JSON Schema exactly — no prose, no code fences:\n{}",
                serde_json::to_string_pretty(schema).unwrap_or_default()
            ),
        }
    }
}

/// Recursively insert `"additionalProperties": false` on every object schema
/// node that lacks it, as required by OpenAI-family strict `json_schema` mode.
///
/// A node is treated as an object schema when its `"type"` is `"object"` or it
/// carries a `"properties"` key. An existing `additionalProperties` (whether
/// `true` or a schema value) is never overwritten. Recurses into `properties`
/// values, `items` (single schema or tuple array), `$defs`/`definitions`
/// values, and `anyOf`/`allOf`/`oneOf` arrays. Operates in place on a clone.
fn patch_strict_additional_properties(value: &mut serde_json::Value) {
    let serde_json::Value::Object(map) = value else {
        return;
    };

    let is_object_schema = map.get("type").and_then(|t| t.as_str()) == Some("object")
        || map.contains_key("properties");
    if is_object_schema && !map.contains_key("additionalProperties") {
        map.insert(
            "additionalProperties".to_string(),
            serde_json::Value::Bool(false),
        );
    }

    if let Some(props) = map.get_mut("properties").and_then(|p| p.as_object_mut()) {
        for v in props.values_mut() {
            patch_strict_additional_properties(v);
        }
    }

    for key in ["$defs", "definitions"] {
        if let Some(defs) = map.get_mut(key).and_then(|d| d.as_object_mut()) {
            for v in defs.values_mut() {
                patch_strict_additional_properties(v);
            }
        }
    }

    for key in ["anyOf", "allOf", "oneOf"] {
        if let Some(arr) = map.get_mut(key).and_then(|a| a.as_array_mut()) {
            for v in arr.iter_mut() {
                patch_strict_additional_properties(v);
            }
        }
    }

    match map.get_mut("items") {
        Some(serde_json::Value::Array(arr)) => {
            for v in arr.iter_mut() {
                patch_strict_additional_properties(v);
            }
        }
        Some(other) => patch_strict_additional_properties(other),
        None => {}
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
    pub usage: Option<Usage>,
}

/// Supported image output formats.
#[derive(Debug, Clone, Copy, PartialEq)]
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

impl ImageFormat {
    /// Lowercase name as used in provider request payloads (`"png"`, `"jpeg"`, `"webp"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Webp => "webp",
        }
    }
}

impl std::str::FromStr for ImageFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "png" => Ok(Self::Png),
            "jpeg" => Ok(Self::Jpeg),
            "webp" => Ok(Self::Webp),
            _ => Err(format!(
                "Unknown image format '{}'. Valid: png, jpeg, webp",
                s
            )),
        }
    }
}

/// Background transparency for image generation (gpt-image models).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Background {
    Transparent,
    Opaque,
    Auto,
}

impl Background {
    pub fn as_str(&self) -> &'static str {
        match self {
            Background::Transparent => "transparent",
            Background::Opaque => "opaque",
            Background::Auto => "auto",
        }
    }
}

impl std::str::FromStr for Background {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "transparent" => Ok(Self::Transparent),
            "opaque" => Ok(Self::Opaque),
            "auto" => Ok(Self::Auto),
            _ => Err(format!(
                "Unknown background '{}'. Valid: transparent, opaque, auto",
                s
            )),
        }
    }
}

/// Moderation strictness for image generation (gpt-image models).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Moderation {
    Auto,
    Low,
}

impl Moderation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Moderation::Auto => "auto",
            Moderation::Low => "low",
        }
    }
}

impl std::str::FromStr for Moderation {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "low" => Ok(Self::Low),
            _ => Err(format!(
                "Unknown moderation level '{}'. Valid: auto, low",
                s
            )),
        }
    }
}

/// How closely edited images should preserve details from reference images
/// (gpt-image models).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputFidelity {
    High,
    Low,
}

impl InputFidelity {
    pub fn as_str(&self) -> &'static str {
        match self {
            InputFidelity::High => "high",
            InputFidelity::Low => "low",
        }
    }
}

impl std::str::FromStr for InputFidelity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "high" => Ok(Self::High),
            "low" => Ok(Self::Low),
            _ => Err(format!("Unknown input fidelity '{}'. Valid: high, low", s)),
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
    /// DALL·E-only style hint (`"natural"` / `"vivid"`); not supported by
    /// gpt-image models. Prefer the other fields on this struct for
    /// gpt-image-2 requests.
    pub style: Option<String>,
    /// Output image encoding for gpt-image models.
    pub output_format: Option<ImageFormat>,
    /// Compression level 0–100, only valid alongside `output_format` of
    /// `Jpeg` or `Webp`.
    pub compression: Option<u8>,
    /// Number of images to generate, 1–10.
    pub n: Option<u8>,
    pub background: Option<Background>,
    pub moderation: Option<Moderation>,
    /// Only meaningful when `reference_images` is non-empty.
    pub input_fidelity: Option<InputFidelity>,
    /// Reference images to edit/compose from (gpt-image models).
    pub reference_images: Vec<std::path::PathBuf>,
    /// Mask image for inpainting; requires `reference_images` to be set.
    pub mask: Option<std::path::PathBuf>,
}

impl ImageOptions {
    pub fn builder() -> ImageOptionsBuilder {
        ImageOptionsBuilder::default()
    }

    /// Validate the parameter combination, returning an actionable error
    /// describing what to change if the options are inconsistent.
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(compression) = self.compression {
            if compression > 100 {
                anyhow::bail!(
                    "Invalid image compression {}: must be between 0 and 100.",
                    compression
                );
            }
            match self.output_format {
                None => {
                    anyhow::bail!(
                        "compression requires output_format to be set to jpeg or webp (compression does not apply to the default png format)."
                    );
                }
                Some(ImageFormat::Png) => {
                    anyhow::bail!(
                        "compression is not supported with output_format png; set output_format to jpeg or webp, or remove compression."
                    );
                }
                Some(ImageFormat::Jpeg) | Some(ImageFormat::Webp) => {}
            }
        }

        if let Some(n) = self.n
            && !(1..=10).contains(&n)
        {
            anyhow::bail!("Invalid image count n={}: must be between 1 and 10.", n);
        }

        if self.reference_images.is_empty() {
            if self.mask.is_some() {
                anyhow::bail!(
                    "mask requires at least one reference image; add reference images via reference_image()/reference_images(), or remove mask."
                );
            }
            if self.input_fidelity.is_some() {
                anyhow::bail!(
                    "input_fidelity requires at least one reference image; add reference images via reference_image()/reference_images(), or remove input_fidelity."
                );
            }
        }

        if self.background == Some(Background::Transparent)
            && self.output_format == Some(ImageFormat::Jpeg)
        {
            anyhow::bail!(
                "background transparent is not supported with output_format jpeg (jpeg has no alpha channel); use png or webp, or set background to opaque/auto."
            );
        }

        Ok(())
    }
}

/// Builder for [`ImageOptions`].
#[derive(Debug, Default)]
pub struct ImageOptionsBuilder {
    size: Option<(u32, u32)>,
    quality: Option<String>,
    style: Option<String>,
    output_format: Option<ImageFormat>,
    compression: Option<u8>,
    n: Option<u8>,
    background: Option<Background>,
    moderation: Option<Moderation>,
    input_fidelity: Option<InputFidelity>,
    reference_images: Vec<std::path::PathBuf>,
    mask: Option<std::path::PathBuf>,
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

    /// DALL·E-only style hint; not supported by gpt-image models. Use
    /// `output_format`, `background`, `moderation`, etc. instead.
    #[deprecated(
        since = "2.0.0",
        note = "DALL·E-only; not supported by gpt-image models"
    )]
    pub fn style(mut self, style: impl Into<String>) -> Self {
        self.style = Some(style.into());
        self
    }

    pub fn output_format(mut self, format: ImageFormat) -> Self {
        self.output_format = Some(format);
        self
    }

    pub fn compression(mut self, compression: u8) -> Self {
        self.compression = Some(compression);
        self
    }

    pub fn n(mut self, n: u8) -> Self {
        self.n = Some(n);
        self
    }

    pub fn background(mut self, background: Background) -> Self {
        self.background = Some(background);
        self
    }

    pub fn moderation(mut self, moderation: Moderation) -> Self {
        self.moderation = Some(moderation);
        self
    }

    pub fn input_fidelity(mut self, input_fidelity: InputFidelity) -> Self {
        self.input_fidelity = Some(input_fidelity);
        self
    }

    /// Push a single reference image path.
    pub fn reference_image(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.reference_images.push(path.into());
        self
    }

    /// Set the full list of reference image paths, replacing any previously
    /// added via [`Self::reference_image`].
    pub fn reference_images(mut self, paths: Vec<std::path::PathBuf>) -> Self {
        self.reference_images = paths;
        self
    }

    pub fn mask(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.mask = Some(path.into());
        self
    }

    pub fn build(self) -> ImageOptions {
        ImageOptions {
            size: self.size,
            quality: self.quality,
            style: self.style,
            output_format: self.output_format,
            compression: self.compression,
            n: self.n,
            background: self.background,
            moderation: self.moderation,
            input_fidelity: self.input_fidelity,
            reference_images: self.reference_images,
            mask: self.mask,
        }
    }
}

/// Response from an embedding request.
#[derive(Debug, Clone)]
pub struct EmbedResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub model: String,
    pub usage: Option<Usage>,
}

/// Options for embedding generation.
#[derive(Debug, Clone, Default)]
pub struct EmbedOptions {
    pub dimensions: Option<u32>,
}

impl EmbedOptions {
    pub fn builder() -> EmbedOptionsBuilder {
        EmbedOptionsBuilder::default()
    }
}

/// Builder for [`EmbedOptions`].
#[derive(Debug, Default)]
pub struct EmbedOptionsBuilder {
    dimensions: Option<u32>,
}

impl EmbedOptionsBuilder {
    pub fn dimensions(mut self, dimensions: u32) -> Self {
        self.dimensions = Some(dimensions);
        self
    }

    pub fn build(self) -> EmbedOptions {
        EmbedOptions {
            dimensions: self.dimensions,
        }
    }
}

/// Options for video generation.
#[derive(Debug, Clone, Default)]
pub struct VideoOptions {
    pub size: Option<(u32, u32)>,
    /// Clip duration in seconds. The Videos API accepts a model-dependent set
    /// (commonly 4, 8, or 12); this crate only sanity-checks a 1–20 range and
    /// lets the API reject unsupported values.
    pub seconds: Option<u32>,
    /// Number of video variants to generate, 1–5. The Videos API has no
    /// multi-variant field, so this is implemented as N separate video
    /// creations.
    pub variants: Option<u8>,
}

impl VideoOptions {
    pub fn builder() -> VideoOptionsBuilder {
        VideoOptionsBuilder::default()
    }

    /// Validate the parameter ranges, returning an actionable error
    /// describing what to change if the options are out of range.
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(seconds) = self.seconds
            && !(1..=20).contains(&seconds)
        {
            anyhow::bail!(
                "Invalid video duration seconds={}: must be between 1 and 20.",
                seconds
            );
        }

        if let Some(variants) = self.variants
            && !(1..=5).contains(&variants)
        {
            anyhow::bail!(
                "Invalid video variants={}: must be between 1 and 5.",
                variants
            );
        }

        Ok(())
    }
}

/// Builder for [`VideoOptions`].
#[derive(Debug, Default)]
pub struct VideoOptionsBuilder {
    size: Option<(u32, u32)>,
    seconds: Option<u32>,
    variants: Option<u8>,
}

impl VideoOptionsBuilder {
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width, height));
        self
    }

    pub fn seconds(mut self, seconds: u32) -> Self {
        self.seconds = Some(seconds);
        self
    }

    pub fn variants(mut self, variants: u8) -> Self {
        self.variants = Some(variants);
        self
    }

    pub fn build(self) -> VideoOptions {
        VideoOptions {
            size: self.size,
            seconds: self.seconds,
            variants: self.variants,
        }
    }
}

/// Lifecycle status of an asynchronous video generation job.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoJobStatus {
    Queued,
    Preprocessing,
    Running,
    Processing,
    Succeeded,
    Failed,
    Cancelled,
}

impl VideoJobStatus {
    /// True once the job has reached a terminal state (no further polling
    /// will change the outcome).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            VideoJobStatus::Succeeded | VideoJobStatus::Failed | VideoJobStatus::Cancelled
        )
    }
}

impl std::fmt::Display for VideoJobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            VideoJobStatus::Queued => "queued",
            VideoJobStatus::Preprocessing => "preprocessing",
            VideoJobStatus::Running => "running",
            VideoJobStatus::Processing => "processing",
            VideoJobStatus::Succeeded => "succeeded",
            VideoJobStatus::Failed => "failed",
            VideoJobStatus::Cancelled => "cancelled",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for VideoJobStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Self::Queued),
            "preprocessing" => Ok(Self::Preprocessing),
            // Videos API wire string for an in-flight generation.
            "running" | "in_progress" => Ok(Self::Running),
            "processing" => Ok(Self::Processing),
            // Videos API wire string for a finished generation.
            "succeeded" | "completed" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!(
                "Unknown video job status '{}'. Valid: queued, preprocessing, running, in_progress, processing, succeeded, completed, failed, cancelled",
                s
            )),
        }
    }
}

/// State of an asynchronous video generation job, as returned by a job
/// status poll.
///
/// On Azure OpenAI / Microsoft Foundry, which drive the OpenAI-style Videos
/// API, a single-variant job's `id` is a plain OpenAI video id (`video_…`).
/// When more than one variant is requested, there is no server-side job that
/// groups them, so `id` is the individual video ids **joined with `+`** (e.g.
/// `video_a+video_b`) and `generation_ids` holds the same ids as a list. The
/// low-level `get_video_job` / `delete_video_job` calls accept that composite
/// id and fan out across the constituent videos.
#[derive(Debug, Clone)]
pub struct VideoJob {
    pub id: String,
    pub status: VideoJobStatus,
    pub generation_ids: Vec<String>,
    pub failure_reason: Option<String>,
}

/// Response from a completed video generation request.
#[derive(Debug, Clone)]
pub struct VideoResponse {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub duration_seconds: u32,
}

/// Callback invoked with the latest job state while polling a video
/// generation job.
pub type VideoProgress<'a> = &'a (dyn Fn(&VideoJob) + Send + Sync);

/// Task types for provider routing.
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    Chat,
    ImageGeneration,
    VideoGeneration,
    Transcription,
    Embedding,
}

impl Task {
    /// Returns the config key for this task (used in `defaults` map).
    pub fn config_key(&self) -> &str {
        match self {
            Self::Chat => "chat",
            Self::ImageGeneration => "image",
            Self::VideoGeneration => "video",
            Self::Transcription => "transcription",
            Self::Embedding => "embedding",
        }
    }

    /// Convert this task to a [`Capability`](crate::config::Capability), if applicable.
    pub fn to_capability(&self) -> Option<crate::config::Capability> {
        match self {
            Self::Chat => Some(crate::config::Capability::Chat),
            Self::ImageGeneration => Some(crate::config::Capability::Image),
            Self::VideoGeneration => Some(crate::config::Capability::Video),
            Self::Transcription => None,
            Self::Embedding => Some(crate::config::Capability::Embedding),
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
        assert_eq!(msg.content.text(), "hello");
        assert_eq!(msg.content.as_text(), Some("hello"));

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
        assert_eq!(parsed.content.text(), "test");
    }

    // --- Task 3.1: MessageContent enum + attachment parts ---

    #[test]
    fn text_only_serializes_byte_identical_to_1x_shape() {
        // A text-only message must produce exactly the 1.x wire shape:
        // content is a bare string, not a tagged object or array.
        let msg = Message::user("hello world");
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"role":"user","content":"hello world"}"#);
    }

    #[test]
    fn plain_string_content_deserializes_to_text() {
        // 1.x on-disk/history JSON with a bare-string content must still load.
        let msg: Message = serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap();
        assert_eq!(msg.role, Role::User);
        assert!(matches!(msg.content, MessageContent::Text(ref s) if s == "hi"));
        assert_eq!(msg.content.text(), "hi");
    }

    #[test]
    fn message_content_json_roundtrip_text() {
        let c = MessageContent::from("plain");
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#""plain""#);
        let back: MessageContent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn message_content_yaml_roundtrip_text() {
        let msg = Message::assistant("yaml me");
        let yaml = serde_yaml::to_string(&msg).unwrap();
        // content renders as a bare scalar, not a mapping/sequence.
        assert!(yaml.contains("content: yaml me"), "yaml was: {yaml}");
        let back: Message = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.content.text(), "yaml me");
    }

    #[test]
    fn parts_roundtrip_with_base64_data() {
        let parts = vec![
            ContentPart::Text {
                text: "look at this".to_string(),
            },
            ContentPart::Image {
                data: vec![0xDE, 0xAD, 0xBE, 0xEF],
                media_type: "image/png".to_string(),
            },
            ContentPart::File {
                data: b"col1,col2\n1,2\n".to_vec(),
                media_type: "text/csv".to_string(),
                filename: "data.csv".to_string(),
            },
        ];
        let content = MessageContent::from(parts.clone());
        let json = serde_json::to_string(&content).unwrap();
        // Image bytes must appear as base64 (DEADBEEF => 3q2+7w==), not raw.
        assert!(json.contains("3q2+7w=="), "json was: {json}");
        assert!(json.contains(r#""type":"image""#));
        let back: MessageContent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, content);

        // YAML round-trip too (history/config can be YAML).
        let yaml = serde_yaml::to_string(&content).unwrap();
        let back_yaml: MessageContent = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back_yaml, content);
    }

    #[test]
    fn text_helpers_on_parts() {
        let content = MessageContent::from(vec![
            ContentPart::Text {
                text: "line one".to_string(),
            },
            ContentPart::Image {
                data: vec![1, 2, 3],
                media_type: "image/png".to_string(),
            },
            ContentPart::Text {
                text: "line two".to_string(),
            },
        ]);
        assert_eq!(content.text(), "line one\nline two");
        assert_eq!(content.as_text(), None);
        assert!(content.has_attachments());

        let text = MessageContent::from("just text");
        assert_eq!(text.text(), "just text");
        assert_eq!(text.as_text(), Some("just text"));
        assert!(!text.has_attachments());
        assert_eq!(format!("{text}"), "just text");
    }

    #[test]
    fn user_with_attachments_png_and_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("pic.png");
        // Minimal fake PNG bytes (content isn't validated here).
        std::fs::write(&png, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
        let pdf = dir.path().join("doc.pdf");
        std::fs::write(&pdf, b"%PDF-1.4 fake").unwrap();

        let msg =
            Message::user_with_attachments("describe these", &[png.clone(), pdf.clone()]).unwrap();
        assert_eq!(msg.role, Role::User);
        assert!(msg.content.has_attachments());
        assert_eq!(msg.content.text(), "describe these");

        let MessageContent::Parts(parts) = &msg.content else {
            panic!("expected Parts");
        };
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "describe these"));
        assert!(
            matches!(&parts[1], ContentPart::Image { media_type, .. } if media_type == "image/png")
        );
        assert!(matches!(
            &parts[2],
            ContentPart::File { media_type, filename, .. }
                if media_type == "application/pdf" && filename == "doc.pdf"
        ));
    }

    #[test]
    fn user_with_attachments_unknown_extension_errors() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("blob.bin");
        std::fs::write(&bin, [0u8; 4]).unwrap();
        let err = Message::user_with_attachments("x", &[bin])
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported attachment type"), "err: {err}");
        assert!(err.contains("bin"), "err: {err}");
        assert!(err.contains("supported"), "err: {err}");
    }

    #[test]
    fn user_with_attachments_missing_file_errors() {
        let missing = std::path::PathBuf::from("/nonexistent/definitely/missing.png");
        let err = Message::user_with_attachments("x", std::slice::from_ref(&missing))
            .unwrap_err()
            .to_string();
        assert!(err.contains("Failed to read attachment"), "err: {err}");
        assert!(err.contains("missing.png"), "err: {err}");
    }

    // --- Task 3.2: OpenAI-family multimodal wire mapping ---

    #[test]
    fn to_openai_wire_text_only_keeps_plain_string_content() {
        // Regression: a text-only message must serialize with `content` as a
        // bare string (1.x wire shape), never an array.
        let msgs = vec![Message::user("hello world")];
        let wire = to_openai_wire(&msgs);
        assert_eq!(
            wire,
            serde_json::json!([{"role": "user", "content": "hello world"}])
        );
        assert!(
            wire[0]["content"].is_string(),
            "content must stay a plain string, got: {}",
            wire[0]["content"]
        );
    }

    #[test]
    fn to_openai_wire_text_image_file_full_shape() {
        let content = MessageContent::from(vec![
            ContentPart::Text {
                text: "look".to_string(),
            },
            ContentPart::Image {
                data: vec![0xDE, 0xAD, 0xBE, 0xEF],
                media_type: "image/png".to_string(),
            },
            ContentPart::File {
                data: b"hi".to_vec(),
                media_type: "application/pdf".to_string(),
                filename: "doc.pdf".to_string(),
            },
        ]);
        let msgs = vec![Message::user(content)];
        let wire = to_openai_wire(&msgs);
        assert_eq!(
            wire,
            serde_json::json!([{
                "role": "user",
                "content": [
                    {"type": "text", "text": "look"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,3q2+7w=="}},
                    {"type": "file", "file": {"filename": "doc.pdf", "file_data": "data:application/pdf;base64,aGk="}},
                ]
            }])
        );
    }

    #[test]
    fn to_openai_wire_multi_message_mix() {
        let msgs = vec![
            Message::system("be brief"),
            Message::user(MessageContent::from(vec![
                ContentPart::Text {
                    text: "what is this".to_string(),
                },
                ContentPart::Image {
                    data: vec![0x00],
                    media_type: "image/jpeg".to_string(),
                },
            ])),
            Message::assistant("a cat"),
        ];
        let wire = to_openai_wire(&msgs);
        // System + assistant stay plain strings; the user message is an array.
        assert_eq!(
            wire[0],
            serde_json::json!({"role": "system", "content": "be brief"})
        );
        assert!(wire[1]["content"].is_array());
        assert_eq!(wire[1]["content"][1]["type"], "image_url");
        assert_eq!(
            wire[1]["content"][1]["image_url"]["url"],
            "data:image/jpeg;base64,AA=="
        );
        assert_eq!(
            wire[2],
            serde_json::json!({"role": "assistant", "content": "a cat"})
        );
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
    #[allow(deprecated)]
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

    #[test]
    fn test_embed_options_builder() {
        let opts = EmbedOptions::builder().dimensions(1536).build();
        assert_eq!(opts.dimensions, Some(1536));
    }

    #[test]
    fn test_embed_options_default() {
        let opts = EmbedOptions::default();
        assert!(opts.dimensions.is_none());
    }

    #[test]
    fn test_task_embedding_config_key() {
        assert_eq!(Task::Embedding.config_key(), "embedding");
    }

    #[test]
    fn test_task_embedding_to_capability() {
        assert_eq!(
            Task::Embedding.to_capability(),
            Some(crate::config::Capability::Embedding),
        );
    }

    #[test]
    fn video_task_config_key() {
        assert_eq!(Task::VideoGeneration.config_key(), "video");
        assert_eq!(
            Task::VideoGeneration.to_capability(),
            Some(crate::config::Capability::Video),
        );
    }

    // --- Task 1.2: full gpt-image parameter surface ---

    #[test]
    #[allow(deprecated)]
    fn test_image_options_builder_full_roundtrip() {
        let opts = ImageOptions::builder()
            .size(1024, 1024)
            .quality("hd")
            .style("natural")
            .output_format(ImageFormat::Jpeg)
            .compression(80)
            .n(4)
            .background(Background::Opaque)
            .moderation(Moderation::Low)
            .input_fidelity(InputFidelity::High)
            .reference_image(std::path::PathBuf::from("ref1.png"))
            .reference_image(std::path::PathBuf::from("ref2.png"))
            .mask(std::path::PathBuf::from("mask.png"))
            .build();

        assert_eq!(opts.size, Some((1024, 1024)));
        assert_eq!(opts.quality.as_deref(), Some("hd"));
        assert_eq!(opts.style.as_deref(), Some("natural"));
        assert_eq!(opts.output_format, Some(ImageFormat::Jpeg));
        assert_eq!(opts.compression, Some(80));
        assert_eq!(opts.n, Some(4));
        assert_eq!(opts.background, Some(Background::Opaque));
        assert_eq!(opts.moderation, Some(Moderation::Low));
        assert_eq!(opts.input_fidelity, Some(InputFidelity::High));
        assert_eq!(
            opts.reference_images,
            vec![
                std::path::PathBuf::from("ref1.png"),
                std::path::PathBuf::from("ref2.png"),
            ]
        );
        assert_eq!(opts.mask, Some(std::path::PathBuf::from("mask.png")));
    }

    #[test]
    fn test_image_options_builder_reference_images_bulk() {
        let paths = vec![
            std::path::PathBuf::from("a.png"),
            std::path::PathBuf::from("b.png"),
        ];
        let opts = ImageOptions::builder()
            .reference_images(paths.clone())
            .build();
        assert_eq!(opts.reference_images, paths);
    }

    #[test]
    fn test_image_format_as_str() {
        assert_eq!(ImageFormat::Png.as_str(), "png");
        assert_eq!(ImageFormat::Jpeg.as_str(), "jpeg");
        assert_eq!(ImageFormat::Webp.as_str(), "webp");
    }

    #[test]
    fn test_image_format_from_str() {
        assert_eq!("png".parse::<ImageFormat>().unwrap(), ImageFormat::Png);
        assert_eq!("jpeg".parse::<ImageFormat>().unwrap(), ImageFormat::Jpeg);
        assert_eq!("webp".parse::<ImageFormat>().unwrap(), ImageFormat::Webp);
    }

    #[test]
    fn test_image_format_from_str_invalid() {
        let err = "bmp".parse::<ImageFormat>().unwrap_err();
        assert!(err.contains("bmp"));
        assert!(err.contains("png"));
        assert!(err.contains("jpeg"));
        assert!(err.contains("webp"));
    }

    #[test]
    fn test_background_as_str_and_from_str() {
        assert_eq!(Background::Transparent.as_str(), "transparent");
        assert_eq!(Background::Opaque.as_str(), "opaque");
        assert_eq!(Background::Auto.as_str(), "auto");
        assert_eq!(
            "transparent".parse::<Background>().unwrap(),
            Background::Transparent
        );
        assert_eq!("opaque".parse::<Background>().unwrap(), Background::Opaque);
        assert_eq!("auto".parse::<Background>().unwrap(), Background::Auto);
    }

    #[test]
    fn test_background_from_str_invalid() {
        let err = "invisible".parse::<Background>().unwrap_err();
        assert!(err.contains("invisible"));
        assert!(err.contains("transparent"));
        assert!(err.contains("opaque"));
        assert!(err.contains("auto"));
    }

    #[test]
    fn test_moderation_as_str_and_from_str() {
        assert_eq!(Moderation::Auto.as_str(), "auto");
        assert_eq!(Moderation::Low.as_str(), "low");
        assert_eq!("auto".parse::<Moderation>().unwrap(), Moderation::Auto);
        assert_eq!("low".parse::<Moderation>().unwrap(), Moderation::Low);
    }

    #[test]
    fn test_moderation_from_str_invalid() {
        let err = "strict".parse::<Moderation>().unwrap_err();
        assert!(err.contains("strict"));
        assert!(err.contains("auto"));
        assert!(err.contains("low"));
    }

    #[test]
    fn test_input_fidelity_as_str_and_from_str() {
        assert_eq!(InputFidelity::High.as_str(), "high");
        assert_eq!(InputFidelity::Low.as_str(), "low");
        assert_eq!(
            "high".parse::<InputFidelity>().unwrap(),
            InputFidelity::High
        );
        assert_eq!("low".parse::<InputFidelity>().unwrap(), InputFidelity::Low);
    }

    #[test]
    fn test_input_fidelity_from_str_invalid() {
        let err = "medium".parse::<InputFidelity>().unwrap_err();
        assert!(err.contains("medium"));
        assert!(err.contains("high"));
        assert!(err.contains("low"));
    }

    #[test]
    fn test_validate_default_ok() {
        assert!(ImageOptions::default().validate().is_ok());
    }

    #[test]
    fn test_validate_compression_too_high_err() {
        let opts = ImageOptions {
            output_format: Some(ImageFormat::Jpeg),
            compression: Some(101),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("compression"));
        assert!(err.contains("100"));
    }

    #[test]
    fn test_validate_compression_at_max_ok() {
        let opts = ImageOptions {
            output_format: Some(ImageFormat::Jpeg),
            compression: Some(100),
            ..Default::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn test_validate_compression_without_output_format_err() {
        let opts = ImageOptions {
            compression: Some(50),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("compression"));
        assert!(err.contains("output_format"));
    }

    #[test]
    fn test_validate_compression_with_png_err() {
        let opts = ImageOptions {
            output_format: Some(ImageFormat::Png),
            compression: Some(50),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("compression"));
        assert!(err.contains("PNG") || err.contains("png"));
    }

    #[test]
    fn test_validate_compression_with_jpeg_ok() {
        let opts = ImageOptions {
            output_format: Some(ImageFormat::Jpeg),
            compression: Some(50),
            ..Default::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn test_validate_compression_with_webp_ok() {
        let opts = ImageOptions {
            output_format: Some(ImageFormat::Webp),
            compression: Some(50),
            ..Default::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn test_validate_n_too_low_err() {
        let opts = ImageOptions {
            n: Some(0),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains('n'));
        assert!(err.contains('1'));
        assert!(err.contains("10"));
    }

    #[test]
    fn test_validate_n_too_high_err() {
        let opts = ImageOptions {
            n: Some(11),
            ..Default::default()
        };
        assert!(opts.validate().is_err());
    }

    #[test]
    fn test_validate_n_in_range_ok() {
        for n in 1..=10u8 {
            let opts = ImageOptions {
                n: Some(n),
                ..Default::default()
            };
            assert!(opts.validate().is_ok(), "n={n} should be valid");
        }
    }

    #[test]
    fn test_validate_mask_without_reference_images_err() {
        let opts = ImageOptions {
            mask: Some(std::path::PathBuf::from("mask.png")),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("mask"));
        assert!(err.contains("reference"));
    }

    #[test]
    fn test_validate_mask_with_reference_images_ok() {
        let opts = ImageOptions {
            mask: Some(std::path::PathBuf::from("mask.png")),
            reference_images: vec![std::path::PathBuf::from("ref.png")],
            ..Default::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn test_validate_input_fidelity_without_reference_images_err() {
        let opts = ImageOptions {
            input_fidelity: Some(InputFidelity::High),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("input_fidelity"));
        assert!(err.contains("reference"));
    }

    #[test]
    fn test_validate_input_fidelity_with_reference_images_ok() {
        let opts = ImageOptions {
            input_fidelity: Some(InputFidelity::High),
            reference_images: vec![std::path::PathBuf::from("ref.png")],
            ..Default::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn test_validate_transparent_background_with_jpeg_err() {
        let opts = ImageOptions {
            output_format: Some(ImageFormat::Jpeg),
            background: Some(Background::Transparent),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("background"));
        assert!(err.contains("jpeg") || err.contains("JPEG"));
    }

    #[test]
    fn test_validate_transparent_background_with_png_ok() {
        let opts = ImageOptions {
            output_format: Some(ImageFormat::Png),
            background: Some(Background::Transparent),
            ..Default::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn test_validate_transparent_background_with_no_format_ok() {
        let opts = ImageOptions {
            background: Some(Background::Transparent),
            ..Default::default()
        };
        assert!(opts.validate().is_ok());
    }

    // --- Task 2.1: video generation types ---

    #[test]
    fn test_video_options_builder_roundtrip() {
        let opts = VideoOptions::builder()
            .size(1280, 720)
            .seconds(10)
            .variants(2)
            .build();
        assert_eq!(opts.size, Some((1280, 720)));
        assert_eq!(opts.seconds, Some(10));
        assert_eq!(opts.variants, Some(2));
    }

    #[test]
    fn test_video_options_default() {
        let opts = VideoOptions::default();
        assert!(opts.size.is_none());
        assert!(opts.seconds.is_none());
        assert!(opts.variants.is_none());
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn test_video_options_validate_seconds_in_range_ok() {
        for seconds in [1, 20] {
            let opts = VideoOptions {
                seconds: Some(seconds),
                ..Default::default()
            };
            assert!(opts.validate().is_ok(), "seconds={seconds} should be valid");
        }
    }

    #[test]
    fn test_video_options_validate_seconds_too_low_err() {
        let opts = VideoOptions {
            seconds: Some(0),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("seconds"));
        assert!(err.contains('1'));
        assert!(err.contains("20"));
    }

    #[test]
    fn test_video_options_validate_seconds_too_high_err() {
        let opts = VideoOptions {
            seconds: Some(21),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("seconds"));
        assert!(err.contains("20"));
    }

    #[test]
    fn test_video_options_validate_variants_in_range_ok() {
        for variants in 1..=5u8 {
            let opts = VideoOptions {
                variants: Some(variants),
                ..Default::default()
            };
            assert!(
                opts.validate().is_ok(),
                "variants={variants} should be valid"
            );
        }
    }

    #[test]
    fn test_video_options_validate_variants_too_low_err() {
        let opts = VideoOptions {
            variants: Some(0),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("variants"));
        assert!(err.contains('1'));
        assert!(err.contains('5'));
    }

    #[test]
    fn test_video_options_validate_variants_too_high_err() {
        let opts = VideoOptions {
            variants: Some(6),
            ..Default::default()
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(err.contains("variants"));
        assert!(err.contains('5'));
    }

    #[test]
    fn test_video_job_status_from_str_all_variants() {
        assert_eq!(
            "queued".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Queued
        );
        assert_eq!(
            "preprocessing".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Preprocessing
        );
        assert_eq!(
            "running".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Running
        );
        assert_eq!(
            "processing".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Processing
        );
        assert_eq!(
            "succeeded".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Succeeded
        );
        assert_eq!(
            "failed".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Failed
        );
        assert_eq!(
            "cancelled".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Cancelled
        );
    }

    #[test]
    fn test_video_job_status_from_str_videos_api_wire_strings() {
        // OpenAI-style /videos surface uses "in_progress" and "completed".
        assert_eq!(
            "in_progress".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Running
        );
        assert_eq!(
            "completed".parse::<VideoJobStatus>().unwrap(),
            VideoJobStatus::Succeeded
        );
    }

    #[test]
    fn test_video_job_status_from_str_invalid() {
        let err = "unknown".parse::<VideoJobStatus>().unwrap_err();
        assert!(err.contains("unknown"));
        assert!(err.contains("queued"));
        assert!(err.contains("preprocessing"));
        assert!(err.contains("running"));
        assert!(err.contains("processing"));
        assert!(err.contains("succeeded"));
        assert!(err.contains("failed"));
        assert!(err.contains("cancelled"));
    }

    #[test]
    fn test_video_job_status_is_terminal() {
        assert!(!VideoJobStatus::Queued.is_terminal());
        assert!(!VideoJobStatus::Preprocessing.is_terminal());
        assert!(!VideoJobStatus::Running.is_terminal());
        assert!(!VideoJobStatus::Processing.is_terminal());
        assert!(VideoJobStatus::Succeeded.is_terminal());
        assert!(VideoJobStatus::Failed.is_terminal());
        assert!(VideoJobStatus::Cancelled.is_terminal());
    }

    #[test]
    fn test_video_job_status_display() {
        assert_eq!(VideoJobStatus::Queued.to_string(), "queued");
        assert_eq!(VideoJobStatus::Preprocessing.to_string(), "preprocessing");
        assert_eq!(VideoJobStatus::Running.to_string(), "running");
        assert_eq!(VideoJobStatus::Processing.to_string(), "processing");
        assert_eq!(VideoJobStatus::Succeeded.to_string(), "succeeded");
        assert_eq!(VideoJobStatus::Failed.to_string(), "failed");
        assert_eq!(VideoJobStatus::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn test_video_job_construction() {
        let job = VideoJob {
            id: "job-123".to_string(),
            status: VideoJobStatus::Running,
            generation_ids: vec!["gen-1".to_string()],
            failure_reason: None,
        };
        assert_eq!(job.id, "job-123");
        assert_eq!(job.status, VideoJobStatus::Running);
        assert_eq!(job.generation_ids, vec!["gen-1".to_string()]);
        assert!(job.failure_reason.is_none());
    }

    #[test]
    fn test_video_response_construction() {
        let resp = VideoResponse {
            data: vec![1, 2, 3],
            width: 1280,
            height: 720,
            duration_seconds: 8,
        };
        assert_eq!(resp.data, vec![1, 2, 3]);
        assert_eq!(resp.width, 1280);
        assert_eq!(resp.height, 720);
        assert_eq!(resp.duration_seconds, 8);
    }

    #[test]
    fn test_video_progress_callback_type() {
        let seen = std::sync::Mutex::new(Vec::new());
        let cb = |job: &VideoJob| seen.lock().unwrap().push(job.status.clone());
        let progress: VideoProgress = &cb;
        let job = VideoJob {
            id: "job-1".to_string(),
            status: VideoJobStatus::Queued,
            generation_ids: vec![],
            failure_reason: None,
        };
        progress(&job);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

/// Does this error response mean the model rejected sampling parameters
/// (temperature/top_p) rather than the request being otherwise invalid?
///
/// Newer reasoning models (OpenAI gpt-5.x / o-series, the newest Claude and
/// Gemini models) accept only their default sampling settings and return
/// HTTP 400 for anything else. Providers retry once without the parameter.
pub(crate) fn is_sampling_rejection(status: u16, body: &str) -> bool {
    if status != 400 {
        return false;
    }
    let b = body.to_ascii_lowercase();
    let names_param = b.contains("temperature") || b.contains("top_p");
    let says_unsupported = b.contains("unsupported")
        || b.contains("not supported")
        || b.contains("does not support")
        || b.contains("only the default");
    names_param && says_unsupported
}

#[cfg(test)]
mod sampling_rejection_tests {
    use super::is_sampling_rejection;

    #[test]
    fn detects_openai_style_rejection() {
        let body = r#"{"error":{"message":"Unsupported value: 'temperature' does not support 0.3 with this model. Only the default (1) value is supported."}}"#;
        assert!(is_sampling_rejection(400, body));
    }

    #[test]
    fn detects_anthropic_style_rejection() {
        let body = r#"{"error":{"message":"`temperature` is not supported on this model."}}"#;
        assert!(is_sampling_rejection(400, body));
    }

    #[test]
    fn ignores_other_400s_and_statuses() {
        assert!(!is_sampling_rejection(400, "missing field `messages`"));
        assert!(!is_sampling_rejection(429, "temperature unsupported"));
        assert!(!is_sampling_rejection(401, "unauthorized"));
    }
}

#[cfg(test)]
mod response_format_tests {
    use super::*;

    #[test]
    fn openai_value_shapes() {
        assert_eq!(
            ResponseFormat::JsonObject.to_openai_value(),
            serde_json::json!({"type": "json_object"})
        );
        let f = ResponseFormat::JsonSchema {
            name: "verdict".into(),
            schema: serde_json::json!({"type": "object"}),
            strict: true,
        };
        let v = f.to_openai_value();
        assert_eq!(v["type"], "json_schema");
        assert_eq!(v["json_schema"]["name"], "verdict");
        assert_eq!(v["json_schema"]["strict"], true);
    }

    #[test]
    fn ollama_value_shapes() {
        assert_eq!(
            ResponseFormat::JsonObject.to_ollama_value(),
            serde_json::json!("json")
        );
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({"type": "object", "required": ["pass"]}),
            strict: true,
        };
        assert_eq!(f.to_ollama_value()["required"][0], "pass");
    }

    #[test]
    fn nudge_text_mentions_schema() {
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({"properties": {"pass": {"type": "boolean"}}}),
            strict: true,
        };
        assert!(f.nudge_text().contains("pass"));
        assert!(
            ResponseFormat::JsonObject
                .nudge_text()
                .contains("JSON object")
        );
    }

    // --- Task 6.2: strict schemas auto-patched with additionalProperties ---

    #[test]
    fn strict_schema_patches_top_level_object() {
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {"pass": {"type": "boolean"}}
            }),
            strict: true,
        };
        let v = f.to_openai_value();
        let schema = &v["json_schema"]["schema"];
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn strict_schema_patches_nested_properties_items_and_defs() {
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "child": {
                        "type": "object",
                        "properties": {"x": {"type": "string"}}
                    },
                    "list": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {"y": {"type": "number"}}
                        }
                    },
                    "ref": {"$ref": "#/$defs/Thing"}
                },
                "$defs": {
                    "Thing": {
                        "type": "object",
                        "properties": {"z": {"type": "boolean"}}
                    }
                }
            }),
            strict: true,
        };
        let v = f.to_openai_value();
        let s = &v["json_schema"]["schema"];
        assert_eq!(s["additionalProperties"], false);
        assert_eq!(s["properties"]["child"]["additionalProperties"], false);
        assert_eq!(
            s["properties"]["list"]["items"]["additionalProperties"],
            false
        );
        assert_eq!(s["$defs"]["Thing"]["additionalProperties"], false);
    }

    #[test]
    fn strict_schema_preserves_explicit_additional_properties() {
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({
                "type": "object",
                "additionalProperties": true,
                "properties": {"pass": {"type": "boolean"}}
            }),
            strict: true,
        };
        let v = f.to_openai_value();
        // Existing explicit value (true) must be preserved, not clobbered.
        assert_eq!(v["json_schema"]["schema"]["additionalProperties"], true);
    }

    #[test]
    fn strict_schema_leaves_non_object_untouched() {
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({"type": "string"}),
            strict: true,
        };
        let v = f.to_openai_value();
        assert!(
            v["json_schema"]["schema"]
                .get("additionalProperties")
                .is_none()
        );
    }

    #[test]
    fn strict_schema_patch_does_not_mutate_original() {
        // to_openai_value takes &self and clones, so the source stays intact.
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({"type": "object", "properties": {}}),
            strict: true,
        };
        let _ = f.to_openai_value();
        if let ResponseFormat::JsonSchema { schema, .. } = &f {
            assert!(schema.get("additionalProperties").is_none());
        } else {
            panic!("expected JsonSchema");
        }
    }

    #[test]
    fn non_strict_schema_is_not_patched() {
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({"type": "object", "properties": {}}),
            strict: false,
        };
        let v = f.to_openai_value();
        assert!(
            v["json_schema"]["schema"]
                .get("additionalProperties")
                .is_none()
        );
    }

    #[test]
    fn ollama_value_not_patched_for_strict_schema() {
        let f = ResponseFormat::JsonSchema {
            name: "v".into(),
            schema: serde_json::json!({"type": "object", "properties": {}}),
            strict: true,
        };
        let v = f.to_ollama_value();
        assert!(v.get("additionalProperties").is_none());
    }

    #[test]
    fn builder_sets_response_format() {
        let opts = ChatOptions::builder()
            .json_schema("verdict", serde_json::json!({"type": "object"}))
            .build();
        assert!(matches!(
            opts.response_format,
            Some(ResponseFormat::JsonSchema { .. })
        ));
        let opts = ChatOptions::builder().json().build();
        assert!(matches!(
            opts.response_format,
            Some(ResponseFormat::JsonObject)
        ));
    }
}
