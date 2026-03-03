# Ailloy API Vision

This document describes the target API for ailloy — both the CLI interface and the Rust library API. It serves as a design validation tool: if we can read through these examples and they feel right, we're building the right thing.

---

## Part 1: CLI Use Cases

### The default command

The most common thing you do with ailloy is ask a question. It should be as simple as possible — no subcommand required.

```bash
# These are equivalent:
ailloy "What is the capital of France?"
ailloy chat "What is the capital of France?"
```

When the first argument isn't a recognized subcommand, ailloy treats it as a message. The `chat` subcommand exists for explicitness and for flags, but the bare form is the default.

Implementation: before clap parses, peek at `argv[1]`. If it's not a known subcommand and doesn't start with `-`, insert `chat` into the args. This is predictable and gives full clap validation for the chat command either way.

### Piping and stdin

When stdin is a pipe (not a terminal), ailloy automatically reads it as context. No `--stdin` flag needed.

```bash
# Stdin is auto-detected as piped — its content becomes context
git diff | ailloy "Review this diff"
cat error.log | ailloy "What's going wrong here?"
curl -s https://api.example.com/data | ailloy "Summarize this JSON"

# No argument with piped input — stdin content is sent as-is
cat report.txt | ailloy

# Works in scripts
SUMMARY=$(cat report.txt | ailloy -q "Summarize in one paragraph")
```

How stdin + message combine:

| stdin | argument | behavior |
| --- | --- | --- |
| piped | present | `"{argument}\n\n{stdin}"` — argument is instruction, stdin is context |
| piped | absent | stdin content sent as the full message |
| not piped | present | argument is the full message |
| not piped | absent + `-i` | enter interactive mode |
| not piped | absent | show help |

### Flags and options

```bash
# Specific provider (override default)
ailloy "Explain Rust lifetimes" --provider ollama

# Control generation
ailloy "Write a haiku" --temperature 1.2 --max-tokens 50

# Stream tokens to the terminal as they're generated
ailloy "Tell me a story" --stream

# Quiet mode — only output the response, no status on stderr
ailloy -q "Give me a JSON object" > output.json

# Combine everything
git diff | ailloy "Review for security issues" --provider openai --stream --temperature 0.3
```

### System prompts

The `--system` flag sets the AI's persona or instructions for a request.

```bash
# Code review with a specific persona
git diff | ailloy "Review this" --system "You are a senior Rust developer. Focus on safety."

# Translation helper
ailloy "The weather is nice today" --system "Translate to Swedish"

# Structured output
ailloy "List the top 5 programming languages" --system "Respond only in valid JSON"
```

When is `--system` useful vs just putting it in the message? System prompts sit in a privileged position in the API — models treat them as persistent instructions rather than conversational input. This matters for:

- Keeping the instruction separate from user content (important for safety)
- Interactive mode, where the system prompt persists across turns but user messages change
- Library users who set a system prompt once for a conversation

For quick one-off questions, you often don't need it. `ailloy "Translate to Swedish: The weather is nice"` works fine too.

### Interactive conversations

```bash
# Start an interactive session
ailloy -i

# Interactive with streaming (the natural way to chat)
ailloy -i --stream

# Start with a persona
ailloy -i --system "You are a Rust tutor. Explain with examples."

# Start with an opening message, then continue interactively
ailloy -i "Let's design a REST API"
```

Inside a session:

```text
ailloy v0.2.0 — azure-gpt5 (gpt-5.3)
Type /help for commands, /quit to exit.

> What is a monad?
A monad is a design pattern from functional programming...

> Can you give me a Rust example?
Here's how you might think of Option as a monad in Rust...

> /clear
History cleared.

> /provider gemini-pro
Switched to gemini-pro (gemini-2.5-pro)

> Now explain it more simply
A monad is basically a wrapper that lets you chain operations...

> /quit
```

### Saving output to file

The `--output` / `-o` flag saves the response to a file instead of printing to the terminal.

