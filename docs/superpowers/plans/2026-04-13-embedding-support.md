# Embedding Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add embedding as a first-class capability — types, Provider trait method, five provider implementations, metadata/Azure-vectorizer export, blocking client, and CLI command.

**Architecture:** Follows the existing pattern: `Capability::Embedding` + `Task::Embedding` for config routing, `Provider::embed()` trait method with default `Unsupported`, per-provider HTTP implementations, `EmbeddingMetadata` struct for config export, `to_azure_search_vectorizer()` helper, and `ailloy embed` CLI command.

**Tech Stack:** Rust, reqwest, serde, async-trait, clap, tokio, serde_json

---

### Task 1: Add embedding types (`types.rs`)

**Files:**
- Modify: `src/types.rs`

- [ ] **Step 1: Write the failing test for EmbedResponse and EmbedOptions**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src/types.rs`:

```rust
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
        Some(crate::config::Capability::Embedding)
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib types::tests::test_embed_options_builder 2>&1 | tail -5`
Expected: FAIL — `EmbedOptions` not defined, `Task::Embedding` not defined

- [ ] **Step 3: Add EmbedResponse, EmbedOptions, EmbedOptionsBuilder, and Task::Embedding**

Add after the `ImageOptions` / `ImageOptionsBuilder` block (after line 239 in `src/types.rs`):

```rust
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
```

Add `Embedding` variant to the `Task` enum:

```rust
pub enum Task {
    Chat,
    ImageGeneration,
    Transcription,
    Embedding,
}
```

Update the `Task` impl:

```rust
impl Task {
    pub fn config_key(&self) -> &str {
        match self {
            Self::Chat => "chat",
            Self::ImageGeneration => "image",
            Self::Transcription => "transcription",
            Self::Embedding => "embedding",
        }
    }

