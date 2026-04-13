# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.2] - 2026-04-13

### Changed

- Bump `actions/checkout` from v4 to v5 in CI and release workflows for Node.js 24 support

## [0.7.1] - 2026-04-13

### Fixed

- Azure Search vectorizer export now supports Microsoft Foundry nodes — converts `.services.ai.azure.com` endpoints to the `.openai.azure.com` variant that Azure AI Search expects
- Config TUI now shows an "Embedding" capability column with default indicators
- Config TUI right margin fix — capability column labels are no longer clipped at the terminal edge

## [0.7.0] - 2026-04-13

### Added

- **Embedding support** — first-class embedding capability across OpenAI, Azure OpenAI, Ollama, Vertex AI, and Microsoft Foundry
- **`Client::embed()`, `Client::embed_with()`, `Client::embed_one()`** — async and blocking convenience methods for generating embeddings (`Vec<f32>`)
- **`EmbeddingMetadata`** — queryable struct for embedding node configuration (provider, model, endpoint, deployment, dimensions, auth)
- **`EmbeddingMetadata::to_azure_search_vectorizer()`** — generates ready-to-use Azure AI Search vectorizer JSON, including API key when configured
- **`Capability::Embedding`** and **`Task::Embedding`** — config routing for embedding nodes with `defaults.embedding` config key
- **`ailloy embed` CLI command** — `ailloy embed "text"` generates embeddings; `--full` prints full JSON vector; `--info` shows node metadata; `--azure-vectorizer NAME` outputs Azure AI Search vectorizer config

## [0.6.0] - 2026-03-20

### Added

- **`ailloy ai skill` command** — helps set up Claude Code skills for ailloy. `--emit` outputs a ready-to-save skill markdown file, `--reference` outputs full up-to-date CLI reference documentation for agent consumption
- **`ailloy ai status` command** — show AI configuration status (same as running `ailloy ai` without a subcommand)
- **Embedded AI reference documentation** — `src/doc/ai-reference.md` bundled into the binary via `include_str!` for runtime access

## [0.5.2] - 2026-03-18

_Patch release — no functional changes._

## [0.5.1] - 2026-03-18

### Added

- **`ailloy image` command** — dedicated image generation command. `ailloy image "prompt"` generates an image directly, with auto-generated filenames, `--size`, `--quality`, and `--style` options
- **`ailloy image -i`** — interactive mode where Ailloy interviews you about the image you want to create, then generates it with your approval before proceeding
- **Interactive greetings** — `ailloy chat -i` and `ailloy image -i` now show a model-generated greeting when starting, with a spinner while waiting for the response. The assistant identifies itself as Ailloy

### Changed

- **Shared CLI utilities** — extracted `Spinner`, `ThinkFilter`, `strip_think_blocks`, and `file_hyperlink` into `commands/util.rs` for reuse across chat and image commands

## [0.5.0] - 2026-03-18

### Added

- **Interactive table-based config TUI** — `ailloy ai config` now shows a keyboard-navigable table of all AI nodes with capability columns. Arrow keys to navigate, `a` to add, `Enter` to edit, `d` to delete, `q` to quit
- **Form-based node editor** — pressing Enter on a node opens an inline form with text fields, capability checkboxes, and default toggles. No more dropping out of the TUI into a different prompt style
- **`config-tui` feature flag** — new feature that gates interactive config wizards, status display, enable/disable, and test functions behind `inquire` + `colored` + `crossterm` deps. The `cli` feature implies `config-tui`. Library consumers can opt into `config-tui` without pulling in clap: `ailloy = { version = "0.5", default-features = false, features = ["config-tui"] }`
- **`ailloy::config_tui` module** — shared interactive configuration TUI for consumer projects (hoist, cosq, mdeck). Provides `print_ai_status()`, `print_nodes_list()`, `run_interactive_config()`, `add_node_interactive()`, `edit_node_interactive()`, `run_test_chat()`, `enable_ai()`, `disable_ai()`, `is_ai_active()`, `reset_config()`, and consent helpers
- **`ailloy::azure_discover` module** — Azure CLI wrappers moved from CLI-only to library level, accessible to consumer projects with `config-tui` feature
- **`ailloy ai` command** — new unified AI management command with subcommands: `config`, `test`, `enable`, `disable`
- **`ailloy ai config` subcommands** — `add-node`, `edit-node`, `delete-node`, `set-default`, `list-nodes`, `show-node`, `show`, `set`, `get`, `unset`, `reset`

