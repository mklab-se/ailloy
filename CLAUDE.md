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
  config.rs           # Config types (AiNode, Capability, Auth, ProviderKind, Config,
                      #   EmbeddingMetadata), load/save, local config merge, node CRUD,
                      #   alias resolution, capability filtering, ALL_CAPABILITIES constant,
                      #   Azure AI Search vectorizer export, programmatic config API
                      #   (AiNode::new, ensure_node, upsert_node, set_default_for),
                      #   keychain helpers (keychain_secret, set_keychain_secret,
                      #   delete_keychain_secret — gated on "keychain" feature)
  config_tui.rs       # Shared config entry points (requires "config-tui" feature) —
                      #   consent helpers, enable/disable/is_ai_active, status/node
                      #   printing, test chat, reset; add_node_interactive/
                      #   edit_node_interactive/run_interactive_config are thin wrappers
                      #   over the ratatui dashboard in src/tui/ (inquire flows removed)
  tui/                # Full-screen ratatui config dashboard (requires "config-tui") —
                      #   mod (event loop, TerminalGuard, Effect executor: Save/RunTest/
                      #   StoreKeychain/Discover/Quit; run_single_form; az-CLI discovery
                      #   + connectivity test run inline), app (App/Mode/Effect reducer),
                      #   forms (Editor + NodeForm: per-provider fields, auth, capability
                      #   toggles, to_node/from_node), ui (table/detail/form/popups),
                      #   actions (upsert_node/delete_node/set_capability_default/
                      #   set_node_default)
  azure_discover.rs   # Azure CLI wrappers (requires "config-tui" feature) —
                      #   list subscriptions, resources, deployments via `az` CLI
  types.rs            # Message, Role, ChatResponse, ChatOptions (incl. response_format),
                      #   ResponseFormat (JsonObject/JsonSchema + per-provider request values),
                      #   StreamEvent, ChatStream, ImageResponse, ImageOptions, EmbedResponse,
                      #   EmbedOptions, Task, Usage, sampling-rejection detection
  error.rs            # ClientError enum (thiserror) — Http, Api, Json, NotConfigured,
                      #   BinaryNotFound, NodeNotFound, Unsupported, Other
  client.rs           # Provider trait, Client struct, ClientBuilder, create_provider_from_node()
  conversation.rs     # ChatHistory trait, InMemoryHistory, Conversation
  blocking.rs         # Sync client wrapper (internal tokio current-thread runtime)
  discover.rs         # Discovery library API — discover_env_keys(), discover_local(),
                      #   discover_ollama(), DiscoveredNode struct
  openai.rs           # OpenAI client — chat, stream (SSE), image gen, embedding
  anthropic.rs        # Anthropic client — chat, stream (SSE), prompted JSON output
  azure.rs            # Azure OpenAI client — chat, stream (SSE), image gen, embedding;
                      #   defaults to unified /openai/v1/ surface, dated api-version = legacy
  foundry.rs          # Microsoft Foundry client — chat, stream (SSE), embedding;
                      #   defaults to unified /openai/v1/ surface, dated api-version = legacy
  vertex.rs           # Vertex AI client — Gemini chat/stream, Imagen, embedding
  ollama.rs           # Ollama client — chat, stream (NDJSON), embedding
  local_agent.rs      # Local CLI agent (claude, codex, copilot) — chat, stream (line-buffered)
  retirement.rs       # Static model retirement table + retirement_warning() for
                      #   `ailloy ai status` warnings
  main.rs             # CLI entry point (requires "cli" feature)
  cli.rs              # Clap CLI definitions (requires "cli" feature)
  banner.rs           # ASCII art logo (requires "cli" feature)
  update.rs           # Background update checker via crates.io (requires "cli" feature)
  commands/
    mod.rs            # Command module exports
    ai.rs             # `ailloy ai` — unified AI management dispatcher, backward-compat
                      #   handlers, `set-key` (keychain), `test --all` (ping all nodes)
    chat.rs           # `ailloy chat` — chat, streaming, image gen, SVG, interactive, stdin
    image.rs          # `ailloy image` — image generation, direct and interactive modes
    embed.rs          # `ailloy embed` — embedding generation, metadata, Azure vectorizer export
    eval.rs           # `ailloy eval` — LLM-as-judge evaluation, exit codes 0/1/2/3
    config_cmd.rs     # Non-interactive config commands: `show/set/get/unset`
    skill.rs          # `ailloy ai skill` — skill setup guide, emit skill markdown, reference docs
    completion.rs     # `ailloy completion` — shell completions
    util.rs           # Shared CLI utilities: Spinner, ThinkFilter, file_hyperlink
  doc/
    ai-reference.md   # Full CLI reference documentation, embedded via include_str!
examples/
  chat.rs             # Library quickstart — chat + structured JSON output
  configure.rs        # Programmatic config for dependent tools (ensure_node, keychain)
  eval.sh             # LLM-as-judge integration-test pattern with `ailloy eval`
