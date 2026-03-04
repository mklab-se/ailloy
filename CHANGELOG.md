# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