### Changed

- **CLI command restructure** — `ailloy config`, `ailloy nodes`, and `ailloy discover` are now under `ailloy ai config`. Old commands still work with deprecation notices
- **Positioning and docs messaging** — clarified Ailloy as library-first for Rust tool builders who need vendor-flexible AI integration for end users; reframed CLI as optional/secondary across README, install docs, crate metadata, and CLI-facing text

### Removed

- **Embedding support** — `Capability::Embedding`, `EmbeddingResponse`, `Task::Embedding`, and all provider `embed()` implementations removed. Ailloy now supports Chat and Image Generation only
- **`Client::embed()` and `blocking::Client::embed()`** — embedding API methods removed from both async and sync clients

### Deprecated

- **`ailloy config`** — use `ailloy ai config` instead
- **`ailloy nodes`** — use `ailloy ai config` subcommands instead
- **`ailloy discover`** — discovery is now integrated into `ailloy ai config` wizard

## [0.4.2] - 2026-03-04

### Fixed

- **Capability matrix** — removed Ollama and Microsoft Foundry from image generation support (neither implements `generate_image`)
- **README** — added LM Studio to providers table, added image generation examples (CLI and library), added `--raw` flag to options, fixed `--stream` description, added LM Studio config example

## [0.4.1] - 2026-03-04

### Fixed

- **Local agent CLI invocation** — each local agent binary now uses its correct non-interactive invocation: `codex exec`, `copilot --prompt`, `claude --print`. Previously all agents were invoked with `--print`, which only works for Claude.

## [0.4.0] - 2026-03-04

### Added

- **AI Nodes config model** — configuration now uses atomic "AI Nodes" (`AiNode`) instead of named providers. Each node represents a specific model from a specific provider with connection details, auth, and capability tags. Node IDs follow `{provider}/{model}` pattern (e.g., `openai/gpt-4o`, `ollama/llama3.2`).
- **`ailloy nodes` command** — new top-level command for node management: `list`, `add`, `edit`, `remove`, `default`, `show`
- **`ailloy discover` command** — auto-detect available AI providers and models from environment variables, running Ollama instances, and local CLI agents (claude, codex, copilot)
- **Discovery library API** — `discover_env_keys()`, `discover_local()`, `discover_ollama()` functions return data-only `DiscoveredNode` structs for programmatic use
- **Node aliases** — optional `alias` field on nodes for shorthand references (e.g., `alias: gpt` to reference `openai/gpt-4o` as just `gpt`)
- **Structured auth** — `Auth` enum supports `env` (environment variable), `api_key` (inline), `azure_cli`, `gcloud_cli` with explicit YAML map serialization
- **Capability enum** — explicit `Capability` enum (`Chat`, `Image`, `Embedding`) replaces string-based task routing
- **`--node` flag** — `ailloy chat --node ollama/llama3.2` to use a specific node (replaces `--provider`)
- **`Client::with_node()`** — load config and create a client for a specific node by ID or alias
- **`Client::for_capability()`** — create a client for the default node of a given capability
- **`Client::from_node()`** — create a client directly from an `AiNode` struct (no config file needed)
- **`NodeNotFound` error variant** — specific error type for missing node references
- **LM Studio provider** — added as a provider option during node setup, using the OpenAI-compatible API with `http://localhost:1234` default endpoint
- **Model listing from provider APIs** — when adding a node, ailloy fetches available models from OpenAI, Anthropic, Ollama, and LM Studio APIs and presents them in a Select prompt (with 5-second timeout and manual entry fallback)
- **Connection config reuse** — when adding a new node for a provider that already has configured nodes, offers to reuse existing connection settings (auth + endpoint)
- **Per-model capability selection** — when adding a node, users select which capabilities the model supports via multi-select prompt (auto-assigned when provider supports only one capability)
- **Think block filtering** — `<think>...</think>` reasoning blocks from models like Qwen and DeepSeek are stripped from displayed output in both streaming and non-streaming modes (full response preserved in conversation history)
- **Image generation spinner** — animated spinner shown during image generation to indicate progress
- **Smart update checker** — update notifications are suppressed when running from source (`cargo run`), and the upgrade hint adapts to install method (`brew upgrade` vs `cargo install`)

### Changed

