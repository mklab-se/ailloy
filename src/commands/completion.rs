use std::io;

use anyhow::Result;
use clap::CommandFactory;
use clap_complete::generate;

use crate::cli::{Cli, CompletionArgs};

/// Generate STATIC shell completions (commands, flags, and known flag values).
///
/// For DYNAMIC completion — where `--node` and node-id arguments complete from
/// the user's configured nodes — the user registers ailloy's built-in
/// `clap_complete` CompleteEnv completer instead (see `CompletionArgs` help and
/// INSTALL.md), e.g. `source <(COMPLETE=zsh ailloy)` for zsh. That path is
/// wired up in `main()` via `clap_complete::CompleteEnv`.
pub fn run(args: CompletionArgs) -> Result<()> {
    let mut cmd = Cli::command();
    generate(args.shell, &mut cmd, "ailloy", &mut io::stdout());
    Ok(())
}
