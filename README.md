<p align="center">
  <img src="https://raw.githubusercontent.com/mklab-se/ailloy/main/media/ailloy-horizontal.png" alt="ailloy" width="600">
</p>

<h1 align="center">ailloy</h1>

<p align="center">Build Rust tools with AI, without locking your users to one vendor</p>

<p align="center">
  <a href="https://github.com/mklab-se/ailloy/actions/workflows/ci.yml"><img src="https://github.com/mklab-se/ailloy/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/ailloy"><img src="https://img.shields.io/crates/v/ailloy.svg" alt="crates.io"></a>
  <a href="https://github.com/mklab-se/ailloy/releases"><img src="https://img.shields.io/github/v/release/mklab-se/ailloy" alt="GitHub Release"></a>
  <a href="https://github.com/mklab-se/homebrew-tap"><img src="https://img.shields.io/badge/homebrew-tap-orange" alt="Homebrew"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

---

If you build Rust tools for other people, Ailloy is for you.

Build once. Let your users choose their AI.

Most teams adding AI to a Rust product face the same problem: you do not know what AI access your users have.
Some users have OpenAI API keys. Others only have Azure OpenAI or Foundry access. Others can only use locally installed agents like `claude`, `codex`, or `copilot`.

Ailloy solves that distribution problem.

You integrate one Rust library once, and your users can bring their own AI path through configuration.

## The Core Promise

- You add AI features once in Rust.
- Your users keep freedom to use the AI access they already have.
- You avoid re-implementing provider setup, auth flows, and selection UX in every new tool.

## What Ailloy Is

- **First: a Rust library for shipping AI-enabled tools to diverse users.**
  Ailloy helps you ship AI features when your users have different vendor access, security constraints, and account setups.

- **Second: an abstraction layer for AI interaction from Rust.**
  The main goal is not to hide SDK ergonomics just for convenience. The main goal is to avoid hard-coding one vendor SDK into your product when your users may need another.

- **Third: an optional standalone CLI.**
  The `ailloy` binary is useful for quick terminal prompts, scripting, and setting global configuration, but it is not the primary reason the project exists.

## Why Teams Adopt Ailloy

Imagine you are building a Rust diff tool and want AI to generate a plain-English explanation of file changes.

Without Ailloy, you either:

- implement and maintain multiple provider integrations yourself,
- build repeated node/config/auth UX in each new tool,
- or lock your users to one vendor.

With Ailloy, you integrate once and let users pick what they already have:

- OpenAI API key,
- Azure OpenAI or Foundry,
- Ollama or LM Studio,
- or a local agent CLI like Claude Code.

That means faster delivery for you, less vendor lock-in for your users, and reusable AI plumbing across all your Rust tools.

If you are building multiple Rust tools over time, this compounds quickly: integrate once, reuse everywhere.

## Library Quick Start (Primary)

Add Ailloy to your project without CLI dependencies:

```toml
[dependencies]
ailloy = { version = "2.0", default-features = false }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
anyhow = "1"
```

Then call Ailloy from your app and let runtime config decide which provider is used:

### Async (recommended)

```rust
use ailloy::{Client, Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let response = client.chat(&[Message::user("Hello!")]).await?;
    println!("{}", response.content);
    Ok(())
}
```

### Blocking (sync)

```rust
use ailloy::blocking::Client;
use ailloy::Message;

fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let response = client.chat(&[Message::user("Hello!")])?;
    println!("{}", response.content);
    Ok(())
}
```

### Programmatic (no config file needed)

```rust
use ailloy::{Client, Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::openai("sk-...", "gpt-5.4-mini")?;
    let response = client.chat(&[Message::user("Hello!")]).await?;
    println!("{}", response.content);
    Ok(())
}
```

### Builder pattern

```rust
use ailloy::{Client, Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::builder()
        .anthropic()
        .api_key("sk-ant-...")
        .model("claude-sonnet-5")
        .build()?;
    let response = client.chat(&[Message::user("Hello!")]).await?;
    println!("{}", response.content);
    Ok(())
}
```

### Structured JSON output

Force the model to answer with JSON — a single object, or strict conformance to a JSON Schema:

```rust
use ailloy::{ChatOptions, Client, Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let options = ChatOptions::builder()
        .json_schema(
            "cities",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "cities": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["cities"]
            }),
        )
        .build();
    let response = client
        .chat_with(&[Message::user("List three Swedish cities")], &options)
        .await?;
    println!("{}", response.content); // valid JSON
    Ok(())
}
```

Use `.json()` instead of `.json_schema(...)` when any single JSON object will do. Structured output is native on OpenAI, Azure OpenAI, Microsoft Foundry, Ollama, and Vertex AI (`response_format` / `response_schema`), and prompted on Anthropic.