```bash
# Save chat response to a file
ailloy "Write a poem about the sea" -o poem.txt
ailloy "Generate a JSON schema for a user" -o user.schema.json
git diff | ailloy "Write a detailed review" -o review.md
```

### Image generation

All image output — raster or vector — uses the same `-o` flag. The user thinks "I want an image", and ailloy handles the rest.

```bash
# Raster images → image generation provider (Nano Banana, Imagen, DALL-E)
ailloy "A cat wearing a top hat, watercolor style" -o cat.png
ailloy "A sunset over mountains" -o sunset.jpg --size 1024x1024 --quality hd
ailloy "Product photo of a watch" --provider vertex-imagen -o watch.png

# SVG → chat provider, ailloy auto-generates the right system prompt
ailloy "Minimalist blue tech logo" -o logo.svg

# The user experience is the same — just pick an extension
ailloy "Corporate logo, clean lines" -o logo.png    # raster via image gen
ailloy "Corporate logo, clean lines" -o logo.svg    # vector via chat
```

Output routing logic for `--output`:

| Extension | Route | How it works |
| --- | --- | --- |
| `.png`, `.jpg`, `.webp` | Image generation | Uses `defaults.image` provider, saves binary |
| `.svg` | Chat (transparent) | Uses `defaults.chat` provider with auto-injected system prompt for valid SVG output |
| `.txt`, `.md`, `.json`, etc. | Chat | Uses `defaults.chat` provider, saves text response |
| No `--output` | Chat | Prints text to stdout (default) |

For SVG, ailloy auto-injects a system prompt like "Generate valid SVG markup. Output only the raw SVG code with no explanation or markdown." The user never has to think about SVG being text — it just works like any other image format.

Ailloy only supports raster formats the image provider supports. If you request `.webp` and the provider only supports PNG, you get a clear error listing supported formats.

### Embeddings

```bash
# Generate embeddings (output as JSON)
ailloy embed "Some text to embed"
ailloy embed "Some text to embed" --output embedding.json

# Embed from file
cat document.txt | ailloy embed --output vectors.json
```

### Transcription (future)

```bash
# Transcribe audio
ailloy transcribe recording.mp3
ailloy transcribe recording.mp3 --output transcript.txt
```

### Setup and configuration

```bash
# Interactive wizard — detects available providers first
ailloy config init

# Per-project config (writes .ailloy.yaml in current directory)
ailloy config init --local

# Show effective config (global + local merged)
ailloy config show
```

### Provider management

```bash
# List configured providers (shows defaults and task types)
ailloy providers list

# Auto-detect what's available on this machine
ailloy providers detect
```

Example `providers list` output:

```text
Configured Providers

  azure-gpt5     azure-openai  gpt-5.3              <- default (chat)
  openai-dalle   openai        dall-e-3              <- default (image)
  claude         local-agent   claude binary
  gemini-pro     openai        gemini-2.5-pro
  azure-dev      azure-openai  gpt4-dev
  openai-fast    openai        gpt-4o-mini
```

Example `providers detect` output:

```text
Detected Providers

  ✓ openai       OPENAI_API_KEY is set
  ✓ ollama       Running at localhost:11434 (3 models available)
  ✓ claude       claude found in PATH
  ✓ azure        az CLI authenticated (2 subscriptions)
  ✗ codex        codex not found

Run 'ailloy config init' to configure detected providers.
```

### Diagnostics

```bash
# Health check all configured providers
ailloy doctor

# AI-assisted troubleshooting
ailloy ask "My Ollama connection keeps timing out"
```

---

## Part 2: Configuration

The config file lives at `~/.config/ailloy/config.yaml` (global) with optional per-project overrides via `.ailloy.yaml` in the project root.

### Config structure

