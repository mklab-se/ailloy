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
ailloy = { version = "0.4", default-features = false }
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

Use it directly:

```bash
ailloy "Explain the Rust borrow checker in one sentence"
ailloy "A sunset over the ocean" -o sunset.png
```

## Providers

| Provider | Kind | Chat | Stream | Images | Auth |
|----------|------|:----:|:------:|:------:|------|
| OpenAI | `openai` | yes | yes | DALL-E | API key |
| Anthropic | `anthropic` | yes | yes | — | API key |
| Azure OpenAI | `azure-openai` | yes | yes | yes | API key / `az` CLI |
| Microsoft Foundry | `microsoft-foundry` | yes | yes | — | API key / `az` CLI |
| Google Vertex AI | `vertex-ai` | yes | yes | Imagen | `gcloud` CLI |
| Ollama | `ollama` | yes | yes | — | None |
| LM Studio | `openai` | yes | yes | — | None |
| Local Agent | `local-agent` | yes | yes | — | None |

**LM Studio** uses the OpenAI-compatible API (`http://localhost:1234` by default). **Local Agent** delegates to CLI tools installed on your system: `claude`, `codex`, or `copilot`.

## Configuration

Ailloy stores its configuration at `~/.config/ailloy/config.yaml`:

```yaml
nodes:
  openai/gpt-4o:
    provider: openai
    model: gpt-4o
    auth:
      env: OPENAI_API_KEY
    capabilities: [chat, image]

  anthropic/claude-sonnet-4-6:
    provider: anthropic
    model: claude-sonnet-4-6
    auth:
      env: ANTHROPIC_API_KEY
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

defaults:
  chat: openai/gpt-4o
  image: openai/gpt-4o
```

### Local project config

Create `.ailloy.yaml` in your project root to override or add nodes for that project. Local config is merged with global config (nodes and defaults merge; consents are global-only).

## CLI Commands

| Command | Description |
|---------|-------------|
| `ailloy <message>` | Send a message (shorthand for `ailloy chat`) |
| `ailloy chat <message>` | Send a message to the configured AI node |
| `ailloy chat -i` | Interactive conversation mode |
| `ailloy ai` | Show AI status |
| `ailloy ai config` | Interactive node configuration wizard |
| `ailloy ai config list-nodes` | List configured AI nodes |
| `ailloy ai config add-node` | Add a new AI node interactively |
| `ailloy ai config show` | Display current configuration |
| `ailloy ai config set-default <id> --task <cap>` | Set the default node for a capability |
| `ailloy ai test` | Test AI connectivity |
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
ailloy "message" -o response.txt         # Save response to file
ailloy "message" -o image.png            # Generate an image
ailloy "message" -o diagram.svg          # Generate SVG via chat
echo "prompt" | ailloy                   # Pipe input via stdin
ailloy "message" --raw                   # Raw output (no newline, no metadata)
ailloy -v chat "message"                 # Debug logging
ailloy -q chat "message"                 # Quiet mode
```

## Feature Flags

Ailloy uses feature flags to keep the library lean:

| Feature | Default | Description |
|---------|---------|-------------|
| `cli` | Yes | CLI binary and all dependencies (clap, inquire, colored, etc.) |
| `config-tui` | No* | Interactive config wizards, status display, enable/disable (inquire, colored) |

\* `config-tui` is automatically included when `cli` is enabled.

Library users should disable default features. To get interactive config TUI without the full CLI:

```toml
ailloy = { version = "0.5", default-features = false, features = ["config-tui"] }
```

For a pure library with no TUI deps:

```toml
ailloy = { version = "0.5", default-features = false }
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