### Image generation

```rust
use ailloy::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let image = client.generate_image("A sunset over the ocean").await?;
    std::fs::write("sunset.png", &image.data)?;
    println!("{}x{} {}", image.width, image.height, image.format);
    Ok(())
}
```

`gpt-image-2` is supported on OpenAI, Azure OpenAI, and Microsoft Foundry, with a
full parameter surface: `output_format` (png/jpeg/webp), `compression`, `n`
(1-10 variants), `background` (transparent/opaque/auto), `moderation`,
`input_fidelity`, and `reference_images`/`mask` for image edits (switches to
the edits endpoint automatically):

```rust
use ailloy::{Client, ImageFormat, ImageOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let options = ImageOptions::builder()
        .quality("high")
        .output_format(ImageFormat::Png)
        .n(2)
        .build();
    let images = client.generate_images_with("A sunset over the ocean", &options).await?;
    for (i, image) in images.iter().enumerate() {
        std::fs::write(format!("sunset-{i}.png"), &image.data)?;
    }
    Ok(())
}
```

`Client::generate_image_with` is deprecated in favor of `generate_images_with`
(some models return multiple variants) — see `MIGRATION.md`.

### Video generation

Generate short video clips with Sora (`sora-2`) on Azure OpenAI or Microsoft
Foundry:

```rust
use ailloy::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let videos = client.generate_video("A drone shot over a coastal cliff at sunrise").await?;
    std::fs::write("clip.mp4", &videos[0].data)?;
    Ok(())
}
```

Under the hood this drives the OpenAI-style Videos API
(`POST/GET/DELETE {base}/openai/v1/videos`). Sizes are model-dependent (e.g.
`720x1280`, `1280x720`) and clip duration is commonly 4, 8, or 12 seconds.
`--variants` is implemented as N parallel video creations. The generation
manual-control methods `Client::create_video_job`, `get_video_job`,
`download_video`, and `delete_video_job` are available, and
`generate_video_with_progress` takes a callback invoked on every status
transition. Video artifacts expire ~24h after completion. From the CLI:

```bash
ailloy video "A drone shot over a coastal cliff at sunrise"
ailloy video "Logo animation" -o logo.mp4 --size 1280x720 --seconds 8
ailloy "A cat playing piano" -o cat.mp4   # chat -o routing also generates video
```

### Chat attachments

Attach images, PDFs, or text files to a chat message; the media type is
inferred from the file extension:

```rust
use ailloy::{Client, Message};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let message = Message::user_with_attachments(
        "What's in this image?",
        &[PathBuf::from("screenshot.png")],
    )?;
    let response = client.chat(&[message]).await?;
    println!("{}", response.content);
    Ok(())
}
```

From the CLI: `ailloy chat "What's in this image?" --attach screenshot.png`
(repeatable, works in single-shot, stdin, and interactive modes). Provider
support and the full attachment-mapping table are in `MIGRATION.md`.

## Optional CLI (Secondary)

If you want a terminal workflow or scripting support, install the CLI:

```bash
# Homebrew (macOS/Linux)
brew install mklab-se/tap/ailloy

# Cargo
cargo install ailloy

# Cargo binstall (pre-built binary)
cargo binstall ailloy
```

Configure your nodes:

```bash
ailloy ai config
```

This opens a full-screen dashboard: a node table on the left, and connection
info, capabilities, and per-node parameter defaults for the selected node on
the right. Keys: `↑`/`↓`/`j` navigate, `Tab` switch focus, `Enter` edit a
default, `a` add a node, `e` edit, `x` delete, `d` set as the capability
default, `k` store a keychain key, `t` test connectivity, `q`/`Esc` quit.
Without a TTY it falls back to printing status.

Use it directly:

```bash
ailloy "Explain the Rust borrow checker in one sentence"
ailloy "A sunset over the ocean" -o sunset.png
ailloy chat "List three Swedish cities as JSON" --json
```

## Evaluate (LLM-as-judge)

`ailloy eval` turns an AI model into a judge with script-friendly exit codes — built for integration tests of AI-powered tools. Traditional assertions cannot check non-deterministic output; an LLM judge can:

```bash
# Judge any output against plain-language criteria (exit 0 pass, 1 fail)
my-tool ask "Summarize the incident report" | ailloy eval \
  --criteria "mentions the outage start time, the root cause, and a follow-up action"

# In a test script or CI job
if ! echo "$output" | ailloy eval -c "written in professional English"; then
  echo "quality gate failed"; exit 1
fi

# Machine-readable verdict, extra context, score threshold
ailloy eval "$output" --criteria-file criteria.txt \
  --context "input is a summary of incident INC-4711" \
  --threshold 0.8 --json
```