    pub fn to_capability(&self) -> Option<crate::config::Capability> {
        match self {
            Self::Chat => Some(crate::config::Capability::Chat),
            Self::ImageGeneration => Some(crate::config::Capability::Image),
            Self::Embedding => Some(crate::config::Capability::Embedding),
            Self::Transcription => None,
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib types::tests 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/types.rs
git commit -m "feat: add EmbedResponse, EmbedOptions, and Task::Embedding types"
```

---

### Task 2: Add Capability::Embedding and update config (`config.rs`)

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/config.rs`:

```rust
#[test]
fn test_capability_embedding_config_key() {
    assert_eq!(Capability::Embedding.config_key(), "embedding");
    assert_eq!(Capability::Embedding.label(), "Embedding");
}

#[test]
fn test_supports_task_embedding() {
    assert!(ProviderKind::OpenAi.supports_task("embedding"));
    assert!(ProviderKind::AzureOpenAi.supports_task("embedding"));
    assert!(ProviderKind::Ollama.supports_task("embedding"));
    assert!(ProviderKind::VertexAi.supports_task("embedding"));
    assert!(ProviderKind::MicrosoftFoundry.supports_task("embedding"));
    assert!(!ProviderKind::Anthropic.supports_task("embedding"));
    assert!(!ProviderKind::LocalAgent.supports_task("embedding"));
}

#[test]
fn test_supported_capabilities_includes_embedding() {
    let caps = ProviderKind::OpenAi.supported_capabilities();
    assert!(caps.contains(&Capability::Embedding));
    let caps = ProviderKind::Anthropic.supported_capabilities();
    assert!(!caps.contains(&Capability::Embedding));
}

#[test]
fn test_embedding_metadata_azure() {
    let node = AiNode {
        provider: ProviderKind::AzureOpenAi,
        alias: None,
        capabilities: vec![Capability::Embedding],
        auth: None,
        model: Some("text-embedding-3-large".to_string()),
        endpoint: Some("https://myresource.openai.azure.com".to_string()),
        deployment: Some("text-embedding-3-large".to_string()),
        api_version: None,
        binary: None,
        project: None,
        location: None,
        node_defaults: Some(BTreeMap::from([("dimensions".to_string(), "3072".to_string())])),
    };
    let meta = node.embedding_metadata();
    assert_eq!(meta.provider, ProviderKind::AzureOpenAi);
    assert_eq!(meta.model.as_deref(), Some("text-embedding-3-large"));
    assert_eq!(meta.endpoint.as_deref(), Some("https://myresource.openai.azure.com"));
    assert_eq!(meta.deployment.as_deref(), Some("text-embedding-3-large"));
    assert_eq!(meta.dimensions, Some(3072));
}

#[test]
fn test_azure_search_vectorizer() {
    let meta = EmbeddingMetadata {
        provider: ProviderKind::AzureOpenAi,
        model: Some("text-embedding-3-large".to_string()),
        endpoint: Some("https://myresource.openai.azure.com".to_string()),
        deployment: Some("text-embedding-3-large".to_string()),
        dimensions: Some(3072),
    };
    let vectorizer = meta.to_azure_search_vectorizer("my-vectorizer").unwrap();
    assert_eq!(vectorizer["name"], "my-vectorizer");
    assert_eq!(vectorizer["kind"], "azureOpenAI");
    assert_eq!(
        vectorizer["azureOpenAIParameters"]["resourceUri"],
        "https://myresource.openai.azure.com"
    );
    assert_eq!(
        vectorizer["azureOpenAIParameters"]["deploymentId"],
        "text-embedding-3-large"
    );
    assert_eq!(
        vectorizer["azureOpenAIParameters"]["modelName"],
        "text-embedding-3-large"
    );
}

#[test]
fn test_azure_search_vectorizer_non_azure_fails() {
    let meta = EmbeddingMetadata {
        provider: ProviderKind::OpenAi,
        model: Some("text-embedding-3-small".to_string()),
        endpoint: None,
        deployment: None,
        dimensions: None,
    };
    assert!(meta.to_azure_search_vectorizer("test").is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib config::tests::test_capability_embedding 2>&1 | tail -5`
Expected: FAIL — `Capability::Embedding` not defined

- [ ] **Step 3: Add Capability::Embedding**

In the `Capability` enum (around line 96), add:

```rust
pub enum Capability {
    Chat,
    Image,
    Embedding,
}
```

Update `Capability::config_key()`:

```rust
pub fn config_key(&self) -> &str {
    match self {
        Self::Chat => "chat",
        Self::Image => "image",
        Self::Embedding => "embedding",
    }
}
```

Update `Capability::label()`:

```rust
pub fn label(&self) -> &str {
    match self {
        Self::Chat => "Chat",
        Self::Image => "Image Generation",
        Self::Embedding => "Embedding",
    }
}
```

Update the `Display` impl for `Capability` if it has one (it delegates to `config_key()`).

- [ ] **Step 4: Update ProviderKind::supports_task**

Change the `supports_task` method (around line 67):

```rust
pub fn supports_task(&self, task: &str) -> bool {
    matches!(
        (self, task),
        (_, "chat")
            | (Self::OpenAi | Self::AzureOpenAi | Self::VertexAi, "image")
            | (Self::OpenAi | Self::AzureOpenAi | Self::Ollama | Self::VertexAi | Self::MicrosoftFoundry, "embedding")
    )
}
```

- [ ] **Step 5: Update ProviderKind::supported_capabilities**

```rust
pub fn supported_capabilities(&self) -> Vec<Capability> {
    let mut caps = vec![Capability::Chat];
    if self.supports_task("image") {
        caps.push(Capability::Image);
    }
    if self.supports_task("embedding") {
        caps.push(Capability::Embedding);
    }
    caps
}
```

- [ ] **Step 6: Update ALL_CAPABILITIES**

```rust
pub const ALL_CAPABILITIES: &[(&str, &str)] = &[
    ("chat", "Chat"),
    ("image", "Image Generation"),
    ("embedding", "Embedding"),
];
```

- [ ] **Step 7: Add EmbeddingMetadata and AiNode::embedding_metadata()**

Add after the `AiNode` impl block (after line 280):

```rust
/// Metadata about an embedding node, useful for configuring downstream systems.
#[derive(Debug, Clone)]
pub struct EmbeddingMetadata {
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub deployment: Option<String>,
    pub dimensions: Option<u32>,
}

impl EmbeddingMetadata {
    /// Generate Azure AI Search vectorizer configuration JSON.
    ///
    /// Only works for Azure OpenAI nodes — Azure AI Search vectorizers
    /// only support Azure OpenAI as a connected embedding source.
    pub fn to_azure_search_vectorizer(&self, name: &str) -> Result<serde_json::Value> {
        if self.provider != ProviderKind::AzureOpenAi {
            anyhow::bail!(
                "Azure AI Search vectorizers only support Azure OpenAI nodes, \
                 but this node uses '{}'. Configure an Azure OpenAI embedding node instead.",
                self.provider
            );
        }
        let endpoint = self.endpoint.as_deref().context(
            "Azure OpenAI embedding node has no endpoint configured.",
        )?;
        let deployment = self.deployment.as_deref().or(self.model.as_deref()).context(
            "Azure OpenAI embedding node has no deployment configured.",
        )?;
        let model_name = self.model.as_deref().unwrap_or(deployment);

        Ok(serde_json::json!({
            "name": name,
            "kind": "azureOpenAI",
            "azureOpenAIParameters": {
                "resourceUri": endpoint,
                "deploymentId": deployment,
                "modelName": model_name,
            }
        }))
    }
}
```

Add `embedding_metadata()` to the `impl AiNode` block:

```rust
/// Returns embedding metadata for this node.
pub fn embedding_metadata(&self) -> EmbeddingMetadata {
    let dimensions = self
        .node_defaults
        .as_ref()
        .and_then(|d| d.get("dimensions"))
        .and_then(|v| v.parse::<u32>().ok());

    EmbeddingMetadata {
        provider: self.provider.clone(),
        model: self.model.clone(),
        endpoint: self.endpoint.clone(),
        deployment: self.deployment.clone(),
        dimensions,
    }
}
```

- [ ] **Step 8: Run all tests to verify they pass**

Run: `cargo test --lib config::tests 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 9: Commit**

```bash
git add src/config.rs
git commit -m "feat: add Capability::Embedding, EmbeddingMetadata, and Azure Search vectorizer helper"
```

---

### Task 3: Add Provider::embed() trait method and Client convenience methods

**Files:**
- Modify: `src/client.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `src/client.rs`:

```rust
#[tokio::test]
async fn test_unsupported_embed() {
    let client = Client::builder()
        .ollama()
        .model("llama3.2")
        .build()
        .unwrap();
    // Ollama Provider impl doesn't have embed yet, so it should return Unsupported
    // (This test will need updating once Ollama embed is implemented in Task 5)
    let result = client.embed(&["hello"]).await;
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib client::tests::test_unsupported_embed 2>&1 | tail -5`
Expected: FAIL — `embed` method not found on `Client`

- [ ] **Step 3: Add Provider::embed() and Client methods**

In `src/client.rs`, update the imports at the top:

```rust
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageOptions,
    ImageResponse, Message, Task,
};
```

Add to the `Provider` trait (after `generate_image`):

```rust
/// Generate embeddings for the given texts.
async fn embed(
    &self,
    _texts: &[&str],
    _options: Option<&EmbedOptions>,
) -> Result<EmbedResponse> {
    Err(ClientError::Unsupported("embedding".to_string()).into())
}
```

Add to `impl Client` (after `generate_image_with`):

```rust
/// Generate embeddings for multiple texts.
pub async fn embed(&self, texts: &[&str]) -> Result<EmbedResponse> {
    self.provider.embed(texts, None).await
}

/// Generate embeddings with options.
pub async fn embed_with(
    &self,
    texts: &[&str],
    options: &EmbedOptions,
) -> Result<EmbedResponse> {
    self.provider.embed(texts, Some(options)).await
}

/// Embed a single text, returning the vector directly.
pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
    let response = self.provider.embed(&[text], None).await?;
    response
        .embeddings
        .into_iter()
        .next()
        .context("No embedding returned")
}
```

- [ ] **Step 4: Update lib.rs re-exports**

In `src/lib.rs`, update the re-exports:

```rust
pub use config::{AiNode, Auth, Capability, EmbeddingMetadata};
pub use types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageFormat, ImageOptions,
    ImageResponse, Message, Role, StreamEvent, Task, Usage,
};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib client::tests 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/client.rs src/lib.rs
git commit -m "feat: add Provider::embed() trait method and Client embed/embed_one convenience methods"
```

---

### Task 4: Implement OpenAI embedding (`openai.rs`)

**Files:**
- Modify: `src/openai.rs`

- [ ] **Step 1: Write the failing test**

Add to the bottom of `src/openai.rs` (create a `#[cfg(test)]` module if one doesn't exist):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_request_serialization() {
        let request = EmbedRequest {
            model: "text-embedding-3-small",
            input: &["hello world", "test"],
            dimensions: Some(1536),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "text-embedding-3-small");
        assert_eq!(json["input"], serde_json::json!(["hello world", "test"]));
        assert_eq!(json["dimensions"], 1536);
    }

    #[test]
    fn test_embed_request_no_dimensions() {
        let request = EmbedRequest {
            model: "text-embedding-3-small",
            input: &["hello"],
            dimensions: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("dimensions").is_none());
    }

    #[test]
    fn test_embed_response_parsing() {
        let json = r#"{
            "object": "list",
            "data": [
                {"object": "embedding", "index": 0, "embedding": [0.1, 0.2, 0.3]},
                {"object": "embedding", "index": 1, "embedding": [0.4, 0.5, 0.6]}
            ],
            "model": "text-embedding-3-small",
            "usage": {"prompt_tokens": 10, "total_tokens": 10}
        }"#;
        let response: EmbedApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.data.len(), 2);
        assert_eq!(response.data[0].embedding, vec![0.1, 0.2, 0.3]);
        assert_eq!(response.data[1].embedding, vec![0.4, 0.5, 0.6]);
        assert_eq!(response.model, "text-embedding-3-small");
        assert_eq!(response.usage.prompt_tokens, 10);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib openai::tests 2>&1 | tail -5`
Expected: FAIL — `EmbedRequest` / `EmbedApiResponse` not defined

- [ ] **Step 3: Add embedding types and Provider::embed implementation**

Add the import for `EmbedOptions` and `EmbedResponse` to the `use crate::types` block at the top of `src/openai.rs`:

```rust
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageFormat, ImageOptions,
    ImageResponse, Message, StreamEvent, Usage,
};
```

Add the request/response structs after the existing streaming types (after `StreamDelta`):

```rust
// Embedding types
#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
}

