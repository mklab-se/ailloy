# ailloy

An AI abstraction layer for Rust.

## Commands

```bash
cargo build                              # Build all targets
cargo build --no-default-features --lib  # Build library only
cargo test                               # Run all tests
cargo clippy -- -D warnings              # Lint (CI-enforced)
cargo fmt --all -- --check               # Format check (CI-enforced)
cargo run -- --help                      # Run the CLI
```

## Architecture

Single crate with feature-flagged CLI, similar to how `clap` separates derive features:

```
src/
  lib.rs              # Public library API — always compiled
  config.rs           # Config types, load/save, local config merge, migration
  types.rs            # Message, Role, ChatResponse, ChatOptions, StreamEvent, ChatStream,
                      #   ImageResponse, ImageOptions, EmbeddingResponse, Task, Usage
  error.rs            # ClientError enum (thiserror) — Http, Api, Json, NotConfigured,
                      #   BinaryNotFound, Unsupported, Other
  client.rs           # Provider trait, Client struct, ClientBuilder, create_provider_from_config()
  conversation.rs     # ChatHistory trait, InMemoryHistory, Conversation
  blocking.rs         # Sync client wrapper (internal tokio current-thread runtime)
  provider.rs         # Legacy AiProvider enum (kept for backward compatibility)
  openai.rs           # OpenAI client — chat, stream (SSE), image gen, embeddings
  anthropic.rs        # Anthropic client — chat, stream (SSE)
  azure.rs            # Azure OpenAI client — chat, stream (SSE), image gen, embeddings
  vertex.rs           # Vertex AI client — Gemini chat/stream, Imagen, embeddings
  ollama.rs           # Ollama client — chat, stream (NDJSON), embeddings
  local_agent.rs      # Local CLI agent (claude, codex, copilot) — chat, stream (line-buffered)
  main.rs             # CLI entry point (requires "cli" feature)
  cli.rs              # Clap CLI definitions (requires "cli" feature)
  banner.rs           # ASCII art logo (requires "cli" feature)
  update.rs           # Background update checker via crates.io (requires "cli" feature)
  commands/
    mod.rs            # Command module exports
    chat.rs           # `ailloy chat` — chat, streaming, image gen, SVG, interactive, stdin
    config_cmd.rs     # `ailloy config init/show` — interactive setup (all 6 provider types)
    completion.rs     # `ailloy completion` — shell completions
    providers.rs      # `ailloy providers list/detect` — list and auto-detect providers
```

## Feature Flags

- `default = ["cli"]` — includes CLI binary and all CLI dependencies
- `cli` — enables clap, inquire, colored, tracing-subscriber, semver, and tokio runtime features
- Library users: `ailloy = { version = "0.2", default-features = false }`
- CLI users: `cargo install ailloy` (uses default features)

## Key Patterns

- Feature-flagged single crate: library code always compiles, CLI code gated behind `cli` feature via `required-features` on `[[bin]]`
- **Provider trait** (`client.rs`): unified `async_trait` with default methods returning `Unsupported` — `name()`, `chat()`, `chat_stream()`, `generate_image()`, `embed()`
- **Client** wraps `Box<dyn Provider>` — constructed via `from_config()`, `with_provider()`, `for_task()`, `from_provider()`, `builder()`, or direct constructors (`Client::openai()`, `Client::anthropic()`, etc.)
- **Streaming**: SSE parsing for OpenAI/Anthropic/Azure/Vertex via `futures_util::stream::unfold`, NDJSON for Ollama, line-buffered for local agents
- **Config v2**: `defaults` map routes task names (chat, image, embedding) to provider names; `ProviderConfig` has `task`, `auth`, `project`, `location`, `provider_defaults` fields
- **Local config**: `.ailloy.yaml` in current or parent directories, merged with global config
- **Config migration**: old `default_provider` auto-migrated to `defaults.chat` on load
- **Blocking wrapper**: `blocking::Client` with internal `tokio::runtime::Builder::new_current_thread()` — mirrors async Client API
- **Conversation**: `Conversation` struct with pluggable `ChatHistory` trait and `InMemoryHistory` default
- CLI built with `clap` derive macros + `clap_complete` for shell completions
- Default command pre-parsing: `ailloy "msg"` → `ailloy chat "msg"`
- Stdin detection: auto-reads piped input via `io::stdin().is_terminal()`
- Output routing: `-o image.png` → image generation, `-o file.svg` → SVG via chat, other → file save
- Async runtime: `tokio`
- Logging: `tracing` + `tracing-subscriber` (CLI only) with `-v`/`-vv` verbosity levels
- Colored output via `colored` crate (respects `--no-color`)
- Interactive prompts via `inquire`
- Error handling: `anyhow` for CLI commands, `thiserror` for `ClientError` in library code
- Config: `~/.config/ailloy/config.yaml` (via `dirs::config_dir()`)
- Update checker: background task, cached at `~/.cache/ailloy/`, skip with `AILLOY_NO_UPDATE_CHECK=1`
- Environment variable support: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` as fallback for providers

## Releasing

1. Bump `version` in `Cargo.toml`
2. Update `CHANGELOG.md`
3. Commit and push to main
4. Tag: `git tag v0.X.Y && git push origin v0.X.Y`
5. Release workflow builds binaries (Linux, macOS Intel+ARM, Windows), creates GitHub Release, updates Homebrew tap (`mklab-se/homebrew-tap`), publishes to crates.io

**Required GitHub secrets:**
- `CARGO_REGISTRY_TOKEN` (in `crates-io` environment)
- `HOMEBREW_TAP_TOKEN` (GitHub PAT with repo scope for `mklab-se/homebrew-tap`)

## Code Style

- Edition 2024, MSRV 1.85
- `cargo clippy` with `-D warnings` (zero warnings policy)
- `cargo fmt` enforced in CI

## Quality Requirements

### Testing
- **Always run the full test suite before declaring work complete:** `cargo test`
- **Always run the full CI check before pushing:** `cargo fmt --all -- --check && cargo clippy -- -D warnings && cargo test`
- Write unit tests for all new functionality — aim for high code coverage
- Test edge cases and error paths, not just the happy path
- For code that interacts with external services (OpenAI, Ollama), test parsing/logic locally with mock data

### Documentation
- **Before pushing or releasing, review all documentation for accuracy:**
  - `README.md` — features, quick start, badges
  - `INSTALL.md` — installation methods, shell completions
  - `CHANGELOG.md` — new entries for every user-visible change
  - `CLAUDE.md` — architecture, commands, patterns
- When adding new commands, flags, or provider types, update all relevant docs in the same commit
- `CHANGELOG.md` must be updated for every release with a dated entry following Keep a Changelog format
