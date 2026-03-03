# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