#[derive(Deserialize)]
struct EmbedApiResponse {
    data: Vec<EmbedData>,
    model: String,
    usage: EmbedApiUsage,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedApiUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}
```

Add the `embed` method to the `impl Provider for OpenAiClient` block (after `generate_image`):

```rust
async fn embed(
    &self,
    texts: &[&str],
    options: Option<&EmbedOptions>,
) -> Result<EmbedResponse> {
    let url = format!("{}/v1/embeddings", self.base_url());
    debug!(url = %url, model = %self.model, count = texts.len(), "Sending embedding request");

    let request = EmbedRequest {
        model: &self.model,
        input: texts,
        dimensions: options.and_then(|o| o.dimensions),
    };

    let response = self
        .client
        .post(&url)
        .bearer_auth(&self.api_key)
        .json(&request)
        .send()
        .await
        .context("Failed to send embedding request to OpenAI API")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ApiError>(&body)
            .map(|e| e.error.message)
            .unwrap_or(body);
        anyhow::bail!("OpenAI embedding error ({}): {}", status.as_u16(), message);
    }

    let api_response: EmbedApiResponse = response
        .json()
        .await
        .context("Failed to parse OpenAI embedding response")?;

    Ok(EmbedResponse {
        embeddings: api_response.data.into_iter().map(|d| d.embedding).collect(),
        model: api_response.model,
        usage: Some(Usage {
            prompt_tokens: api_response.usage.prompt_tokens,
            completion_tokens: 0,
            total_tokens: api_response.usage.total_tokens,
        }),
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib openai::tests 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/openai.rs
git commit -m "feat: implement OpenAI embedding support"
```

---

### Task 5: Implement Azure OpenAI embedding (`azure.rs`)

**Files:**
- Modify: `src/azure.rs`

- [ ] **Step 1: Write the failing test**

Add a `#[cfg(test)] mod tests` block at the bottom of `src/azure.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_response_parsing() {
        let json = r#"{
            "data": [
                {"embedding": [0.1, 0.2, 0.3], "index": 0}
            ],
            "model": "text-embedding-3-large",
            "usage": {"prompt_tokens": 5, "total_tokens": 5}
        }"#;
        let response: EmbedApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].embedding, vec![0.1, 0.2, 0.3]);
        assert_eq!(response.model, "text-embedding-3-large");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib azure::tests 2>&1 | tail -5`
Expected: FAIL — `EmbedApiResponse` not defined

- [ ] **Step 3: Add embedding types and implementation**

Add imports for embedding types at the top of `src/azure.rs`:

```rust
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageFormat, ImageOptions,
    ImageResponse, Message, StreamEvent, Usage,
};
```

Add embedding structs after the `ImageData` struct:

```rust
// Embedding types
#[derive(Serialize)]
struct EmbedRequest<'a> {
    input: &'a [&'a str],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
}