```yaml
# Task-level defaults — which provider to use for each task type
defaults:
  chat: azure-gpt5
  image: vertex-nanobanana
  # embedding: openai-embed
  # transcription: whisper

# Named provider configurations — each fully self-contained
providers:
  # --- Chat providers ---

  azure-gpt5:
    kind: azure-openai
    endpoint: https://my-foundry.openai.azure.com
    deployment: gpt-53-deployment
    api_version: 2025-01-01
    auth: azure-cli

  anthropic-sonnet:
    kind: anthropic
    api_key: sk-ant-abc123
    model: claude-sonnet-4-6

  vertex-gemini:
    kind: vertex-ai
    project: my-gcp-project
    location: us-central1
    model: gemini-3.1-pro
    auth: gcloud-cli

  openai:
    kind: openai
    api_key: sk-proj-abc123
    model: gpt-5.2

  openai-fast:
    kind: openai
    api_key: sk-proj-abc123
    model: gpt-4o-mini

  ollama:
    kind: ollama
    model: qwen3.5:latest

  claude-local:
    kind: local-agent
    binary: claude

  # --- Image generation providers ---

  vertex-nanobanana:
    kind: vertex-ai
    project: my-gcp-project
    location: us-central1
    model: gemini-3.1-flash-image    # Nano Banana 2
    task: image-generation
    auth: gcloud-cli

  vertex-imagen:
    kind: vertex-ai
    project: my-gcp-project
    location: us-central1
    model: imagen-4.0-generate-001
    task: image-generation
    auth: gcloud-cli

  openai-dalle:
    kind: openai
    api_key: sk-proj-abc123
    model: dall-e-3
    task: image-generation
    defaults:
      size: 1024x1024
      quality: hd
```

### Design principles

- **Each provider entry is fully self-contained.** No inheritance, no shared credentials sections, no magic. You can read any single entry and understand exactly what it does.
- **Duplication is acceptable.** Two providers using the same OpenAI API key both carry it. Config files are written once and read many times — clarity beats DRY.
- **`defaults` maps task types to provider names.** `defaults.chat` is required (error with helpful message if missing). Other task defaults are optional — if not configured, those tasks require `--provider`.
- **`task` defaults to `chat` when omitted.** Most providers are chat providers. Only non-chat providers (image generation, embeddings, etc.) need to specify their task explicitly.
- **Env var fallback.** A `kind: openai` entry without `api_key` falls back to `OPENAI_API_KEY`. A `kind: azure-openai` entry with `auth: azure-cli` shells out to `az account get-access-token`. This covers the common simple cases.

### Multiple providers, same vendor

This config supports real-world complexity:

- Two different OpenAI API keys (`openai-dalle` and `openai-fast`)
- Two different Azure subscriptions/deployments (`azure-gpt5` and `azure-dev`)
- An OpenAI-compatible endpoint that isn't OpenAI (`gemini-pro` through Google's OpenAI-compatible API)
- A local agent (`claude`)

Each is named, each stands alone, and the user picks which to use by name or lets the defaults route automatically.

### Local (per-project) config

`.ailloy.yaml` in the project root overrides the global config. The merge is simple: local `defaults` entries override global ones, local `providers` entries are added (or override by name).

```yaml
# .ailloy.yaml — project-level override
defaults:
  chat: claude    # this project uses claude instead of azure-gpt5
```

---

## Part 3: Library API

### Design principles

1. **Simple things should be simple.** A one-shot chat should be 3-5 lines.
2. **Complex things should be possible.** Custom providers, history stores, streaming with backpressure.
3. **No CLI dependencies in the library.** Zero bloat from clap, colored, inquire.
4. **Trait-based extensibility.** Providers and history stores are traits. Users implement their own.
5. **Builder pattern for ergonomics.** No 8-parameter function signatures.
6. **Async-first, sync-friendly.** The primary API is async. A `blocking` feature provides sync wrappers.

### Quick start — one-shot chat

```rust
use ailloy::{Client, Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;  // uses defaults.chat
    let response = client.chat(&[Message::user("Hello!")]).await?;
    println!("{}", response.content);
    Ok(())
}
```

### Sync usage (blocking feature)