Options: `--criteria/-c` or `--criteria-file`, input as an argument, `--file`, or stdin, `--context` for background, `--node` to pick the judge, `--threshold` to gate on a 0.0–1.0 score, `--json` for structured output. Exit codes: `0` pass, `1` fail, `2` usage error, `3` provider error. See `examples/eval.sh` for the full pattern.

## Providers

| Provider | Kind | Chat | Stream | Images | Video | Auth |
|----------|------|:----:|:------:|:------:|:-----:|------|
| OpenAI | `openai` | yes | yes | DALL-E, gpt-image-2 | — | API key |
| Anthropic | `anthropic` | yes | yes | — | — | API key |
| Azure OpenAI | `azure-openai` | yes | yes | yes (gpt-image-2) | Sora | API key / `az` CLI |
| Microsoft Foundry | `microsoft-foundry` | yes | yes | yes (gpt-image-2) | Sora | API key / `az` CLI |
| Google Vertex AI | `vertex-ai` | yes | yes | Imagen | — | `gcloud` CLI |
| Ollama | `ollama` | yes | yes | — | — | None |
| LM Studio | `openai` | yes | yes | — | — | None |
| Local Agent | `local-agent` | yes | yes | — | — | None |

**LM Studio** uses the OpenAI-compatible API (`http://localhost:1234` by default). **Local Agent** delegates to CLI tools installed on your system: `claude`, `codex`, or `copilot`.

**Azure OpenAI and Microsoft Foundry** default to the unified `/openai/v1/` endpoint surface — no dated `api-version` needed, and the `model` field is your deployment name. Nodes that set an explicit `api_version` in config keep using the legacy dated endpoints. Video (Sora) uses the OpenAI-style `/openai/v1/videos` endpoints, following the same rule — no `?api-version` on the v1 surface, appended only for dated nodes.

Chat, image, and video attachments/mapping also vary by provider — see the provider support table in `MIGRATION.md` for exactly which providers accept image/PDF/text attachments.

## Configuration

Ailloy stores its configuration at `~/.config/ailloy/config.yaml`:

```yaml
nodes:
  openai/gpt-5.4-mini:
    provider: openai
    model: gpt-5.4-mini
    auth:
      env: OPENAI_API_KEY
    capabilities: [chat, image]

  anthropic/claude-sonnet-5:
    provider: anthropic
    model: claude-sonnet-5
    auth:
      keychain: true
    capabilities: [chat]

  ollama/llama3.2:
    provider: ollama
    model: llama3.2
    endpoint: http://localhost:11434
    capabilities: [chat]

  lm-studio/qwen3.5:
    provider: openai
    model: qwen3.5
    endpoint: http://localhost:1234
    capabilities: [chat]

  openai/gpt-image-2:
    provider: openai
    model: gpt-image-2
    auth:
      env: OPENAI_API_KEY
    capabilities: [image]
    defaults:
      image.quality: high
      image.format: png

defaults:
  chat: openai/gpt-5.4-mini
  image: openai/gpt-image-2
```

### API keys in the OS keychain

Instead of environment variables or inline keys, store API keys in the operating system keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service):

```bash
ailloy ai config set-key openai/gpt-5.4-mini   # prompts for the key, stores it securely
```

This switches the node's auth to `keychain: true` — the key never touches the config file. Keys are stored under service `ailloy` with the node ID as account. Keychain support is behind the `keychain` feature (enabled by default).

### Local project config

Create `.ailloy.yaml` in your project root to override or add nodes for that project. Local config is used instead of global config (nodes and defaults merge; consents are global-only).

### Per-node default parameters

