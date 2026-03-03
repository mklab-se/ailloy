<p align="center">
  <img src="https://raw.githubusercontent.com/mklab-se/ailloy/main/media/ailloy-horizontal.png" alt="ailloy" width="600">
</p>

<h1 align="center">ailloy</h1>

<p align="center">An AI abstraction layer for Rust</p>

<p align="center">
  <a href="https://github.com/mklab-se/ailloy/actions/workflows/ci.yml"><img src="https://github.com/mklab-se/ailloy/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/ailloy"><img src="https://img.shields.io/crates/v/ailloy.svg" alt="crates.io"></a>
  <a href="https://github.com/mklab-se/ailloy/releases"><img src="https://img.shields.io/github/v/release/mklab-se/ailloy" alt="GitHub Release"></a>
  <a href="https://github.com/mklab-se/homebrew-tap"><img src="https://img.shields.io/badge/homebrew-tap-orange" alt="Homebrew"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

---

Ailloy provides a unified interface for interacting with multiple AI providers from Rust — both as a **CLI tool** for quick tasks and scripting, and as a **library** for integration into your own projects.

## Quick Start

### Install the CLI

```bash
# Homebrew (macOS/Linux)
brew install mklab-se/tap/ailloy

# Cargo
cargo install ailloy

# Cargo binstall (pre-built binary)
cargo binstall ailloy
```

### Configure a provider

```bash
ailloy config init
```

### Send a message

```bash
ailloy "Explain the Rust borrow checker in one sentence"
```

## Use as a Library

Add ailloy to your project without CLI dependencies:

```toml
[dependencies]
ailloy = { version = "0.2", default-features = false }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
anyhow = "1"
```

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
    let client = Client::openai("sk-...", "gpt-4o")?;
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
        .model("claude-sonnet-4-6")
        .build()?;
    let response = client.chat(&[Message::user("Hello!")]).await?;
    println!("{}", response.content);
    Ok(())
}
```

## Providers

| Provider | Kind | Chat | Stream | Images | Embeddings | Auth |
|----------|------|:----:|:------:|:------:|:----------:|------|
| OpenAI | `openai` | yes | yes | DALL-E | yes | API key |
| Anthropic | `anthropic` | yes | yes | — | — | API key |
| Azure OpenAI | `azure-openai` | yes | yes | yes | yes | API key / `az` CLI |
| Google Vertex AI | `vertex-ai` | yes | yes | Imagen | yes | `gcloud` CLI |
| Ollama | `ollama` | yes | yes | — | yes | None |
| Local Agent | `local-agent` | yes | yes | — | — | None |

## Configuration

Ailloy stores its configuration at `~/.config/ailloy/config.yaml`:

```yaml
defaults:
  chat: openai
  image: dalle
providers:
  openai:
    kind: open-ai
    api_key: sk-...
    model: gpt-4o
  anthropic:
    kind: anthropic
    api_key: sk-ant-...
    model: claude-sonnet-4-6
  dalle:
    kind: open-ai
    api_key: sk-...
    model: dall-e-3
    task: image-generation
  ollama:
    kind: ollama
    model: llama3.2
  claude:
    kind: local-agent
    binary: claude
```

### Local project config

Create `.ailloy.yaml` in your project root to override or add providers for that project. Local config is merged with global config.

## CLI Commands

| Command | Description |
|---------|-------------|
| `ailloy <message>` | Send a message (shorthand for `ailloy chat`) |
| `ailloy chat <message>` | Send a message to the configured AI provider |
| `ailloy chat -i` | Interactive conversation mode |
| `ailloy config init` | Interactive provider setup wizard |
| `ailloy config show` | Display current configuration |
| `ailloy providers list` | List configured providers |
| `ailloy providers detect` | Auto-detect available providers |
| `ailloy completion <shell>` | Generate shell completions |
| `ailloy version` | Show version and banner |

### Options

```bash
ailloy "message" --provider ollama       # Use a specific provider
ailloy "message" --system "Be brief"     # Set a system prompt
ailloy "message" --stream                # Stream response tokens
ailloy "message" --max-tokens 100        # Limit response length
ailloy "message" --temperature 0.7       # Control randomness
ailloy "message" -o response.txt         # Save response to file
ailloy "message" -o image.png            # Generate an image
ailloy "message" -o diagram.svg          # Generate SVG via chat
echo "prompt" | ailloy                   # Pipe input via stdin
ailloy -v chat "message"                 # Debug logging
ailloy -q chat "message"                 # Quiet mode
```

## Feature Flags

Ailloy uses feature flags to keep the library lean:

| Feature | Default | Description |
|---------|---------|-------------|
| `cli` | Yes | CLI binary and dependencies (clap, inquire, colored, etc.) |

Library users should disable default features:

```toml
ailloy = { version = "0.2", default-features = false }
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

## License

MIT