For applications that don't use an async runtime. Uses its own internal tokio runtime, modeled after `reqwest::blocking`.

```rust
// Cargo.toml: ailloy = { version = "0.2", default-features = false, features = ["blocking"] }
use ailloy::blocking::Client;
use ailloy::Message;

fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let response = client.chat(&[Message::user("Hello!")])?;
    println!("{}", response.content);
    Ok(())
}
```

Streaming in blocking mode returns an iterator:

```rust
use ailloy::blocking::Client;
use ailloy::{Message, StreamEvent};

let client = Client::from_config()?;
for event in client.chat_stream(&[Message::user("Tell me a story")])? {
    match event? {
        StreamEvent::Delta(text) => print!("{text}"),
        StreamEvent::Done(_) => println!(),
    }
}
```

### Configuring the client

```rust
use ailloy::{Client, Task};

// From config — uses defaults.chat provider
let client = Client::from_config()?;

// From config — specific named provider
let client = Client::from_config()?.with_provider("anthropic-sonnet")?;

// From config — default provider for a task type
let client = Client::from_config()?.for_task(Task::ImageGeneration)?;

// Construct programmatically — no config file needed
let client = Client::openai("sk-...", "gpt-5.2")?;
let client = Client::anthropic("sk-ant-...", "claude-sonnet-4-6")?;
let client = Client::ollama("qwen3.5:latest", None)?;
let client = Client::azure(
    "https://myinstance.openai.azure.com",
    "my-deployment",
    "2025-01-01",
)?;
let client = Client::vertex("my-project", "us-central1", "gemini-3.1-pro")?;

// Full builder
let client = Client::builder()
    .openai()
    .api_key("sk-...")
    .model("gpt-4o-mini")
    .endpoint("https://my-proxy.example.com")
    .build()?;
```

### Chat with options

```rust
use ailloy::{Client, ChatOptions, Message};

let client = Client::from_config()?;

// Simple — no options
let response = client.chat(&[Message::user("Hello")]).await?;

// With options
let options = ChatOptions::builder()
    .temperature(0.7)
    .max_tokens(500)
    .build();

let response = client
    .chat_with(&[Message::user("Write a poem")], &options)
    .await?;
```

### Streaming

```rust
use ailloy::{Client, Message, StreamEvent};
use futures_util::StreamExt;

let client = Client::from_config()?;

let mut stream = client
    .chat_stream(&[Message::user("Tell me a story")])
    .await?;

while let Some(event) = stream.next().await {
    match event? {
        StreamEvent::Delta(text) => print!("{text}"),
        StreamEvent::Done(response) => {
            println!();
            if let Some(usage) = response.usage {
                eprintln!("Tokens: {}", usage.total_tokens);
            }
        }
    }
}
```

### Conversations

Managing multi-turn history correctly is tedious. Ailloy handles it.

```rust
use ailloy::{Client, Conversation};

let client = Client::from_config()?;
let mut conv = Conversation::new(client);

// Set a system prompt (persists across turns)
conv.system("You are a helpful coding assistant");

// Each send() appends user message + AI response to history
let r1 = conv.send("What is Rust's ownership model?").await?;
println!("{}", r1.content);

let r2 = conv.send("How does that relate to lifetimes?").await?;
println!("{}", r2.content);
// ^ this request included the full conversation history

// Streaming works too — response auto-appended to history when stream completes
let mut stream = conv.send_stream("Give me an example").await?;
while let Some(event) = stream.next().await {
    // ...
}

// Inspect or reset
println!("{} messages in history", conv.history().len());
conv.clear();
```

A single question and a multi-turn conversation use the same `Client`. The difference is whether you use `client.chat()` directly or wrap it in a `Conversation` that manages state.

### Pluggable history store

By default, history is `Vec<Message>` in memory. Library users can swap it.

