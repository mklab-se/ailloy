use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ailloy", version, about = "An AI abstraction layer for Rust")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Increase verbosity (use -vv for trace)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Disable colored output
    #[arg(long, global = true)]
    pub no_color: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Send a message to the configured AI provider
    Chat(ChatArgs),

    /// Manage ailloy configuration
    #[command(subcommand)]
    Config(ConfigCommands),

    /// List and manage AI providers
    #[command(subcommand)]
    Providers(ProviderCommands),

    /// Generate shell completions
    Completion(CompletionArgs),

    /// Show version information
    Version,
}

#[derive(clap::Args)]
pub struct ChatArgs {
    /// The message to send (optional if piped via stdin or using -i)
    pub message: Option<String>,

    /// Provider to use (overrides default)
    #[arg(short, long)]
    pub provider: Option<String>,

    /// System prompt
    #[arg(short, long)]
    pub system: Option<String>,

    /// Stream the response token by token
    #[arg(long)]
    pub stream: bool,

    /// Maximum tokens to generate
    #[arg(long)]
    pub max_tokens: Option<u32>,

    /// Temperature for generation (0.0 - 2.0)
    #[arg(long)]
    pub temperature: Option<f32>,

    /// Save response to file (image extensions trigger image generation)
    #[arg(short, long)]
    pub output: Option<String>,

    /// Interactive conversation mode
    #[arg(short, long)]
    pub interactive: bool,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Interactive configuration setup
    Init,
    /// Show current configuration
    Show,
}

#[derive(Subcommand)]
pub enum ProviderCommands {
    /// List configured providers
    List,
    /// Auto-detect available providers
    Detect,
}

#[derive(clap::Args)]
pub struct CompletionArgs {
    /// Shell to generate completions for
    pub shell: clap_complete::Shell,
}

/// Known subcommand names for default command pre-parsing.
pub const KNOWN_SUBCOMMANDS: &[&str] = &[
    "chat",
    "config",
    "providers",
    "completion",
    "version",
    "help",
];