Each node can carry its own defaults for tunable request parameters — `defaults:`
under a node in YAML (`AiNode.node_defaults` in Rust), keyed by capability, e.g.
`image.quality`, `image.format`, `video.seconds`, `chat.temperature`,
`embedding.dimensions`. Resolution order is explicit call options > node
defaults > provider defaults, so passing an explicit `ImageOptions`/`ChatOptions`
always wins. The full set of recognized keys, their value shapes, and which
providers accept them live in `src/params.rs` (also drives the `ailloy ai config`
dashboard's Defaults editor — see below).

## CLI Commands

| Command | Description |
|---------|-------------|
| `ailloy <message>` | Send a message (shorthand for `ailloy chat`) |
| `ailloy chat <message>` | Send a message to the configured AI node |
| `ailloy chat -i` | Interactive conversation mode |
| `ailloy chat <message> --attach FILE` | Attach an image/PDF/text file (repeatable) |
| `ailloy image <prompt>` | Generate an image |
| `ailloy video <prompt>` | Generate a video (sora-2, Azure OpenAI / Microsoft Foundry) |
| `ailloy embed <text>` | Generate embeddings |
| `ailloy eval <input> -c <criteria>` | LLM-as-judge evaluation (exit 0 pass, 1 fail) |
| `ailloy ai` | Show AI status (includes model retirement warnings) |
| `ailloy ai config` | Full-screen node configuration dashboard (TUI) |
| `ailloy ai config list-nodes` | List configured AI nodes |
| `ailloy ai config add-node` | Add a new AI node interactively |
| `ailloy ai config set-key <id>` | Store a node's API key in the OS keychain |
| `ailloy ai config show` | Display current configuration |
| `ailloy ai config set-default <id> --task <cap>` | Set the default node for a capability |
| `ailloy ai test` | Test AI connectivity |
| `ailloy ai test --all` | Ping every configured node with latency (exit 1 on failures) |
| `ailloy ai enable` / `disable` | Toggle AI features |
| `ailloy completion <shell>` | Generate shell completions |
| `ailloy version` | Show version and banner |

### Options

```bash
ailloy "message" --node ollama/llama3.2  # Use a specific node
ailloy "message" --system "Be brief"     # Set a system prompt
ailloy "message" --stream                # Stream response tokens (always on in -i mode)
ailloy "message" --max-tokens 100        # Limit response length
ailloy "message" --temperature 0.7       # Control randomness
ailloy "message" --json                  # Force a single JSON object response
ailloy "message" --schema out.json       # Force response to match a JSON Schema file
ailloy "message" -o response.txt         # Save response to file
ailloy "message" -o image.png            # Generate an image
ailloy "message" -o clip.mp4             # Generate a video
ailloy "message" -o diagram.svg          # Generate SVG via chat
ailloy "message" --attach screenshot.png # Attach a file (repeatable)
echo "prompt" | ailloy                   # Pipe input via stdin
ailloy "message" --raw                   # Raw output (no newline, no metadata)
ailloy -v chat "message"                 # Debug logging
ailloy -q chat "message"                 # Quiet mode
```

## Feature Flags

Ailloy uses feature flags to keep the library lean:

| Feature | Default | Description |
|---------|---------|-------------|
| `cli` | Yes | CLI binary and all dependencies (clap, clap_complete, inquire, colored, crossterm, ratatui, etc.) |
| `keychain` | Yes | OS keychain storage for API keys (keyring) |
| `config-tui` | No* | Full-screen ratatui config dashboard, status display, enable/disable (colored, crossterm, ratatui) |

\* `config-tui` is automatically included when `cli` is enabled. `inquire` is a `cli`-only dependency (used by `ai config set-key`'s key prompt) — it is not pulled in by `config-tui` alone.

Library users should disable default features. To get the interactive config dashboard without the full CLI:

```toml
ailloy = { version = "2.0", default-features = false, features = ["config-tui"] }
```

For a pure library with no TUI deps:

```toml
ailloy = { version = "2.0", default-features = false }
```

Add `keychain` to either of the above to read `auth: keychain` nodes without the CLI:

```toml
ailloy = { version = "2.0", default-features = false, features = ["keychain"] }
```

## Development

```bash
cargo build                              # Build everything
cargo build --no-default-features --lib  # Build library only
cargo test                               # Run tests
cargo clippy -- -D warnings              # Lint (zero warnings)
cargo fmt --all -- --check               # Format check
cargo run -- chat "hello"                # Run the CLI
```


## Folder-local configuration

Drop a `.ailloy.yaml` in a repository and it becomes the complete ailloy
configuration for everything run inside — different projects can use different
providers, models, and defaults. The **closest** file (walking up from the
working directory) wins; folders without one fall back to the machine-wide
config. Add `extends: global` to merge with the global config instead of
replacing it. Tool consents always come from the global config.

```bash
ailloy ai config init-local          # starter file (replaces global here)
ailloy ai config init-local --extends-global
ailloy ai status                     # shows which config file is active
```

## Upgrading from 1.x

Ailloy 2.0 changes `Message.content` from a plain `String` to a `MessageContent`
enum (to support attachments) and deprecates `generate_image_with` /
`ImageOptionsBuilder::style` in favor of their multi-image/gpt-image
replacements. Message *construction* (`Message::user(..)` etc.) is unaffected;
code that *reads* `.content` as a string needs a one-line change. See
[`MIGRATION.md`](MIGRATION.md) for the full guide.

## License

MIT
