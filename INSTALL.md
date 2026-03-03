# Installation

## Homebrew (macOS/Linux)

```bash
brew install mklab-se/tap/ailloy
```

## Cargo

```bash
cargo install ailloy
```

## Cargo binstall

Pre-built binaries via [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall ailloy
```

## Shell Completions

Generate completions for your shell:

### Bash

```bash
ailloy completion bash > ~/.local/share/bash-completion/completions/ailloy
```

### Zsh

```bash
ailloy completion zsh > ~/.zfunc/_ailloy
# Then add to ~/.zshrc: fpath+=~/.zfunc
```

### Fish

```bash
ailloy completion fish > ~/.config/fish/completions/ailloy.fish
```

### PowerShell

```powershell
ailloy completion powershell >> $PROFILE
```