```rust
use ailloy::{ChatHistory, Message, Role};

/// Trait for conversation history storage.
pub trait ChatHistory: Send + Sync {
    /// Messages to send to the provider (may filter/window).
    fn messages(&self) -> Vec<Message>;

    /// Append a message.
    fn push(&mut self, message: Message);

    /// Clear all messages.
    fn clear(&mut self);

    /// Total number of stored messages.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
```

The key design choice: `messages()` returns `Vec<Message>`, not `&[Message]`. This lets implementations transform what gets sent — for example, a sliding window:

```rust
struct SlidingWindowHistory {
    messages: Vec<Message>,
    window: usize,
}

impl ChatHistory for SlidingWindowHistory {
    fn messages(&self) -> Vec<Message> {
        if self.messages.len() <= self.window {
            return self.messages.clone();
        }
        let mut result = vec![];
        // Always keep the system prompt
        if let Some(first) = self.messages.first() {
            if first.role == Role::System {
                result.push(first.clone());
            }
        }
        let start = self.messages.len() - self.window;
        result.extend(self.messages[start..].iter().cloned());
        result
    }

    fn push(&mut self, msg: Message) { self.messages.push(msg); }
    fn clear(&mut self) { self.messages.clear(); }
    fn len(&self) -> usize { self.messages.len() }
}

// Use it
let client = Client::from_config()?;
let history = SlidingWindowHistory { messages: vec![], window: 20 };
let mut conv = Conversation::with_history(client, history);
```

Other implementations a library user might build:

- **Redis-backed** — persist conversations across process restarts
- **Token-counting** — window based on token budget, not message count
- **Summarizing** — compress old messages using the AI itself
- **Database-backed** — store conversations for audit/analytics

Ailloy doesn't implement these, but the trait makes them straightforward.

### Image generation

```rust
use ailloy::{Client, ImageOptions, Task};

// Use the default image provider
let client = Client::from_config()?.for_task(Task::ImageGeneration)?;

// Simple
let image = client.generate_image("A cat in a top hat").await?;
std::fs::write("cat.png", &image.data)?;

// With options
let options = ImageOptions::builder()
    .size(1024, 1024)
    .quality("hd")
    .build();

let image = client
    .generate_image_with("A sunset over mountains", &options)
    .await?;
```

### Embeddings

```rust
use ailloy::{Client, Task};

let client = Client::from_config()?.for_task(Task::Embedding)?;

// Single text
let embedding = client.embed("Hello world").await?;
println!("Vector of {} dimensions", embedding.vector.len());

// Batch
let embeddings = client
    .embed_batch(&["Hello", "World", "Foo"])
    .await?;
```

### Custom providers

Library users can implement their own providers. The `Provider` trait has default methods that return `Unsupported` — implement only the capabilities your provider supports.

```rust
use ailloy::provider::Provider;
use ailloy::{Message, ChatResponse, ChatOptions, ChatStream, ClientError};

/// Unified provider trait. Override methods for the capabilities you support.
/// Methods you don't override return ClientError::Unsupported.
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;

    async fn chat(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        Err(ClientError::Unsupported("chat").into())
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatStream> {
        Err(ClientError::Unsupported("streaming").into())
    }

    async fn generate_image(
        &self,
        prompt: &str,
        options: Option<&ImageOptions>,
    ) -> Result<ImageResponse> {
        Err(ClientError::Unsupported("image generation").into())
    }

    async fn embed(
        &self,
        input: &str,
    ) -> Result<EmbeddingResponse> {
        Err(ClientError::Unsupported("embeddings").into())
    }
}
```

Example custom provider:

```rust
struct MyProvider { /* ... */ }

impl Provider for MyProvider {
    fn name(&self) -> &str { "my-provider" }

    async fn chat(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        // ... your implementation ...
    }
}

// Use it like any built-in provider
let client = Client::from_provider(Box::new(MyProvider { /* ... */ }));
let response = client.chat(&[Message::user("Hello")]).await?;
```

