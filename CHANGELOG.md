# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
