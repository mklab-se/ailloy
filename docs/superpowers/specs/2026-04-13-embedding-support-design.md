# Embedding Support for Ailloy

**Date:** 2026-04-13
**Status:** Approved

## Summary

Add embedding as a first-class capability in ailloy: a Provider trait method, response types, config integration (defaults, capability tagging), metadata export for downstream systems (Azure AI Search vectorizer config), and a CLI command for testing.

## Motivation

Tools built on ailloy need to generate embeddings for populating vector search indexes (Azure AI Search, pgvector, Qdrant, etc.) and configure those indexes to use the same embedding model. Today ailloy has no embedding support, forcing consumers to integrate directly with provider SDKs and duplicate connection details.

## Providers

Embedding support for all providers that offer it:

| Provider | Endpoint | Notes |
|---|---|---|
| OpenAI | `POST /v1/embeddings` | `text-embedding-3-small`, `text-embedding-3-large` |
| Azure OpenAI | `POST {endpoint}/openai/deployments/{deployment}/embeddings` | Same models via Azure deployments |
| Ollama | `POST /api/embed` | `nomic-embed-text`, `all-minilm`, etc. |
| Vertex AI | `POST :predict` on embedding model | `text-embedding-004`, etc. |
| Microsoft Foundry | OpenAI-compatible `/embeddings` | Via Foundry endpoint |

Anthropic and LocalAgent do not support embeddings.

## Types (`types.rs`)

### EmbedResponse

```rust
pub struct EmbedResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub model: String,
    pub usage: Option<Usage>,
}
```

`Vec<f32>` is the universal embedding currency — serializes directly to JSON arrays for Azure AI Search, converts trivially to `pgvector::Vector`, and matches every vector DB's expected format.

### EmbedOptions

```rust
pub struct EmbedOptions {
    pub dimensions: Option<u32>,
}
```

Builder pattern matching `ChatOptions` and `ImageOptions`.

### Task enum

Add `Embedding` variant with config key `"embedding"`.

## Capability (`config.rs`)

### Capability enum

Add `Capability::Embedding`.

### ALL_CAPABILITIES

Add `("embedding", "Embedding")` to the ordered list.

### ProviderKind::supports_task

Update the match to return true for `"embedding"` on: `OpenAi`, `AzureOpenAi`, `Ollama`, `VertexAi`, `MicrosoftFoundry`.

### ProviderKind::supported_capabilities

Add `Capability::Embedding` to the returned vec for providers that support it.

## Provider Trait (`client.rs`)

Add default method:

```rust
async fn embed(
    &self,
    _texts: &[&str],
    _options: Option<&EmbedOptions>,
) -> Result<EmbedResponse> {
    Err(ClientError::Unsupported("embedding".to_string()).into())
}
```

## Client (`client.rs`)

### Convenience methods

```rust
/// Embed multiple texts.
pub async fn embed(&self, texts: &[&str]) -> Result<EmbedResponse>;

/// Embed multiple texts with options.
pub async fn embed_with(&self, texts: &[&str], options: &EmbedOptions) -> Result<EmbedResponse>;

/// Embed a single text, returning the vector directly.
pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>>;
```

## Provider Implementations

### OpenAI (`openai.rs`)

```
POST {endpoint}/v1/embeddings
{
  "model": "text-embedding-3-small",
  "input": ["text1", "text2"],
  "dimensions": 1536  // optional
}
```

Response: `{ "data": [{ "embedding": [...] }], "model": "...", "usage": {...} }`

### Azure OpenAI (`azure.rs`)

```
POST {endpoint}/openai/deployments/{deployment}/embeddings?api-version={version}
```

Same request/response body as OpenAI. Auth via API key or Azure CLI token (reuse existing `resolve_azure_auth`).

### Ollama (`ollama.rs`)

```
POST {endpoint}/api/embed
{
  "model": "nomic-embed-text",
  "input": ["text1", "text2"]
}
```

Response: `{ "model": "...", "embeddings": [[...], [...]] }`