- **Config format** — `nodes` map replaces `providers` map; `ProviderConfig` replaced by `AiNode`; this is a clean break (no migration from old format)
- **Config UI** — `ailloy config` replaced crossterm-based TUI with sequential `inquire` prompts (Select/Confirm/Text)
- **Interactive mode always streams** — `ailloy chat -i` now streams responses by default for real-time token display
- **`create_provider_from_node()`** replaces `create_provider_from_config()` in client.rs
- **`ailloy providers`** command replaced by **`ailloy nodes`**
- **OpenAI provider auth is optional** — nodes with `ProviderKind::OpenAi` and no auth configured (e.g., LM Studio) no longer fail; an empty API key is used instead

### Removed

- **`provider.rs`** — legacy `AiProvider` enum removed
- **`providers.rs`** — `ailloy providers list/detect` command replaced by `ailloy nodes list` and `ailloy discover`
- **`tui.rs`** — custom crossterm-based TUI replaced by `inquire` prompts
- **Old config format support** — configs without `nodes` key are treated as empty (clean break)

## [0.3.1] - 2026-03-04

_Patch release — no functional changes._

## [0.3.0] - 2026-03-04

### Added

- **Custom crossterm-based config TUI** — `ailloy config` now uses a custom terminal UI built with crossterm: providers grouped by capability (Chat, Image Generation, Embeddings) with `★` prefix for defaults, `>` cursor indicator, non-selectable section headers, `<add new>` per section, and keyboard shortcuts (Space=toggle default, Enter=edit, Delete/Backspace=delete, A=add, Q/ESC=quit)
- **Capability-aware provider setup** — adding a provider from a task section filters to only providers that support that task; defaults are auto-assigned and filtered by capability
- **Provider placement algorithm** — each provider appears exactly once in the TUI, placed under the task it's default for (or the first supported task if not a default)
- **Deterministic config serialization** — config fields now use `BTreeMap` instead of `HashMap`, ensuring consistent key ordering in `config.yaml` across saves
- **`ProviderKind::supports_task()`** — public method encoding the provider capability matrix (which providers support chat, image, embedding)
- **Non-interactive config commands** — `ailloy config set`, `get`, and `unset` for scripting and CI/CD (e.g., `ailloy config set providers.openai.model gpt-4o`, `ailloy config get defaults.chat`, `ailloy config unset providers.openai`)
- **Microsoft Foundry provider** — new provider supporting the Model Inference API (`*.services.ai.azure.com`) for models from multiple vendors (GPT, Llama, Mistral, etc.) via `kind: microsoft-foundry`
- **Microsoft Foundry auto-discovery** — `ailloy config` discovers AIServices resources and deployments via `az` CLI, alongside existing Azure OpenAI discovery
- **CLI tool consent system** — ailloy now asks for explicit permission before running external CLI tools (`az`, `gcloud`), with "remember my choice" option persisted in config
- **Azure OpenAI auto-discovery** — when consented, `ailloy config` uses `az` CLI to automatically discover subscriptions, Azure OpenAI resources, and deployed models
- **Consent display** — `ailloy config show` now displays tool consent status
- **Provider detection consent gating** — `ailloy providers detect` respects consent decisions for `az` and `gcloud` checks
- **Terminal hyperlinks** — saved file paths are now clickable (Cmd+click) in supported terminals (Ghostty, iTerm2, WezTerm, VS Code) via OSC 8 escape sequences
- **Accurate image dimensions** — image width/height are now read from the actual PNG/JPEG/WebP data instead of trusting API defaults
- **`--raw` flag** — `ailloy chat --raw` outputs only the raw model response (no trailing newline, no metadata, no color), ideal for piping and scripting

### Changed

- **Config UI redesign** — `ailloy config` (no subcommand) is now the primary entry point with a task-centric home screen, auto-save on every mutation. The old `ailloy config init` still works but is hidden.
- **Microsoft Foundry rebranding** — "Azure AI Foundry" renamed to "Microsoft Foundry" throughout: config kind `azure-ai-foundry` → `microsoft-foundry`, struct `AzureFoundryClient` → `FoundryClient`, all display strings and error messages updated
- **Azure OpenAI default API version** — updated from `2025-01-01` (GA) to `2025-04-01-preview` to support newer models like GPT-5.2
- **Microsoft Foundry default API version** — updated to `2024-05-01-preview` (Model Inference API)
- **`max_tokens` → `max_completion_tokens`** — OpenAI, Azure OpenAI, and Microsoft Foundry providers now use `max_completion_tokens` in API requests (required by newer models)
- **XDG config/cache paths** — config now lives at `~/.config/ailloy/` and cache at `~/.cache/ailloy/` on all platforms (respects `XDG_CONFIG_HOME`/`XDG_CACHE_HOME`), instead of macOS-native `~/Library/Application Support/`
- **Image format display** — format now shows as `PNG`/`JPEG`/`WebP` instead of `Png`/`Jpeg`/`Webp`

