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
ailloy chat "Explain the Rust borrow checker in one sentence"
```

## Use as a Library

Add ailloy to your project without CLI dependencies:

```toml
[dependencies]
ailloy = { version = "0.1", default-features = false }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
anyhow = "1"
```

```rust
use ailloy::config::Config;
use ailloy::provider::create_provider;
use ailloy::types::Message;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    let provider = create_provider(&config)?;
    let response = provider.chat(&[Message::user("Hello!")]).await?;
    println!("{}", response.content);
    Ok(())
}
```

Or configure a provider directly in code:

```rust
use ailloy::openai::OpenAiClient;
use ailloy::types::Message;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = OpenAiClient::new("sk-...", "gpt-4o", None);
    let response = client.chat(&[Message::user("Hello!")]).await?;
    println!("{}", response.content);
    Ok(())
}
```

## Providers

| Provider | Kind | Auth | Notes |
|----------|------|------|-------|
| OpenAI | `openai` | API key | GPT-4o, GPT-4, etc. Works with any OpenAI-compatible endpoint |
| Ollama | `ollama` | None | Local LLMs (Llama, Mistral, etc.) |
| Local Agent | `local-agent` | None | Claude, Codex, Copilot via subprocess |
| Azure OpenAI | `azure-openai` | — | Coming soon |

## Configuration

Ailloy stores its configuration at `~/.config/ailloy/config.yaml`:

```yaml
default_provider: openai
providers:
  openai:
    kind: openai
    api_key: sk-...
    model: gpt-4o
  ollama:
    kind: ollama
    model: llama3.2
  claude:
    kind: local-agent
    binary: claude
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `ailloy chat <message>` | Send a message to the configured AI provider |
| `ailloy config init` | Interactive provider setup wizard |
| `ailloy config show` | Display current configuration |
| `ailloy providers list` | List configured providers |
| `ailloy completion <shell>` | Generate shell completions |
| `ailloy version` | Show version and banner |

### Options

```bash
ailloy chat "message" --provider ollama    # Use a specific provider
ailloy chat "message" --system "Be brief"  # Set a system prompt
ailloy -v chat "message"                   # Debug logging
ailloy -q chat "message"                   # Quiet mode
```

## Feature Flags

Ailloy uses feature flags to keep the library lean:

| Feature | Default | Description |
|---------|---------|-------------|
| `cli` | Yes | CLI binary and dependencies (clap, inquire, colored, etc.) |

Library users should disable default features:

```toml
ailloy = { version = "0.1", default-features = false }
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