#[derive(Deserialize)]
struct EmbedApiResponse {
    data: Vec<EmbedData>,
    model: String,
    usage: EmbedApiUsage,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedApiUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}
```

Add an `embed_url` method to `impl AzureOpenAiClient`:

```rust
fn embed_url(&self) -> String {
    format!(
        "{}/openai/deployments/{}/embeddings?api-version={}",
        self.base_url(),
        self.deployment,
        self.api_version
    )
}
```

Add `embed` to `impl Provider for AzureOpenAiClient` (after `generate_image`):

```rust
async fn embed(
    &self,
    texts: &[&str],
    options: Option<&EmbedOptions>,
) -> Result<EmbedResponse> {
    let url = self.embed_url();
    debug!(url = %url, deployment = %self.deployment, count = texts.len(), "Sending embedding request to Azure OpenAI");

    let (header_name, header_value) = self.get_auth_header().await?;

    let request = EmbedRequest {
        input: texts,
        dimensions: options.and_then(|o| o.dimensions),
    };

    let response = self
        .client
        .post(&url)
        .header(header_name, &header_value)
        .json(&request)
        .send()
        .await
        .context("Failed to send embedding request to Azure OpenAI")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
    }

    let api_response: EmbedApiResponse = response
        .json()
        .await
        .context("Failed to parse Azure OpenAI embedding response")?;

    Ok(EmbedResponse {
        embeddings: api_response.data.into_iter().map(|d| d.embedding).collect(),
        model: api_response.model,
        usage: Some(Usage {
            prompt_tokens: api_response.usage.prompt_tokens,
            completion_tokens: 0,
            total_tokens: api_response.usage.total_tokens,
        }),
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib azure::tests 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/azure.rs
git commit -m "feat: implement Azure OpenAI embedding support"
```

---

### Task 6: Implement Ollama embedding (`ollama.rs`)

**Files:**
- Modify: `src/ollama.rs`

- [ ] **Step 1: Write the failing test**

Add a `#[cfg(test)] mod tests` block at the bottom of `src/ollama.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_response_parsing() {
        let json = r#"{
            "model": "nomic-embed-text",
            "embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]
        }"#;
        let response: EmbedApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.model, "nomic-embed-text");
        assert_eq!(response.embeddings.len(), 2);
        assert_eq!(response.embeddings[0], vec![0.1, 0.2, 0.3]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib ollama::tests 2>&1 | tail -5`
Expected: FAIL — `EmbedApiResponse` not defined

- [ ] **Step 3: Add embedding types and implementation**

Add `EmbedOptions` and `EmbedResponse` to the imports in `src/ollama.rs`:

```rust
use crate::types::{ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, Message, StreamEvent};
```

Add embedding structs after the `StreamChunk` struct:

```rust
// Embedding types
#[derive(Serialize)]
struct EmbedApiRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbedApiResponse {
    model: String,
    embeddings: Vec<Vec<f32>>,
}
```

Add `embed` to `impl Provider for OllamaClient` (after `chat_stream`):

```rust
async fn embed(
    &self,
    texts: &[&str],
    _options: Option<&EmbedOptions>,
) -> Result<EmbedResponse> {
    let url = format!("{}/api/embed", self.base_url());
    debug!(url = %url, model = %self.model, count = texts.len(), "Sending embedding request to Ollama");

    let request = EmbedApiRequest {
        model: &self.model,
        input: texts,
    };

    let response = self
        .client
        .post(&url)
        .json(&request)
        .send()
        .await
        .context("Failed to send embedding request to Ollama. Is Ollama running?")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Ollama embedding error ({}): {}", status.as_u16(), body);
    }

    let api_response: EmbedApiResponse = response
        .json()
        .await
        .context("Failed to parse Ollama embedding response")?;

    Ok(EmbedResponse {
        embeddings: api_response.embeddings,
        model: api_response.model,
        usage: None,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib ollama::tests 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/ollama.rs
git commit -m "feat: implement Ollama embedding support"
```

---

### Task 7: Implement Vertex AI embedding (`vertex.rs`)

**Files:**
- Modify: `src/vertex.rs`

- [ ] **Step 1: Write the failing test**

Add a `#[cfg(test)] mod tests` block at the bottom of `src/vertex.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_response_parsing() {
        let json = r#"{
            "predictions": [
                {"embeddings": {"values": [0.1, 0.2, 0.3]}},
                {"embeddings": {"values": [0.4, 0.5, 0.6]}}
            ]
        }"#;
        let response: EmbedPredictResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.predictions.len(), 2);
        assert_eq!(response.predictions[0].embeddings.values, vec![0.1, 0.2, 0.3]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib vertex::tests 2>&1 | tail -5`
Expected: FAIL — `EmbedPredictResponse` not defined

- [ ] **Step 3: Add embedding types and implementation**

Add `EmbedOptions` and `EmbedResponse` to the imports in `src/vertex.rs`:

```rust
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageFormat, ImageOptions,
    ImageResponse, Message, Role, StreamEvent, Usage,
};
```

Add embedding structs after the `ImagenPrediction` struct:

```rust
// Embedding types
#[derive(Serialize)]
struct EmbedPredictRequest {
    instances: Vec<EmbedInstance>,
}

#[derive(Serialize)]
struct EmbedInstance {
    content: String,
}

#[derive(Deserialize)]
struct EmbedPredictResponse {
    predictions: Vec<EmbedPrediction>,
}

#[derive(Deserialize)]
struct EmbedPrediction {
    embeddings: EmbedValues,
}

#[derive(Deserialize)]
struct EmbedValues {
    values: Vec<f32>,
}
```

Add `embed` to `impl Provider for VertexAiClient` (after `generate_image`):

```rust
async fn embed(
    &self,
    texts: &[&str],
    _options: Option<&EmbedOptions>,
) -> Result<EmbedResponse> {
    let url = format!("{}:predict", self.base_url());
    debug!(url = %url, model = %self.model, count = texts.len(), "Sending embedding request to Vertex AI");

    let token = Self::get_access_token().await?;

    let request = EmbedPredictRequest {
        instances: texts
            .iter()
            .map(|t| EmbedInstance {
                content: t.to_string(),
            })
            .collect(),
    };

    let response = self
        .client
        .post(&url)
        .bearer_auth(&token)
        .json(&request)
        .send()
        .await
        .context("Failed to send embedding request to Vertex AI")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ApiError>(&body)
            .ok()
            .and_then(|e| e.error.map(|d| d.message))
            .unwrap_or(body);
        anyhow::bail!("Vertex AI embedding error ({}): {}", status.as_u16(), message);
    }

    let api_response: EmbedPredictResponse = response
        .json()
        .await
        .context("Failed to parse Vertex AI embedding response")?;

    Ok(EmbedResponse {
        embeddings: api_response
            .predictions
            .into_iter()
            .map(|p| p.embeddings.values)
            .collect(),
        model: self.model.clone(),
        usage: None,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib vertex::tests 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/vertex.rs
git commit -m "feat: implement Vertex AI embedding support"
```

---

### Task 8: Implement Microsoft Foundry embedding (`foundry.rs`)

**Files:**
- Modify: `src/foundry.rs`

- [ ] **Step 1: Write the failing test**

Add a `#[cfg(test)] mod tests` block at the bottom of `src/foundry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_response_parsing() {
        let json = r#"{
            "data": [
                {"embedding": [0.1, 0.2, 0.3], "index": 0}
            ],
            "model": "text-embedding-3-large",
            "usage": {"prompt_tokens": 5, "total_tokens": 5}
        }"#;
        let response: EmbedApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].embedding, vec![0.1, 0.2, 0.3]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib foundry::tests 2>&1 | tail -5`
Expected: FAIL — `EmbedApiResponse` not defined

- [ ] **Step 3: Add embedding types and implementation**

Add `EmbedOptions` and `EmbedResponse` to the imports in `src/foundry.rs`:

```rust
use crate::types::{ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, Message, StreamEvent, Usage};
```

Add embedding structs after the `StreamDelta` struct:

```rust
// Embedding types
#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
}