### Fixed

- **Microsoft Foundry 404 bug** — auto-discovered endpoints from `az cognitiveservices account list` used `*.cognitiveservices.azure.com` which doesn't serve the `/models/` path; now auto-converts to `*.services.ai.azure.com` both at discovery time and request time
- **"Try it" hint** — changed from `ailloy chat "Hello!"` to `ailloy 'Hello'` (avoids zsh `!` history expansion, uses default command)
- **Azure error messages** — API errors now include the affected resource, endpoint, and actionable next steps instead of raw error text
- **Azure OpenAI 404 with newer models** — GA API version `2025-01-01` returned "Resource not found" for models like `gpt-5.2-chat`; fixed by defaulting to preview API version
- **OpenAI image generation with gpt-image models** — `gpt-image-1` and `gpt-image-1.5` models rejected `response_format` and `style` parameters; request body is now built conditionally based on model family (gpt-image uses `output_format`, DALL-E uses `response_format`/`style`)
- **OpenAI image generation with chat models** — chat models like `gpt-5` and `gpt-4o` can now generate images via the Responses API (`/v1/responses`) with the `image_generation` tool; dedicated image models (`dall-e-*`, `gpt-image-*`) continue to use the Images API (`/v1/images/generations`)

## [0.2.0] - 2026-03-03

### Added

- **New providers:** Anthropic (Claude), Azure OpenAI, Google Vertex AI (Gemini, Imagen)
- **Unified `Provider` trait** with `Client` abstraction — `Client::from_config()`, `Client::builder()`, programmatic constructors (`Client::openai()`, `Client::anthropic()`, etc.)
- **Streaming support** for all providers via `chat_stream()` (SSE for OpenAI/Anthropic/Azure/Vertex, NDJSON for Ollama, line-buffered for local agents)
- **Image generation** via `generate_image()` — OpenAI DALL-E, Azure DALL-E, Vertex AI Imagen
- **Embeddings** via `embed()` — OpenAI, Ollama, Azure, Vertex AI
- **Conversation management** — `Conversation` struct with pluggable `ChatHistory` trait and `InMemoryHistory`
- **Blocking (sync) client** — `blocking::Client` with internal tokio runtime for non-async applications
- **Interactive mode** — `ailloy chat -i` for REPL conversations with `/quit`, `/clear`, `/help` commands
- **Streaming CLI** — `ailloy chat --stream` for real-time token output
- **Output routing** — `ailloy chat -o image.png` triggers image generation, `-o file.svg` generates SVG via chat
- **Stdin support** — pipe input via `echo "prompt" | ailloy chat`
- **Default command** — `ailloy "message"` works as shorthand for `ailloy chat "message"`
- **Chat options** — `--max-tokens`, `--temperature` flags
- **Provider auto-detection** — `ailloy providers detect` scans for available providers (env vars, running services, CLI tools)
- **Task-based defaults** — config `defaults` map routes tasks (chat, image, embedding) to different providers
- **Local project config** — `.ailloy.yaml` in current/parent directory merges with global config
- **Config wizard** — updated `ailloy config init` with all 6 provider types including auth method selection

### Changed

- Config format updated: `default_provider` replaced by `defaults` map (auto-migrated on load)
- `ProviderConfig` gains `task`, `auth`, `project`, `location`, `provider_defaults` fields
- Library users now use `Client` and `Provider` trait instead of `create_provider()` directly

## [0.1.0] - 2026-03-03

### Added

- Initial release
- Multi-provider AI support: OpenAI, Ollama, local CLI agents (Claude, Codex, Copilot)
- CLI commands: `chat`, `config init`, `config show`, `providers list`, `completion`, `version`
- Library API for Rust integration (`ailloy = { version = "0.1", default-features = false }`)
- Feature-flagged design: `cli` feature for CLI dependencies, lean library without it
- YAML-based configuration at `~/.config/ailloy/config.yaml`
- Interactive setup wizard via `ailloy config init`
- Shell completion support (Bash, Zsh, Fish, PowerShell)
- Background update checker
- CI/CD pipeline with multi-platform builds, Homebrew tap, and crates.io publishing
