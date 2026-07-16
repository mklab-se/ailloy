# ailloy -- AI Agent Reference

## Overview

ailloy is a vendor-flexible AI integration library and CLI. It provides a
unified interface to multiple AI providers for chat, embeddings, and image
and video generation, with a node-based configuration system that makes
switching providers trivial.

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
| `-o, --output <FILE>` | Save response to file (image extensions trigger image gen, `.mp4` triggers video gen) |
| `-i, --interactive` | Interactive conversation mode |
| `--raw` | Output only raw model response |
| `--attach <FILE>` | Attach a file (image, pdf, or text) — repeatable |

Reads from stdin when piped. Running `ailloy "message"` is shorthand for `ailloy chat "message"`.
`--attach` accepts images (png, jpg, jpeg, gif, webp), pdf, and text files (txt, md, csv, json,
yaml, yml, xml, html); the media type is inferred from the extension. In `-i`/`--interactive`
mode, `--attach` files are attached to the first user message only.

`--schema` documents are auto-patched with `additionalProperties: false` on every object node
(as required by OpenAI-family strict mode), so you don't need to hand-add it; explicit values you
set are preserved.

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
| `--quality <Q>` | Image quality: `low`, `medium`, `high`, `auto` (gpt-image models; DALL·E takes `hd`/`standard`) |
| `--style <S>` | Image style (natural, vivid) — DALL·E only, ignored by gpt-image models |
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

### Video

```
ailloy video [MESSAGE] [OPTIONS]
```

Generate a video from a text description. Requires a node with the `video`
capability -- currently only Azure OpenAI and Microsoft Foundry nodes with a
Sora deployment support this.

| Flag | Description |
|------|-------------|
| `-n, --node <ID>` | Node to use for video generation |
| `-o, --output <FILE>` | Output file path (default: `ailloy-video-<timestamp>.mp4`) |
| `--size <WxH>` | Video size, e.g. 720x1280 or 1280x720 (model-dependent) |
| `--seconds <N>` | Clip duration in seconds (typically 4, 8, or 12 -- model-dependent) |
| `--variants <1-5>` | Number of video variants (each is a separate video creation) |
| `--raw` | Print only the output path(s), no banner or metadata |

Video generation drives the OpenAI-style Videos API
(`POST/GET/DELETE {base}/openai/v1/videos`) and is asynchronous: the CLI
creates the video(s), polls until they complete (or fail), and prints a status
line each time the status changes (queued -> in_progress -> completed). The
Videos API has no multi-variant field, so `--variants` issues N separate video
creations; results are written as `name.mp4`, `name-2.mp4`, `name-3.mp4`, ...
Video artifacts expire ~24h after completion.

```bash
ailloy video "A drone shot over a coastal cliff at sunrise"
ailloy video "Logo animation" -o logo.mp4 --size 1280x720 --seconds 8
```

Chat's `-o` routing also recognizes `.mp4` and delegates to video generation
with default options: `ailloy "A cat playing piano" -o cat.mp4`.

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
ailloy ai config              # Interactive configuration dashboard (TUI)
ailloy ai config add-node     # Add a new AI node (single add-node form)
ailloy ai config edit-node ID # Edit an existing node (single edit form)
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

#### Configuration dashboard

`ailloy ai config` opens a full-screen ratatui dashboard (on a TTY; without one
it prints status and exits). It edits the **global** config and saves changes
immediately. A two-pane layout shows the node table on the left and the selected
node's connection, capabilities, and per-node parameter defaults on the right.

Keys (Browse):

| Key | Action |
|-----|--------|
| `↑`/`↓`, `j` | Move selection / scroll the detail pane |
| `Tab` | Toggle focus between the node list and detail pane |
| `Enter` | Edit the highlighted per-node default (detail pane) |
| `a` | Add a node (opens the add-node form) |
| `e` | Edit the selected node |
| `x` | Delete the selected node (asks to confirm) |
| `d` | Set the selected node as the default for one of its capabilities |
| `k` | Store an API key in the OS keychain for the selected node |
| `t` | Run a one-line chat connectivity test against the selected node |
| `q`/`Esc` | Quit |