The unified trait (over separate `ChatCapable` / `ImageCapable` traits) was chosen because provider selection is config-driven and dynamic at runtime — you don't know at compile time which provider the user configured. One `Box<dyn Provider>` is simpler than juggling multiple trait objects. Library users who want compile-time guarantees can use the concrete types directly (`OpenAiClient`, `OllamaClient`, etc.).

### Multiple providers

Route different tasks to different models:

```rust
let fast = Client::from_config()?.with_provider("openai-fast")?;     // GPT-4o-mini
let smart = Client::from_config()?.with_provider("azure-gpt5")?;     // GPT-5.3
let creative = Client::from_config()?.with_provider("anthropic-sonnet")?; // Claude Sonnet 4.6
let images = Client::from_config()?.for_task(Task::ImageGeneration)?; // Nano Banana 2

// Classify with the fast model
let category = fast
    .chat(&[
        Message::system("Reply with one word: positive/negative/neutral"),
        Message::user(&feedback),
    ])
    .await?;

// Generate detailed response with the smart model
let response = smart
    .chat(&[Message::user(&feedback)])
    .await?;

// Generate an image
let image = images
    .generate_image("Product photo of the item")
    .await?;
```

### Provider detection

```rust
use ailloy::detect::detect_providers;

let detected = detect_providers().await;
for p in &detected {
    println!("{}: {} — {}", p.kind, p.name, p.details);
}

// Auto-configure from detection
if let Some(p) = detected.first() {
    let client = Client::from_detected(p)?;
}
```

### Error handling

```rust
use ailloy::{Client, ClientError, Message};

let client = Client::from_config()?;

match client.chat(&[Message::user("Hello")]).await {
    Ok(response) => println!("{}", response.content),
    Err(e) => match e.downcast_ref::<ClientError>() {
        Some(ClientError::Api { status: 401, .. }) => {
            eprintln!("Invalid API key.");
        }
        Some(ClientError::Api { status: 429, .. }) => {
            eprintln!("Rate limited. Retry later.");
        }
        Some(ClientError::Unsupported(cap)) => {
            eprintln!("This provider doesn't support {}.", cap);
        }
        Some(ClientError::BinaryNotFound { binary }) => {
            eprintln!("'{}' not found in PATH.", binary);
        }
        _ => eprintln!("Error: {e}"),
    },
}
```

---

## Part 4: Task types and capabilities

Ailloy abstracts multiple AI capabilities, not just chat:

| Task | Input | Output | CLI form |
| --- | --- | --- | --- |
| Chat | text (messages) | text | `ailloy "question"` |
| Chat (streaming) | text (messages) | text stream | `ailloy "question" --stream` |
| Image generation | text (prompt) | image bytes | `ailloy "prompt" -o image.png` |
| Embeddings | text | float vector | `ailloy embed "text"` |
| Transcription | audio file | text | `ailloy transcribe file.mp3` |

Task routing:

1. `defaults.chat` is used for chat (required — error if missing)
2. `defaults.image` is used when `--output` has an image extension
3. `defaults.embedding` is used for `ailloy embed`
4. `--provider name` always overrides the default
5. Providers without an explicit `task` field default to `task: chat`

---

## Part 5: Type reference

```rust
// --- Messages ---

pub struct Message {
    pub role: Role,
    pub content: String,
}

pub enum Role { System, User, Assistant }

// --- Chat ---

pub struct ChatOptions {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<Usage>,
}

pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// --- Streaming ---

pub enum StreamEvent {
    Delta(String),
    Done(ChatResponse),
}

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

// --- Images ---

pub struct ImageResponse {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub revised_prompt: Option<String>,
}

pub struct ImageOptions {
    pub size: Option<(u32, u32)>,
    pub quality: Option<String>,
    pub style: Option<String>,
}

pub enum ImageFormat { Png, Jpeg, Webp }

// --- Embeddings ---

pub struct EmbeddingResponse {
    pub vector: Vec<f32>,
    pub model: String,
    pub usage: Option<Usage>,
}

// --- Tasks ---

pub enum Task {
    Chat,
    ImageGeneration,
    Embedding,
    Transcription,
}
```

