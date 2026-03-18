# ailloy

Vendor-flexible AI integration for Rust tools, with an optional CLI.

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
  config.rs           # Config types (AiNode, Capability, Auth, ProviderKind, Config),
                      #   load/save, local config merge, node CRUD, alias resolution,
                      #   capability filtering, ALL_CAPABILITIES constant
  config_tui.rs       # Shared interactive config TUI (requires "config-tui" feature) —
                      #   consent prompts, interactive wizard, node setup, enable/disable,
                      #   status display, test chat, reset, Azure/Foundry discovery flows
  azure_discover.rs   # Azure CLI wrappers (requires "config-tui" feature) —
                      #   list subscriptions, resources, deployments via `az` CLI
  types.rs            # Message, Role, ChatResponse, ChatOptions, StreamEvent, ChatStream,
                      #   ImageResponse, ImageOptions, Task, Usage
  error.rs            # ClientError enum (thiserror) — Http, Api, Json, NotConfigured,
                      #   BinaryNotFound, NodeNotFound, Unsupported, Other
  client.rs           # Provider trait, Client struct, ClientBuilder, create_provider_from_node()
  conversation.rs     # ChatHistory trait, InMemoryHistory, Conversation
  blocking.rs         # Sync client wrapper (internal tokio current-thread runtime)
  discover.rs         # Discovery library API — discover_env_keys(), discover_local(),
                      #   discover_ollama(), DiscoveredNode struct
  openai.rs           # OpenAI client — chat, stream (SSE), image gen
  anthropic.rs        # Anthropic client — chat, stream (SSE)
  azure.rs            # Azure OpenAI client — chat, stream (SSE), image gen
  foundry.rs          # Microsoft Foundry client — chat, stream (SSE)
  vertex.rs           # Vertex AI client — Gemini chat/stream, Imagen
  ollama.rs           # Ollama client — chat, stream (NDJSON)
  local_agent.rs      # Local CLI agent (claude, codex, copilot) — chat, stream (line-buffered)
  main.rs             # CLI entry point (requires "cli" feature)
  cli.rs              # Clap CLI definitions (requires "cli" feature)
  banner.rs           # ASCII art logo (requires "cli" feature)
  update.rs           # Background update checker via crates.io (requires "cli" feature)
  commands/
    mod.rs            # Command module exports
    ai.rs             # `ailloy ai` — unified AI management dispatcher, backward-compat handlers
    chat.rs           # `ailloy chat` — chat, streaming, image gen, SVG, interactive, stdin
    config_cmd.rs     # Non-interactive config commands: `show/set/get/unset`
    completion.rs     # `ailloy completion` — shell completions
```

## Feature Flags

- `default = ["cli"]` — includes CLI binary and all CLI dependencies
- `cli` — enables `config-tui`, clap, tracing-subscriber, semver, and tokio runtime features
- `config-tui` — enables interactive config wizards, table-based TUI, status display, enable/disable (inquire, colored, crossterm); consumer projects use this without pulling in clap
- Library users (pure): `ailloy = { version = "0.5", default-features = false }`
- Library users (with TUI): `ailloy = { version = "0.5", default-features = false, features = ["config-tui"] }`
- CLI users: `cargo install ailloy` (uses default features)

## Key Patterns

- Feature-flagged single crate: library code always compiles, CLI code gated behind `cli` feature via `required-features` on `[[bin]]`
- **AI Nodes**: atomic config units representing a specific model from a specific provider with connection details and capability tags; node IDs follow `{provider}/{model|deployment|binary}` pattern with optional `alias` for shorthand
- **Provider trait** (`client.rs`): unified `async_trait` with default methods returning `Unsupported` — `name()`, `chat()`, `chat_stream()`, `generate_image()`
- **Client** wraps `Box<dyn Provider>` — constructed via `from_config()`, `with_node()`, `for_capability()`, `from_node()`, `builder()`, or direct constructors (`Client::openai()`, `Client::anthropic()`, etc.)
- **Streaming**: SSE parsing for OpenAI/Anthropic/Azure/Vertex via `futures_util::stream::unfold`, NDJSON for Ollama, line-buffered for local agents
- **Config**: `nodes` map of `AiNode` structs; `defaults` map routes capability names (chat, image) to node IDs; `Auth` enum supports `env`, `api_key`, `azure_cli`, `gcloud_cli`; all config maps use `BTreeMap` for deterministic serialization
- **Interactive config TUI**: `ailloy ai config` shows a crossterm-based table of nodes with capability columns; form-based editor for adding/editing nodes; `ProviderKind::supports_task()` drives capability filtering; TUI logic lives in `config_tui.rs` (library level, gated on `config-tui` feature) so consumer projects can reuse it
- **Discovery**: `discover.rs` library provides `discover_env_keys()`, `discover_local()`, `discover_ollama()` returning data only; Azure/Foundry discovery is in `azure_discover.rs` (library level, gated on `config-tui`), integrated into the `ai config` wizard's add-node flow
- **Local config**: `.ailloy.yaml` in current or parent directories, merged with global config (nodes/defaults merge, consents are global-only)
- **CLI tool consent**: `consents` map in config tracks user permission for external tools (`azure-cli`, `gcloud-cli`); security decisions use global config only (not overridable by local `.ailloy.yaml`)
- **Azure auto-discovery**: `azure_discover.rs` wraps `az` CLI for subscription/resource/deployment listing; discovers both `kind=='OpenAI'` and `kind=='AIServices'` resources; `ailloy ai config` wizard uses it when user consents
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
- Error handling: `anyhow` for CLI commands, `thiserror` for `ClientError` in library code. **All error messages must be actionable** — tell the user what went wrong, what resource/config is involved, and what to do next (e.g. "run 'az login'", "run 'ailloy config'"). Never show raw API errors like "Resource not found" without context.
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