The add/edit form starts with a provider selector, then provider-specific fields
(model, endpoint, deployment, api_version, project/location, or agent binary), an
auth selector (`env`/`api_key`/`keychain`/`azure_cli`/`gcloud_cli` as applicable),
an optional alias, and capability toggles. In the form: `↑`/`↓` move between
fields, `←`/`→` change a selector, `Space` toggles a capability, `Ctrl+S` (or the
`[ Save ]` action) commits, and `Esc` cancels. For Azure OpenAI and Microsoft
Foundry, a `[ Discover via az CLI ]` action lists your subscriptions, resources,
and deployments via the Azure CLI (after a one-time consent prompt) and prefills
the form. A connectivity test blocks the UI briefly while it runs.

### Global Flags

| Flag | Description |
|------|-------------|
| `-v` | Increase verbosity (`-vv` for trace) |
| `-q, --quiet` | Suppress non-essential output |
| `--no-color` | Disable colored output |

## Provider Types

| Provider | Chat | Stream | Embed | Image | Video | Auth |
|----------|------|--------|-------|-------|-------|------|
| `openai` | yes | yes | yes | yes | no | API key, keychain, or env (`OPENAI_API_KEY`) |
| `anthropic` | yes | yes | no | no | no | API key, keychain, or env (`ANTHROPIC_API_KEY`) |
| `azure-openai` | yes | yes | yes | yes | yes (Sora deployment) | API key, keychain, Azure CLI, or env |
| `microsoft-foundry` | yes | yes | yes | yes (gpt-image deployment) | yes (Sora deployment) | API key, keychain, or Azure CLI |
| `vertex-ai` | yes | yes | yes | yes | no | gcloud CLI |
| `ollama` | yes | yes | yes | no | no | None (local) |
| `local-agent` | yes | yes | no | no | no | None (local binary: claude, codex, copilot) |

Azure OpenAI and Microsoft Foundry default to the unified `/openai/v1/`
endpoint surface (model field = deployment name). Set `api_version` on the
node to use the legacy dated endpoints instead. Video generation is only
available on Azure OpenAI and Microsoft Foundry nodes with a Sora deployment.

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

  microsoft-foundry/gpt-image-2:
    provider: microsoft-foundry
    model: gpt-image-2            # = deployment name on the v1 surface
    endpoint: https://myresource.services.ai.azure.com
    auth:
      azure_cli: true
    capabilities: [image]
    defaults:                     # per-node default parameters (see below)
      image.quality: low
      image.format: jpeg
      image.compression: "80"

defaults:
  chat: openai/gpt-5.4-mini
  image: microsoft-foundry/gpt-image-2

consents:
  azure-cli: true
```

### Per-node default parameters

Each node may carry a `defaults:` map of parameter values that are applied
whenever a call does not set that parameter explicitly. Resolution order is
always: explicit flag/option > node default > provider default. Values are
strings in YAML. Edit them in the `ailloy ai config` dashboard (Detail pane →
Enter) or directly in the YAML.

| Key | Values | Applies to |
|-----|--------|------------|
| `image.size` | `WxH`, e.g. `1024x1024` | image |
| `image.quality` | `low`, `medium`, `high`, `auto` | image |
| `image.format` | `png`, `jpeg` | image (OpenAI/Azure/Foundry) |
| `image.compression` | `0`–`100` (jpeg/webp only; auto-skipped when the effective format is png) | image (OpenAI/Azure/Foundry) |
| `image.background` | `transparent`, `opaque`, `auto` | image (OpenAI/Azure/Foundry) |
| `image.variants` | `1`–`10` | image |
| `video.size` | `WxH`, e.g. `1280x720` | video |
| `video.seconds` | `1`–`20` (API typically accepts 4/8/12) | video |
| `video.variants` | `1`–`5` | video |
| `chat.temperature` | `0`–`2` | chat |
| `chat.max_tokens` | positive integer | chat |
| `embedding.dimensions` | positive integer (legacy key `dimensions` also read) | embedding |

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
ailloy image "Logo design" -o logo.png --size 1024x1024 --quality high
ailloy image "3 icon options" -o icon.png --variants 3        # icon.png, icon-2.png, icon-3.png
ailloy image "same scene at night" --ref day.png -o night.png # edit from a reference image
```

### Chat with attachments

```bash
ailloy chat "What animal is in this image?" --attach photo.jpg
ailloy chat "Summarize this report" --attach report.pdf --raw
```

### Embeddings

```bash
ailloy embed "text to embed"          # dimensions + preview
ailloy embed "text to embed" --full   # full vector as JSON
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
