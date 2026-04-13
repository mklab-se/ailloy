use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ailloy",
    version,
    about = "Vendor-flexible AI for Rust tools (CLI optional)"
)]
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

    /// Generate an image from a text description
    Image(ImageArgs),

    /// Generate embeddings from text
    Embed(EmbedArgs),

    /// Manage AI configuration and providers
    Ai {
        #[command(subcommand)]
        command: Option<AiCommands>,
    },

    /// Generate shell completions
    Completion(CompletionArgs),

    /// Show version information
    Version,

    // Hidden backward-compat aliases (deprecated)
    #[command(hide = true)]
    Config(ConfigArgs),

    #[command(hide = true, subcommand)]
    Nodes(NodeCommands),

    #[command(hide = true)]
    Discover(DiscoverArgs),
}

// ---------------------------------------------------------------------------
// AI subcommands (new)
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AiCommands {
    /// Configure AI nodes and settings
    Config {
        #[command(subcommand)]
        command: Option<AiConfigCommands>,
    },

    /// Test AI connectivity
    Test {
        /// Message to send (default: "Say hello in one sentence.")
        message: Option<String>,
    },

    /// Enable AI features
    Enable,

    /// Disable AI features
    Disable,

    /// Show AI status (same as running `ailloy ai` without a subcommand)
    Status,

    /// AI agent skill information — helps set up Claude Code skills for ailloy
    Skill {
        /// Output the skill markdown content (ready to save as a skill file)
        #[arg(long)]
        emit: bool,

        /// Output detailed reference documentation for AI agents
        #[arg(long)]
        reference: bool,
    },
}

#[derive(Subcommand)]
pub enum AiConfigCommands {
    /// Add a new AI node
    AddNode,

    /// Edit an existing node
    EditNode {
        /// Node ID or alias
        id: String,
    },

    /// Delete a node
    DeleteNode {
        /// Node ID or alias
        id: String,
    },

    /// Set default node for a capability
    SetDefault {
        /// Node ID or alias
        node_name: String,
        /// Capability (chat, image)
        #[arg(long)]
        task: String,
    },

    /// List all configured nodes
    ListNodes,

    /// Show details of a specific node
    ShowNode {
        /// Node ID or alias
        id: String,
    },

    /// Show full configuration
    Show,

    /// Set a config value (dot notation: defaults.chat, nodes.openai/gpt-4o.model)
    Set {
        /// Key in dot notation
        key: String,
        /// Value to set
        value: String,
    },

    /// Get a config value (dot notation: defaults.chat, nodes.openai/gpt-4o)
    Get {
        /// Key in dot notation
        key: String,
    },

    /// Remove a config value (dot notation: defaults.chat, nodes.openai/gpt-4o)
    Unset {
        /// Key in dot notation
        key: String,
    },

    /// Reset all AI configuration
    Reset,
}

// ---------------------------------------------------------------------------
// Chat args
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Image args
// ---------------------------------------------------------------------------

#[derive(clap::Args)]
pub struct ImageArgs {
    /// Image description / prompt
    pub message: Option<String>,

    /// Node to use for image generation (overrides default)
    #[arg(short, long)]
    pub node: Option<String>,

    /// Output file path (auto-generated if omitted)
    #[arg(short, long)]
    pub output: Option<String>,

    /// Interactive mode — AI helps you describe the image
    #[arg(short, long)]
    pub interactive: bool,

    /// Image size (e.g. 1024x1024)
    #[arg(long)]
    pub size: Option<String>,

    /// Image quality (e.g. hd, standard)
    #[arg(long)]
    pub quality: Option<String>,

    /// Image style (e.g. natural, vivid)
    #[arg(long)]
    pub style: Option<String>,

    /// Raw output (no banner, no metadata)
    #[arg(long)]
    pub raw: bool,
}

// ---------------------------------------------------------------------------
// Embed args
// ---------------------------------------------------------------------------

#[derive(clap::Args)]
pub struct EmbedArgs {
    /// Text to embed
    pub text: Option<String>,

    /// Node to use for embedding (overrides default)
    #[arg(short, long)]
    pub node: Option<String>,

    /// Print the full vector as JSON
    #[arg(long)]
    pub full: bool,

    /// Show embedding node metadata
    #[arg(long)]
    pub info: bool,

    /// Print Azure AI Search vectorizer JSON for the embedding node
    #[arg(long, value_name = "NAME")]
    pub azure_vectorizer: Option<String>,
}

// ---------------------------------------------------------------------------
// Backward-compat types (deprecated)
// ---------------------------------------------------------------------------

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
    /// Set a config value
    Set { key: String, value: String },
    /// Get a config value
    Get { key: String },
    /// Remove a config value
    Unset { key: String },
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
        /// Capability (chat, image)
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

// ---------------------------------------------------------------------------
// Completions
// ---------------------------------------------------------------------------

#[derive(clap::Args)]
pub struct CompletionArgs {
    /// Shell to generate completions for
    pub shell: clap_complete::Shell,
}

/// Known subcommand names for default command pre-parsing.
pub const KNOWN_SUBCOMMANDS: &[&str] = &[
    "chat",
    "image",
    "embed",
    "ai",
    "completion",
    "version",
    "help",
    // Hidden backward-compat aliases:
    "config",
    "nodes",
    "discover",
];