#[derive(Deserialize)]
struct EmbedApiResponse {
    data: Vec<EmbedData>,
    model: String,
    usage: EmbedApiUsage,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedApiUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}
```

Add an `embed_url` method to `impl FoundryClient`:

```rust
fn embed_url(&self) -> String {
    format!(
        "{}/models/embeddings?api-version={}",
        self.base_url(),
        self.api_version
    )
}
```

Add `embed` to `impl Provider for FoundryClient` (after `chat_stream`):

```rust
async fn embed(
    &self,
    texts: &[&str],
    options: Option<&EmbedOptions>,
) -> Result<EmbedResponse> {
    let url = self.embed_url();
    debug!(url = %url, model = %self.model, count = texts.len(), "Sending embedding request to Microsoft Foundry");

    let (header_name, header_value) = self.get_auth_header().await?;

    let request = EmbedRequest {
        model: &self.model,
        input: texts,
        dimensions: options.and_then(|o| o.dimensions),
    };

    let response = self
        .client
        .post(&url)
        .header(header_name, &header_value)
        .json(&request)
        .send()
        .await
        .context("Failed to send embedding request to Microsoft Foundry")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
    }

    let api_response: EmbedApiResponse = response
        .json()
        .await
        .context("Failed to parse Microsoft Foundry embedding response")?;

    Ok(EmbedResponse {
        embeddings: api_response.data.into_iter().map(|d| d.embedding).collect(),
        model: api_response.model,
        usage: Some(Usage {
            prompt_tokens: api_response.usage.prompt_tokens,
            completion_tokens: 0,
            total_tokens: api_response.usage.total_tokens,
        }),
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib foundry::tests 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/foundry.rs
git commit -m "feat: implement Microsoft Foundry embedding support"
```

---

### Task 9: Add blocking client embedding methods (`blocking.rs`)

**Files:**
- Modify: `src/blocking.rs`

- [ ] **Step 1: Add blocking embedding methods**

Add `EmbedOptions` and `EmbedResponse` to the imports at the top of `src/blocking.rs`:

```rust
use crate::types::{
    ChatOptions, ChatResponse, EmbedOptions, EmbedResponse, ImageOptions, ImageResponse, Message,
    StreamEvent, Task,
};
```

Add to `impl Client` (after `generate_image_with`):

```rust
/// Generate embeddings for multiple texts.
pub fn embed(&self, texts: &[&str]) -> Result<EmbedResponse> {
    self.runtime.block_on(self.inner.embed(texts))
}

/// Generate embeddings with options.
pub fn embed_with(&self, texts: &[&str], options: &EmbedOptions) -> Result<EmbedResponse> {
    self.runtime
        .block_on(self.inner.embed_with(texts, options))
}

/// Embed a single text, returning the vector directly.
pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
    self.runtime.block_on(self.inner.embed_one(text))
}
```

- [ ] **Step 2: Run full build to verify it compiles**

Run: `cargo build --no-default-features --lib 2>&1 | tail -5`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add src/blocking.rs
git commit -m "feat: add blocking client embedding methods"
```

---

### Task 10: Add CLI embed command

**Files:**
- Create: `src/commands/embed.rs`
- Modify: `src/commands/mod.rs`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add EmbedArgs to cli.rs**

Add after the `ImageArgs` block in `src/cli.rs`:

```rust
// ---------------------------------------------------------------------------
// Embed args
// ---------------------------------------------------------------------------

#[derive(clap::Args)]
pub struct EmbedArgs {
    /// Text to embed
    pub text: Option<String>,

    /// Node to use for embedding (overrides default)
    #[arg(short, long)]
    pub node: Option<String>,

    /// Print the full vector as JSON
    #[arg(long)]
    pub full: bool,

    /// Show embedding node metadata
    #[arg(long)]
    pub info: bool,

    /// Print Azure AI Search vectorizer JSON for the embedding node
    #[arg(long, value_name = "NAME")]
    pub azure_vectorizer: Option<String>,
}
```

Add `Embed` variant to the `Commands` enum (after `Image`):

```rust
/// Generate embeddings from text
Embed(EmbedArgs),
```

Add `"embed"` to `KNOWN_SUBCOMMANDS`:

```rust
pub const KNOWN_SUBCOMMANDS: &[&str] = &[
    "chat",
    "image",
    "embed",
    "ai",
    "completion",
    "version",
    "help",
    // Hidden backward-compat aliases:
    "config",
    "nodes",
    "discover",
];
```

- [ ] **Step 2: Create commands/embed.rs**

Create `src/commands/embed.rs`:

```rust
use anyhow::{Context, Result};
use colored::Colorize;

use ailloy::client::create_provider_from_node;
use ailloy::config::Config;

use crate::cli::EmbedArgs;

pub async fn run(args: EmbedArgs, quiet: bool) -> Result<()> {
    let config = Config::load()?;

    // --info: show embedding node metadata
    if args.info {
        return run_info(&args, &config);
    }

    // --azure-vectorizer: output vectorizer JSON
    if let Some(ref name) = args.azure_vectorizer {
        return run_azure_vectorizer(&args, &config, name);
    }

    // Embed text
    let text = args.text.as_deref().context(
        "No text provided. Usage: ailloy embed \"text to embed\"\n\
         Or use --info to show node metadata, --azure-vectorizer NAME for vectorizer config.",
    )?;

    let node_id = resolve_embed_node(args.node.as_deref(), &config)?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    if !quiet {
        eprintln!(
            "{} {} (embedding)",
            "Using:".dimmed(),
            provider.name().dimmed()
        );
    }

    let response = provider.embed(&[text], None).await?;

    let vector = response
        .embeddings
        .first()
        .context("No embedding returned")?;

    if args.full {
        // Print full vector as JSON
        println!("{}", serde_json::to_string(vector)?);
    } else {
        // Print summary
        println!("{} {}", "Model:".bold(), response.model);
        println!("{} {}", "Dimensions:".bold(), vector.len());
        if let Some(usage) = &response.usage {
            println!("{} {}", "Tokens:".bold(), usage.prompt_tokens);
        }
        // Preview: first 5 values
        let preview: Vec<String> = vector.iter().take(5).map(|v| format!("{:.6}", v)).collect();
        let suffix = if vector.len() > 5 {
            format!(", ... ({} more)", vector.len() - 5)
        } else {
            String::new()
        };
        println!("{} [{}{}]", "Vector:".bold(), preview.join(", "), suffix);
    }

    Ok(())
}

fn run_info(args: &EmbedArgs, config: &Config) -> Result<()> {
    let node_id = resolve_embed_node(args.node.as_deref(), config)?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let meta = node.embedding_metadata();

    println!("{} {}", "Node:".bold(), node_id);
    println!("{} {}", "Provider:".bold(), meta.provider);
    if let Some(model) = &meta.model {
        println!("{} {}", "Model:".bold(), model);
    }
    if let Some(endpoint) = &meta.endpoint {
        println!("{} {}", "Endpoint:".bold(), endpoint);
    }
    if let Some(deployment) = &meta.deployment {
        println!("{} {}", "Deployment:".bold(), deployment);
    }
    if let Some(dimensions) = meta.dimensions {
        println!("{} {}", "Dimensions:".bold(), dimensions);
    }

    Ok(())
}

fn run_azure_vectorizer(args: &EmbedArgs, config: &Config, name: &str) -> Result<()> {
    let node_id = resolve_embed_node(args.node.as_deref(), config)?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let meta = node.embedding_metadata();
    let vectorizer = meta.to_azure_search_vectorizer(name)?;
    println!("{}", serde_json::to_string_pretty(&vectorizer)?);
    Ok(())
}

fn resolve_embed_node(node_arg: Option<&str>, config: &Config) -> Result<String> {
    if let Some(node_id) = node_arg {
        let (resolved_id, _) = config.get_node(node_id).with_context(|| {
            format!(
                "Embedding node '{}' not found. Run `ailloy ai config` to add it.",
                node_id
            )
        })?;
        return Ok(resolved_id.to_string());
    }

    let (id, _) = config.default_node_for("embedding")?;
    Ok(id.to_string())
}
```

- [ ] **Step 3: Update commands/mod.rs**

Add the embed module:

```rust
pub mod ai;
pub mod chat;
pub mod completion;
pub mod config_cmd;
pub mod embed;
pub mod image;
pub mod skill;
pub(crate) mod util;
```

- [ ] **Step 4: Update main.rs**

Add the `Embed` match arm in the `match cli.command` block (after `Commands::Image`):

```rust
Commands::Embed(args) => commands::embed::run(args, quiet).await,
```

Also update the `is_raw` check to not include embed (embed has no `--raw` flag).

- [ ] **Step 5: Run full build to verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles successfully

- [ ] **Step 6: Commit**

```bash
git add src/commands/embed.rs src/commands/mod.rs src/cli.rs src/main.rs
git commit -m "feat: add 'ailloy embed' CLI command with --info and --azure-vectorizer"
```

---

### Task 11: Remove stale test and run full CI checks

**Files:**
- Modify: `src/client.rs` (update the `test_unsupported_embed` test now that Ollama has embed)

- [ ] **Step 1: Update the unsupported embed test**

The test from Task 3 used Ollama which now supports embed. Change it to use `LocalAgent` which doesn't support embedding:

```rust
#[tokio::test]
async fn test_unsupported_embed() {
    let client = Client::builder()
        .local_agent()
        .binary("echo")
        .build()
        .unwrap();
    let result = client.embed(&["hello"]).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("embedding"), "Error should mention embedding: {}", err);
}
```

- [ ] **Step 2: Run full CI check**

Run: `cargo fmt --all -- --check && cargo clippy -- -D warnings && cargo test`
Expected: All pass with no warnings

- [ ] **Step 3: Fix any issues found**

If clippy or tests fail, fix the issues.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: fix unsupported embed test and pass full CI checks"
```

---

### Task 12: Update documentation

**Files:**
- Modify: `CLAUDE.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Update CLAUDE.md architecture section**

In the `Architecture` section file listing, add after the `commands/image.rs` entry:

```
    image.rs          # `ailloy image` — image generation, direct and interactive modes
    embed.rs          # `ailloy embed` — embedding generation, metadata, Azure vectorizer export
```

Update the `config.rs` description to mention `EmbeddingMetadata`:

```
  config.rs           # Config types (AiNode, Capability, Auth, ProviderKind, Config,
                      #   EmbeddingMetadata), load/save, local config merge, node CRUD,
                      #   alias resolution, capability filtering, ALL_CAPABILITIES constant,
                      #   Azure AI Search vectorizer export
```

In the `Key Patterns` section, update the **Provider trait** bullet to mention `embed()`:

```
- **Provider trait** (`client.rs`): unified `async_trait` with default methods returning `Unsupported` — `name()`, `chat()`, `chat_stream()`, `generate_image()`, `embed()`
```

- [ ] **Step 2: Update CHANGELOG.md**

Add an `## [Unreleased]` entry (or update existing) at the top:

```markdown
## [Unreleased]

### Added
- Embedding support as a first-class capability across OpenAI, Azure OpenAI, Ollama, Vertex AI, and Microsoft Foundry
- `Client::embed()`, `Client::embed_with()`, and `Client::embed_one()` convenience methods (async and blocking)
- `EmbeddingMetadata` struct for querying embedding node configuration
- `EmbeddingMetadata::to_azure_search_vectorizer()` for generating Azure AI Search vectorizer JSON
- `Capability::Embedding` and `Task::Embedding` for config routing
- `ailloy embed` CLI command with `--info`, `--full`, and `--azure-vectorizer` flags
- `defaults.embedding` config key for setting the default embedding node
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md CHANGELOG.md
git commit -m "docs: update CLAUDE.md and CHANGELOG.md for embedding support"
```
