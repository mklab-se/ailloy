# ailloy -- AI Agent Reference

## Overview

ailloy is a vendor-flexible AI integration library and CLI. It provides a
unified interface to multiple AI providers for chat, embeddings, and image
generation, with a node-based configuration system that makes switching
providers trivial.

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
| `--json` | Force the response to be a single JSON object (script-friendly) |
| `--schema <FILE>` | Force the response to match a JSON Schema file (implies `--json`) |
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
| `--format <F>` | Output image format (png, jpeg, webp) |
| `--compression <0-100>` | Compression level (only with `--format` jpeg or webp) |
| `--variants <1-10>` | Number of image variants to generate |
| `--background <B>` | Background transparency (transparent, opaque, auto) |
| `--moderation <M>` | Content moderation strictness (auto, low) |
| `--fidelity <F>` | How closely edits preserve details from reference images (high, low) |
| `--ref <FILE>` | Reference image to edit/compose from (repeatable); switches to the image-edits endpoint |
| `--mask <FILE>` | Mask image for inpainting (requires at least one `--ref` image) |
| `--raw` | No banner, no metadata |

With `--variants`, results are written as `name.png`, `name-2.png`, `name-3.png`, ...

### Embed

```
ailloy embed [TEXT] [OPTIONS]
```

Generate embeddings from text using the default (or specified) embedding node.

| Flag | Description |
|------|-------------|
| `-n, --node <ID>` | Node to use for embedding (overrides default) |
| `--full` | Print the full vector as JSON |
| `--info` | Show embedding node metadata |
| `--azure-vectorizer <NAME>` | Print Azure AI Search vectorizer JSON for the embedding node |

### Eval (LLM-as-judge)

```
ailloy eval [INPUT] [OPTIONS]
```

Evaluate input against plain-language criteria with an AI judge. Built for
scripts and integration tests: exit code 0 = pass, 1 = fail, 2 = usage/config
error, 3 = provider error.

| Flag | Description |
|------|-------------|
| `-c, --criteria <TEXT>` | Criteria the input must satisfy |
| `--criteria-file <FILE>` | Read criteria from a file |
| `-f, --file <FILE>` | Read the input to evaluate from a file |
| `--context <TEXT>` | Extra context for the judge (what produced the input, expectations) |
| `-n, --node <ID>` | Judge node (defaults to the default chat node) |
| `-t, --threshold <F>` | Pass when score >= threshold (0.0-1.0) instead of the judge's verdict |
| `--json` | Print the verdict as JSON |

Input comes from the positional argument, `--file`, or stdin:

```bash
my-tool run | ailloy eval --criteria "output mentions the order id"
ailloy eval "$output" -c "written in professional English" --threshold 0.8 --json
```

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
ailloy ai config set-key ID   # Store a node's API key in the OS keychain
                              #   (switches the node's auth to keychain)
ailloy ai config set-default NODE --task CAPABILITY  # Set default node
ailloy ai config reset        # Reset all configuration
ailloy ai test [MESSAGE]      # Test AI connectivity
ailloy ai test --all          # Ping every configured node (chat/embedding)
                              #   with latency; exit 1 if any node fails
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

| Provider | Chat | Stream | Embed | Image | Auth |
|----------|------|--------|-------|-------|------|
| `openai` | yes | yes | yes | yes | API key, keychain, or env (`OPENAI_API_KEY`) |
| `anthropic` | yes | yes | no | no | API key, keychain, or env (`ANTHROPIC_API_KEY`) |
| `azure-openai` | yes | yes | yes | yes | API key, keychain, Azure CLI, or env |
| `microsoft-foundry` | yes | yes | yes | no | API key, keychain, or Azure CLI |
| `vertex-ai` | yes | yes | yes | yes | gcloud CLI |
| `ollama` | yes | yes | yes | no | None (local) |
| `local-agent` | yes | yes | no | no | None (local binary: claude, codex, copilot) |

Azure OpenAI and Microsoft Foundry default to the unified `/openai/v1/`
endpoint surface (model field = deployment name). Set `api_version` on the
node to use the legacy dated endpoints instead.

## Configuration

Config file: `~/.config/ailloy/config.yaml`
Local override: `.ailloy.yaml` in current or parent directory (merged with global).

### Structure

```yaml
nodes:
  openai/gpt-5.4-mini:
    provider: openai
    model: gpt-5.4-mini
    auth:
      env: OPENAI_API_KEY
    capabilities: [chat, image]
    alias: gpt

  anthropic/claude-sonnet-5:
    provider: anthropic
    model: claude-sonnet-5
    auth:
      keychain: true
    capabilities: [chat]
    alias: claude

  ollama/llama3:
    provider: ollama
    model: llama3
    endpoint: http://localhost:11434
    capabilities: [chat]

defaults:
  chat: openai/gpt-5.4-mini
  image: openai/gpt-5.4-mini

consents:
  azure-cli: true
```

### Node ID Format

Node IDs follow the pattern `{provider}/{model}` (e.g., `openai/gpt-5.4-mini`,
`anthropic/claude-sonnet-5`). Each node can have an `alias` for shorthand
use (e.g., `--node gpt` instead of `--node openai/gpt-5.4-mini`).

### Auth Types

- `env` -- reads API key from a named environment variable (`env: OPENAI_API_KEY`)
- `api_key` -- stores API key directly in config (less secure)
- `keychain` -- reads API key from the OS keychain (service `ailloy`, account =
  node ID); store with `ailloy ai config set-key <node>`
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
ailloy chat "List 3 cities as JSON" --json  # Force JSON object output
ailloy chat "Extract the order" --schema order.schema.json  # Strict JSON Schema
```

### Judge output in scripts and tests

```bash
my-tool run | ailloy eval -c "output mentions the order id"   # exit 0/1
ailloy eval "$out" -c "polite tone" --threshold 0.8 --json    # scored, JSON verdict
```

### Store an API key in the OS keychain

```bash
ailloy ai config set-key openai/gpt-5.4-mini   # prompts for the key
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
