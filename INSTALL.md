# Installation

Primary use case: add Ailloy as a Rust library in your own tool.

```toml
[dependencies]
ailloy = { version = "2.0", default-features = false }
```

The CLI is optional and useful for scripting or direct terminal usage.

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

## Software bill of materials (SBOM)

Every GitHub release asset has a matching CycloneDX 1.5 SBOM listing the exact crate versions
compiled into that platform's binary:

```
ailloy-vX.Y.Z-<target>.cdx.json
```

The binaries are also built with [`cargo auditable`](https://github.com/rust-secure-code/cargo-auditable),
so the dependency list travels inside the executable itself. Check a downloaded binary against the
RustSec advisory database with:

```sh
cargo install cargo-audit --features=fix
cargo audit bin ./ailloy
```

`syft` and `trivy` also understand this format.

## Shell Completions

ailloy offers two kinds of completion.

### Static completions

`ailloy completion <shell>` generates a static script covering commands, flag
names, and known flag values (e.g. `--quality` → low/medium/high/...). Install
it once:

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

### Dynamic completions (recommended)

Static scripts can't know your configured nodes. For completion that also
completes `--node` and node-id arguments (`ai config set-default`,
`edit-node`, `delete-node`, `show-node`, `set-key`) from the nodes in your
config — showing each node's provider and model as a hint — register ailloy's
built-in completer instead. It runs `ailloy` itself on each Tab, so new nodes
show up immediately with no regeneration.

```bash
# zsh — add to ~/.zshrc
source <(COMPLETE=zsh ailloy)

# bash — add to ~/.bashrc
source <(COMPLETE=bash ailloy)

# fish — add to ~/.config/fish/completions/ailloy.fish
COMPLETE=fish ailloy | source
```

Reload your shell (or `source` the rc file) afterwards. Use either the static
script or the dynamic completer for a given shell, not both.