```

## Feature Flags

- `default = ["cli", "keychain"]` — CLI binary, all CLI dependencies, and OS keychain support
- `cli` — enables `config-tui`, clap, tracing-subscriber, semver, and tokio runtime features
- `keychain` — OS keychain storage for API keys via the `keyring` crate (service `ailloy`, account = node ID); without it, `Auth::Keychain` nodes fail with an actionable error
- `config-tui` — enables the ratatui config dashboard, status display, enable/disable (colored, crossterm, ratatui); consumer projects use this without pulling in clap. `inquire` is now a `cli`-only dependency (used by `ai config set-key`), not part of `config-tui`
- Library users (pure): `ailloy = { version = "1.0", default-features = false }`
- Library users (with TUI): `ailloy = { version = "1.0", default-features = false, features = ["config-tui"] }`
- Library users needing keychain auth: add `"keychain"` to `features`
- CLI users: `cargo install ailloy` (uses default features)

## Key Patterns

- Feature-flagged single crate: library code always compiles, CLI code gated behind `cli` feature via `required-features` on `[[bin]]`
- **AI Nodes**: atomic config units representing a specific model from a specific provider with connection details and capability tags; node IDs follow `{provider}/{model|deployment|binary}` pattern with optional `alias` for shorthand
- **Provider trait** (`client.rs`): unified `async_trait` with default methods returning `Unsupported` — `name()`, `chat()`, `chat_stream()`, `generate_image()`, `embed()`
- **Client** wraps `Box<dyn Provider>` — constructed via `from_config()`, `with_node()`, `for_capability()`, `from_node()`, `builder()`, or direct constructors (`Client::openai()`, `Client::anthropic()`, etc.)
- **Streaming**: SSE parsing for OpenAI/Anthropic/Azure/Vertex via `futures_util::stream::unfold`, NDJSON for Ollama, line-buffered for local agents
- **Config**: `nodes` map of `AiNode` structs; `defaults` map routes capability names (chat, image, video, embedding) to node IDs; `Auth` enum supports `env`, `api_key`, `keychain`, `azure_cli`, `gcloud_cli`; all config maps use `BTreeMap` for deterministic serialization
- **Programmatic config API**: dependent tools build nodes with `AiNode::new(provider)` and install them via `Config::ensure_node` (never overwrites existing user config), `Config::upsert_node`, and `Config::set_default_for`; secrets go through `ailloy::config::{keychain_secret, set_keychain_secret, delete_keychain_secret}` — see `examples/configure.rs`
- **Azure/Foundry endpoint rule**: no `api_version` on the node (the default) → unified `/openai/v1/` surface, model field = deployment name; explicit `api_version` in config → legacy dated endpoints (`/openai/deployments/...` for Azure, `/models/...` for Foundry). `AzureOpenAiClient::new`/`FoundryClient::new` build v1 clients; `with_api_version` builds legacy ones; `Client::azure`/`Client::foundry` take `Option<String>` api_version
- **Structured output**: `ChatOptions.response_format` (`ResponseFormat::JsonObject` / `JsonSchema`, builder `.json()` / `.json_schema(name, schema)`); native on OpenAI-family/Ollama (`response_format`/`format`) and Vertex (`response_mime_type`/`response_schema`), prompted JSON on Anthropic
- **Sampling guard**: all HTTP providers retry once without `temperature` when the model rejects sampling params (`is_sampling_rejection` in types.rs) — covers gpt-5.x/o-series, newest Claude, Gemini 3
- **Model retirements**: static prefix table in `retirement.rs`; `ailloy ai status` warns on configured models with scheduled/past retirement dates and suggests replacements
- **Interactive config TUI**: `ailloy ai config` opens a full-screen ratatui dashboard (`src/tui/`) — a two-pane node table + detail view with keys `a`/`e`/`x`/`d`/`k`/`t` (add/edit/delete/set-default/keychain/test); a provider-selector form with dynamic per-provider fields and capability toggles (`ProviderKind::supported_capabilities()` constrains them). All state lives in `app::App` with a pure reducer returning `Effect`s executed by the event loop in `tui::mod`. `config_tui.rs` keeps the stable non-UI API (status/print/consent/test) plus thin wrappers over the dashboard
- **Discovery**: `discover.rs` library provides `discover_env_keys()`, `discover_local()`, `discover_ollama()` returning data only; Azure/Foundry discovery is in `azure_discover.rs` (library level, gated on `config-tui`), driven inline by the dashboard's add-node `[ Discover via az CLI ]` action behind a consent modal
- **Local config**: `.ailloy.yaml` in current or parent directories, merged with global config (nodes/defaults merge, consents are global-only)
- **CLI tool consent**: `consents` map in config tracks user permission for external tools (`azure-cli`, `gcloud-cli`); security decisions use global config only (not overridable by local `.ailloy.yaml`)
- **Azure auto-discovery**: `azure_discover.rs` wraps `az` CLI for subscription/resource/deployment listing; discovers both `kind=='OpenAI'` and `kind=='AIServices'` resources; the `ailloy ai config` dashboard uses it when the user consents
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

- Edition 2024, MSRV 1.86
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
