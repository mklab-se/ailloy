# ailloy -- AI Agent Reference

## Overview

ailloy is a vendor-flexible AI integration library and CLI. It provides a
unified interface to multiple AI providers for chat and image generation,
with a node-based configuration system that makes switching providers trivial.

## CLI Command Reference

### Chat

```
ailloy chat [MESSAGE] [OPTIONS]
```

Send a message to the configured AI provider.

| Flag | Description |
|------|-------------|
| `-n, --node <ID>` | Node to use (overrides default) |
| `-s, --system <PROMPT>` | System prompt |
| `--stream` | Stream response token by token |
| `--max-tokens <N>` | Maximum tokens to generate |
| `--temperature <F>` | Temperature (0.0-2.0) |
| `-o, --output <FILE>` | Save response to file (image extensions trigger image gen) |
| `-i, --interactive` | Interactive conversation mode |
| `--raw` | Output only raw model response |

Reads from stdin when piped. Running `ailloy "message"` is shorthand for `ailloy chat "message"`.

### Image

```
ailloy image [MESSAGE] [OPTIONS]
```

Generate an image from a text description.

| Flag | Description |
|------|-------------|
| `-n, --node <ID>` | Node to use for image generation |
| `-o, --output <FILE>` | Output file path (auto-generated if omitted) |
| `-i, --interactive` | AI helps describe the image |
| `--size <WxH>` | Image size (e.g. 1024x1024) |
| `--quality <Q>` | Image quality (hd, standard) |
| `--style <S>` | Image style (natural, vivid) |
| `--raw` | No banner, no metadata |

### AI Management

```
ailloy ai                     # Show AI status
ailloy ai status              # Show AI status (same as above)
ailloy ai config              # Interactive configuration wizard
ailloy ai config add-node     # Add a new AI node
ailloy ai config edit-node ID # Edit an existing node
ailloy ai config delete-node ID  # Delete a node
ailloy ai config list-nodes   # List all configured nodes
ailloy ai config show-node ID # Show node details
ailloy ai config show         # Show full configuration
ailloy ai config set KEY VAL  # Set a config value (dot notation)
ailloy ai config get KEY      # Get a config value
ailloy ai config unset KEY    # Remove a config value
ailloy ai config set-default NODE --task CAPABILITY  # Set default node
ailloy ai config reset        # Reset all configuration
ailloy ai test [MESSAGE]      # Test AI connectivity
ailloy ai enable              # Enable AI features
ailloy ai disable             # Disable AI features
ailloy ai skill               # Show skill setup guide
ailloy ai skill --emit        # Output skill markdown
ailloy ai skill --reference   # Output this reference
```

### Global Flags

| Flag | Description |
|------|-------------|
| `-v` | Increase verbosity (`-vv` for trace) |
| `-q, --quiet` | Suppress non-essential output |
| `--no-color` | Disable colored output |

## Provider Types

| Provider | Chat | Stream | Image | Auth |
|----------|------|--------|-------|------|
| `openai` | yes | yes | yes | API key or env (`OPENAI_API_KEY`) |
| `anthropic` | yes | yes | no | API key or env (`ANTHROPIC_API_KEY`) |
| `azure-openai` | yes | yes | yes | API key, Azure CLI, or env |
| `microsoft-foundry` | yes | yes | no | API key or Azure CLI |
| `vertex-ai` | yes | yes | yes | gcloud CLI |
| `ollama` | yes | yes | no | None (local) |
| `local-agent` | yes | yes | no | None (local binary: claude, codex, copilot) |

## Configuration

Config file: `~/.config/ailloy/config.yaml`
Local override: `.ailloy.yaml` in current or parent directory (merged with global).

### Structure

```yaml
nodes:
  openai/gpt-4o:
    provider: openai
    model: gpt-4o
    auth:
      type: env
      var: OPENAI_API_KEY
    capabilities: [chat, image]
    alias: gpt4o

  anthropic/claude-sonnet-4-20250514:
    provider: anthropic
    model: claude-sonnet-4-20250514
    auth:
      type: env
      var: ANTHROPIC_API_KEY
    capabilities: [chat]
    alias: claude

  ollama/llama3:
    provider: ollama
    model: llama3
    base_url: http://localhost:11434
    capabilities: [chat]

defaults:
  chat: openai/gpt-4o
  image: openai/gpt-4o

consents:
  azure-cli: true
```

### Node ID Format

Node IDs follow the pattern `{provider}/{model}` (e.g., `openai/gpt-4o`,
`anthropic/claude-sonnet-4-20250514`). Each node can have an `alias` for shorthand
use (e.g., `--node gpt4o` instead of `--node openai/gpt-4o`).

### Auth Types

- `env` -- reads API key from an environment variable (`var` field)
- `api_key` -- stores API key directly in config (less secure)
- `azure_cli` -- uses `az` CLI for Azure authentication
- `gcloud_cli` -- uses `gcloud` CLI for Google Cloud authentication

## Common Workflows

### First-time setup

```bash
ailloy ai config          # Interactive wizard guides through provider setup
```

### Quick chat

```bash
ailloy "What is Rust?"                     # Default provider
ailloy chat "Explain monads" --node claude  # Specific node by alias
echo "Summarize this" | ailloy chat        # Pipe from stdin
```

### Image generation

```bash
ailloy image "A sunset over mountains"
ailloy image "Logo design" -o logo.png --size 1024x1024
```

### Check status and test

```bash
ailloy ai status    # See configured nodes and defaults
ailloy ai test      # Send a test message to verify connectivity
```

### Switch providers

```bash
ailloy ai config set-default ollama/llama3 --task chat
```