---

## Part 6: What the CLI doesn't expose

Library-only capabilities:

| Capability | Why library-only |
| --- | --- |
| `Conversation` with pluggable history | CLI uses in-memory; library users need persistence |
| Custom `Provider` trait implementations | CLI uses config; library users may have custom backends |
| `Client::from_provider(Box<dyn Provider>)` | Programmatic construction, no config file |
| Multiple concurrent clients | CLI uses one provider at a time |
| `ChatStream` with backpressure | CLI prints to stdout |
| `blocking::Client` | CLI always uses tokio |
| Batch embedding | CLI embeds one text at a time |

---

## Part 7: Migration from v0.1

Config migration:

```yaml
# v0.1 config
default_provider: openai
providers:
  openai:
    kind: openai
    api_key: sk-abc123
    model: gpt-4o

# v0.2 config (ailloy auto-migrates on first load)
defaults:
  chat: openai
providers:
  openai:
    kind: openai
    api_key: sk-abc123
    model: gpt-4o
```

API migration:

```rust
// v0.1 — works, low-level
let config = Config::load()?;
let provider = create_provider(&config)?;
let response = provider.chat(&[Message::user("Hello")]).await?;

// v0.2 — ergonomic
let client = Client::from_config()?;
let response = client.chat(&[Message::user("Hello")]).await?;

// v0.2 — raw clients still public for advanced use
use ailloy::openai::OpenAiClient;
let raw = OpenAiClient::new("sk-...", "gpt-4o", None);
let response = raw.chat(&[Message::user("Hello")], None).await?;
```

---

## Part 8: Resolved design decisions

1. **Default command parsing.** Pre-parse approach: before clap runs, check if `argv[1]` is a known subcommand. If not, insert `chat`. Predictable, no clap hacks, full validation either way.

2. **Stdin + message assembly.** Auto-detect piped stdin via `stdin().is_terminal()`. When piped + argument: `"{argument}\n\n{stdin}"`. When piped + no argument: stdin as-is. When not piped + no argument + no `-i`: show help.

3. **Provider routing for non-chat tasks.** Config has `defaults` map: `defaults.chat`, `defaults.image`, etc. Required for chat, optional for others. `--provider` always overrides. No auto-guessing.

4. **Unified Provider trait.** One trait with default methods returning `Unsupported`, not separate capability traits. Rationale: provider selection is config-driven and dynamic, so `Box<dyn Provider>` is the natural type. Compile-time guarantees are available through concrete types for advanced users.

5. **Blocking feature.** Own internal tokio runtime, like `reqwest::blocking`. Simple, well-understood tradeoff. Can't be called from within an existing async runtime — documented limitation.

6. **Streaming in blocking mode.** Iterator-based: `blocking::ChatStream` implements `Iterator<Item = Result<StreamEvent>>`. Natural for Rust, composable with standard iterator patterns.

7. **Image format handling.** Only support formats the provider actually supports. Clear error with list of supported formats. No silent conversion. `"dall-e-3 supports PNG and JPEG. SVG is not available."`

8. **Config structure.** Self-contained provider entries with `defaults` task-routing map. No inheritance, no shared credentials. Duplication is acceptable — clarity over DRY. `task` defaults to `chat` when omitted. `defaults.chat` is required.

9. **Client owns the provider.** `Client` holds `Box<dyn Provider>`. No lifetime complexity. Users who need sharing wrap in `Arc<Client>`.

10. **`Conversation::send()` returns owned `ChatResponse`.** Simpler than returning a reference. The response is also in history, so no data is lost.

11. **Streaming conversations auto-append.** When the stream's `Done` event fires, the assembled response is added to history automatically.

12. **`ChatHistory::messages()` returns `Vec<Message>`.** More flexible than `&[Message]` — allows filtering/windowing. Clone cost is negligible for typical conversation lengths.
