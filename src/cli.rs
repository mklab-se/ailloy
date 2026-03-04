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
    Config(ConfigArgs),

    /// List and manage AI nodes
    #[command(subcommand)]
    Nodes(NodeCommands),

    /// Discover available AI providers and models
    Discover(DiscoverArgs),

    /// Generate shell completions
    Completion(CompletionArgs),

    /// Show version information
    Version,
}

#[derive(clap::Args)]
pub struct ChatArgs {
    /// The message to send (optional if piped via stdin or using -i)
    pub message: Option<String>,

    /// Node to use (overrides default, accepts ID or alias)
    #[arg(short, long)]
    pub node: Option<String>,

    /// Provider to use (hidden alias for --node)
    #[arg(short, long, hide = true)]
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

    /// Output only the raw model response (no newline, no metadata, no color)
    #[arg(long)]
    pub raw: bool,
}

impl ChatArgs {
    /// Resolve the effective node identifier from --node or --provider (hidden alias).
    pub fn effective_node(&self) -> Option<&str> {
        self.node.as_deref().or(self.provider.as_deref())
    }
}

#[derive(clap::Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: Option<ConfigCommands>,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Interactive configuration setup
    #[command(hide = true)]
    Init,
    /// Show current configuration
    Show,
    /// Set a config value (dot notation: defaults.chat, nodes.openai/gpt-4o.model)
    Set {
        /// Key in dot notation (e.g., defaults.chat, nodes.openai/gpt-4o.model)
        key: String,
        /// Value to set
        value: String,
    },
    /// Get a config value (dot notation: defaults.chat, nodes.openai/gpt-4o)
    Get {
        /// Key in dot notation (e.g., defaults.chat, nodes.openai/gpt-4o)
        key: String,
    },
    /// Remove a config value (dot notation: defaults.chat, nodes.openai/gpt-4o)
    Unset {
        /// Key in dot notation (e.g., defaults.chat, nodes.openai/gpt-4o)
        key: String,
    },
}

#[derive(Subcommand)]
pub enum NodeCommands {
    /// List all configured nodes
    List,
    /// Add a new node interactively
    Add,
    /// Edit a node's configuration
    Edit {
        /// Node ID or alias
        id: String,
    },
    /// Remove a node
    Remove {
        /// Node ID or alias
        id: String,
    },
    /// Set or show the default node for a capability
    Default {
        /// Capability (chat, image, embedding)
        capability: String,
        /// Node ID to set as default (omit to show current default)
        node_id: Option<String>,
    },
    /// Show detailed information about a node
    Show {
        /// Node ID or alias
        id: String,
    },
}

#[derive(clap::Args)]
pub struct DiscoverArgs {
    /// Discover local agents and Ollama models
    #[arg(long)]
    pub locally: bool,

    /// Discover Azure OpenAI resources
    #[arg(long)]
    pub azure: bool,

    /// Discover all available sources
    #[arg(long)]
    pub all: bool,
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
    "nodes",
    "discover",
    "completion",
    "version",
    "help",
];