### Vertex AI (`vertex.rs`)

```
POST https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:predict
{
  "instances": [{ "content": "text1" }, { "content": "text2" }]
}
```

Response: `{ "predictions": [{ "embeddings": { "values": [...] } }] }`

### Microsoft Foundry (`foundry.rs`)

OpenAI-compatible embeddings endpoint via Foundry. Same request format as OpenAI, auth via Azure CLI or API key.

## Embedding Metadata (`config.rs`)

```rust
pub struct EmbeddingMetadata {
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub deployment: Option<String>,
    pub dimensions: Option<u32>,
}
```

Queryable from `AiNode`:

```rust
impl AiNode {
    pub fn embedding_metadata(&self) -> EmbeddingMetadata;
}
```

Returns the node's connection details relevant to embedding. The `dimensions` field comes from the node's `node_defaults` (a new optional field for embedding-specific defaults) or is `None` if not configured.

## Azure AI Search Vectorizer Helper

Method on `EmbeddingMetadata`:

```rust
impl EmbeddingMetadata {
    /// Generate Azure AI Search vectorizer configuration.
    /// Only works for Azure OpenAI nodes.
    pub fn to_azure_search_vectorizer(
        &self,
        name: &str,
    ) -> Result<serde_json::Value>;
}
```

Output for an Azure OpenAI node:

```json
{
  "name": "my-vectorizer",
  "kind": "azureOpenAI",
  "azureOpenAIParameters": {
    "resourceUri": "https://myresource.openai.azure.com",
    "deploymentId": "text-embedding-3-large",
    "modelName": "text-embedding-3-large"
  }
}
```

Returns an error for non-Azure providers since Azure AI Search vectorizers only support Azure OpenAI as a connected embedding source.

## Blocking Client (`blocking.rs`)

Mirror all async embedding methods:

```rust
pub fn embed(&self, texts: &[&str]) -> Result<EmbedResponse>;
pub fn embed_with(&self, texts: &[&str], options: &EmbedOptions) -> Result<EmbedResponse>;
pub fn embed_one(&self, text: &str) -> Result<Vec<f32>>;
```

## CLI (`commands/embed.rs`)

New `ailloy embed` subcommand:

| Usage | Description |
|---|---|
| `ailloy embed "some text"` | Embed text, print model name and dimensions |
| `ailloy embed "text" --full` | Print the full vector as JSON |
| `ailloy embed --info` | Print embedding node metadata |
| `ailloy embed --azure-vectorizer NAME` | Print Azure AI Search vectorizer JSON |

Supports `--node` / `-n` to select a specific embedding node (defaults to `defaults.embedding`).

## Library Re-exports (`lib.rs`)

Add to root re-exports: `EmbedResponse`, `EmbedOptions`, `EmbeddingMetadata`.

## Config Example

```yaml
nodes:
  azure-openai/text-embedding-3-large:
    provider: azure-openai
    deployment: text-embedding-3-large
    endpoint: https://myresource.openai.azure.com
    auth:
      azure-cli: {}
    capabilities:
      - embedding
    node_defaults:
      dimensions: 3072

  ollama/nomic-embed-text:
    provider: ollama
    model: nomic-embed-text
    capabilities:
      - embedding

defaults:
  chat: openai/gpt-4o
  image: openai/dall-e-3
  embedding: azure-openai/text-embedding-3-large
```

## Testing

- Unit tests for each provider's embedding request/response parsing (mock HTTP responses)
- Unit tests for `EmbeddingMetadata` construction and `to_azure_search_vectorizer()` output
- Unit tests for `Task::Embedding` config key, capability routing
- Integration test for `Client::for_capability("embedding")` with config
- CLI tests for `ailloy embed --info` and `--azure-vectorizer`

## Out of Scope

- Vectorizer export for non-Azure systems (Qdrant config, pgvector schema) — can be added later
- Batch size limits / automatic chunking — consumers handle this
- Embedding model discovery (listing available models from providers)
