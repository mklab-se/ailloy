use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ailloy",
    version,
    about = "Vendor-flexible AI — chat, images, video, and embeddings from your terminal",
    after_help = "\
Examples:
  ailloy \"What is Rust?\"                            Quick chat with the default model
  ailloy chat \"Summarize this\" --attach doc.pdf     Chat about an attached file
  ailloy image \"A sunset over mountains\" -o s.png   Generate an image
  ailloy video \"Waves on a beach\" -o waves.mp4      Generate a video (sora)
  ailloy embed \"some text\"                          Create an embedding vector
  ailloy ai config                                  Configure providers (dashboard)

Run 'ailloy <command> --help' for command-specific examples."
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

    /// Generate a video from a text description
    Video(VideoArgs),

    /// Generate embeddings from text
    Embed(EmbedArgs),

    /// Evaluate input against criteria with an AI judge (exit 0 pass, 1 fail)
    ///
    /// Built for scripts and integration tests:
    ///   my-tool run | ailloy eval --criteria "output mentions the order id"
    Eval(EvalArgs),

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

        /// Test every configured node (chat and embedding pings)
        #[arg(long)]
        all: bool,
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
        #[arg(add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
        id: String,
    },

    /// Delete a node
    DeleteNode {
        /// Node ID or alias
        #[arg(add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
        id: String,
    },

    /// Write a starter .ailloy.yaml in the current directory (folder-local config)
    InitLocal {
        /// Inherit the machine-wide config instead of replacing it (extends: global)
        #[arg(long)]
        extends_global: bool,
    },

    /// Store a node's API key in the OS keychain (and switch its auth to keychain)
    SetKey {
        /// Node ID or alias
        #[arg(add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
        id: String,
    },

    /// Set default node for a capability
    SetDefault {
        /// Node ID or alias
        #[arg(add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
        node_name: String,
        /// Capability (chat, image)
        #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
            ["chat", "image", "video", "embedding"]))]
        task: String,
    },

    /// List all configured nodes
    ListNodes,

    /// Show details of a specific node
    ShowNode {
        /// Node ID or alias
        #[arg(add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
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
#[command(after_help = "\
Examples:
  my-tool run | ailloy eval -c \"output mentions the order id\"    Exit 0 pass, 1 fail
  ailloy eval \"$output\" -c \"polite, professional English\" --threshold 0.8 --json
  ailloy eval -f report.txt --criteria-file checks.txt --context \"generated by my-tool\"")]
pub struct EvalArgs {
    /// The input to evaluate (or pipe via stdin / use --file)
    pub input: Option<String>,

    /// Criteria the input must satisfy
    #[arg(short, long)]
    pub criteria: Option<String>,

    /// Read criteria from a file
    #[arg(long, conflicts_with = "criteria")]
    pub criteria_file: Option<String>,

    /// Read the input to evaluate from a file
    #[arg(short, long)]
    pub file: Option<String>,

    /// Extra context for the judge (what produced the input, expectations)
    #[arg(long)]
    pub context: Option<String>,

    /// Judge node (defaults to the default chat node)
    #[arg(short, long, add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
    pub node: Option<String>,

    /// Pass when score >= threshold (0.0-1.0) instead of the judge's verdict
    #[arg(short, long)]
    pub threshold: Option<f32>,

    /// Print the verdict as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
#[command(after_help = "\
Examples:
  ailloy chat \"Explain lifetimes in Rust\"
  ailloy chat \"What's in this picture?\" --attach photo.jpg
  ailloy chat \"List 3 cities as JSON\" --json --raw
  ailloy chat \"Extract the order\" --schema order.schema.json
  echo \"long text here\" | ailloy chat \"Summarize the piped input\"
  ailloy chat -i --node claude                     Interactive session on a specific node
  ailloy chat \"A rocket logo\" -o logo.svg          -o routes: .png/.jpg image, .svg, .mp4 video")]
pub struct ChatArgs {
    /// The message to send (optional if piped via stdin or using -i)
    pub message: Option<String>,

    /// Node to use (overrides default, accepts ID or alias)
    #[arg(short, long, add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
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

    /// Force the response to be a single JSON object (script-friendly)
    #[arg(long)]
    pub json: bool,

    /// Force the response to match a JSON Schema file (implies --json)
    #[arg(long, value_name = "FILE")]
    pub schema: Option<String>,

    /// Save response to file (.png/.jpg/.webp → image generation, .svg → SVG, .mp4 → video)
    #[arg(short, long)]
    pub output: Option<String>,

    /// Interactive conversation mode
    #[arg(short, long)]
    pub interactive: bool,

    /// Output only the raw model response (no newline, no metadata, no color)
    #[arg(long)]
    pub raw: bool,

    /// Attach a file (image, pdf, or text) — repeatable
    #[arg(long = "attach", value_name = "FILE")]
    pub attach: Vec<String>,
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

#[derive(clap::Args, Default)]
#[command(after_help = "\
Examples:
  ailloy image \"A sunset over mountains\"
  ailloy image \"Fashion portrait\" -o p.png --quality high --size 1024x1536    Portrait, best quality
  ailloy image \"Wide banner art\" -o banner.jpg --size 1536x1024 --compression 85
  ailloy image \"3 icon options\" -o icon.png --variants 3     Writes icon.png, icon-2.png, icon-3.png
  ailloy image \"same scene at night\" --ref day.png -o night.png
  ailloy image \"replace the sky\" --ref photo.png --mask sky-mask.png -o out.png

Per-node defaults (image.quality, image.format, ...) apply when a flag is omitted;
set them in 'ailloy ai config' (Detail pane → Enter).")]
pub struct ImageArgs {
    /// Image description / prompt
    pub message: Option<String>,

    /// Node to use for image generation (overrides default)
    #[arg(short, long, add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
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

    /// Image quality: low, medium, high, auto (DALL·E models: hd, standard)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["low", "medium", "high", "auto", "hd", "standard"]))]
    pub quality: Option<String>,

    /// Image style (e.g. natural, vivid)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["natural", "vivid"]))]
    pub style: Option<String>,

    /// Output image format (png, jpeg, webp)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["png", "jpeg", "webp"]))]
    pub format: Option<String>,

    /// Compression level 0-100 (only with --format jpeg or webp)
    #[arg(long)]
    pub compression: Option<u8>,

    /// Number of image variants to generate, 1-10
    #[arg(long)]
    pub variants: Option<u8>,

    /// Background transparency (transparent, opaque, auto)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["transparent", "opaque", "auto"]))]
    pub background: Option<String>,

    /// Content moderation strictness (auto, low)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["auto", "low"]))]
    pub moderation: Option<String>,

    /// How closely edits should preserve details from reference images (high, low)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["high", "low"]))]
    pub fidelity: Option<String>,

    /// Reference image to edit/compose from (repeatable); using this switches
    /// to the edits endpoint and drives the generation from these images
    #[arg(long = "ref", value_name = "FILE")]
    pub reference: Vec<String>,

    /// Mask image for inpainting (requires at least one --ref image)
    #[arg(long, value_name = "FILE")]
    pub mask: Option<String>,

    /// Raw output (no banner, no metadata)
    #[arg(long)]
    pub raw: bool,
}

// ---------------------------------------------------------------------------
// Video args
// ---------------------------------------------------------------------------

#[derive(clap::Args, Default)]
#[command(after_help = "\
Examples:
  ailloy video \"A drone shot over a coastal cliff at sunrise\"
  ailloy video \"Logo animation\" -o logo.mp4 --size 1280x720 --seconds 8    Landscape, 8 seconds
  ailloy video \"Dancer on stage\" -o d.mp4 --size 720x1280                  Portrait / vertical
  ailloy video \"Two takes of waves\" -o waves.mp4 --variants 2   Writes waves.mp4, waves-2.mp4

Needs an Azure OpenAI or Microsoft Foundry node with a sora deployment
(video capability). Generation is asynchronous and can take a few minutes;
progress is shown as the job status changes. Results expire server-side ~24h.")]
pub struct VideoArgs {
    /// Video description / prompt
    pub message: Option<String>,

    /// Node to use for video generation (overrides default)
    #[arg(short, long, add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
    pub node: Option<String>,

    /// Output file path (auto-generated if omitted)
    #[arg(short, long)]
    pub output: Option<String>,

    /// Video size, WxH (e.g. 720x1280 or 1280x720; model-dependent)
    #[arg(long)]
    pub size: Option<String>,

    /// Clip duration in seconds (typically 4, 8, or 12 — model-dependent)
    #[arg(long)]
    pub seconds: Option<u32>,

    /// Number of video variants (1-5; each is a separate video creation)
    #[arg(long)]
    pub variants: Option<u8>,

    /// Raw output (no banner, no metadata)
    #[arg(long)]
    pub raw: bool,
}

// ---------------------------------------------------------------------------
// Embed args
// ---------------------------------------------------------------------------

#[derive(clap::Args)]
#[command(after_help = "\
Examples:
  ailloy embed \"text to embed\"                    Dimensions + vector preview
  ailloy embed \"text to embed\" --full             Full vector as JSON
  ailloy embed --info                             Show the embedding node's metadata
  ailloy embed --azure-vectorizer my-vectorizer   Azure AI Search vectorizer JSON")]
pub struct EmbedArgs {
    /// Text to embed
    pub text: Option<String>,

    /// Node to use for embedding (overrides default)
    #[arg(short, long, add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
    pub node: Option<String>,

    /// Print the full vector as JSON
    #[arg(long)]
    pub full: bool,

    /// Show embedding node metadata
    #[arg(long, conflicts_with = "text")]
    pub info: bool,

    /// Print Azure AI Search vectorizer JSON for the embedding node
    #[arg(long, value_name = "NAME", conflicts_with = "text")]
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
#[command(after_help = "\
This generates STATIC completions (commands, flags, and known flag values).

For DYNAMIC completion that also completes --node and node-id arguments from
your configured nodes, register ailloy's built-in completer instead:
  zsh:   echo 'source <(COMPLETE=zsh ailloy)'  >> ~/.zshrc
  bash:  echo 'source <(COMPLETE=bash ailloy)' >> ~/.bashrc
  fish:  echo 'COMPLETE=fish ailloy | source'  >> ~/.config/fish/completions/ailloy.fish

Reload your shell afterwards. See INSTALL.md for details.")]
pub struct CompletionArgs {
    /// Shell to generate completions for
    pub shell: clap_complete::Shell,
}

// ---------------------------------------------------------------------------
// Dynamic completion: node ids + aliases
// ---------------------------------------------------------------------------

use clap_complete::engine::CompletionCandidate;

/// Build completion candidates for node identifiers from a loaded config.
///
/// Emits one candidate per node id (help = provider + model/deployment/binary
/// detail) and one per alias (help = "alias for <id>"). Sorted by candidate
/// value for deterministic output. Kept separate from the loader so it can be
/// unit-tested without touching the filesystem or environment.
pub(crate) fn candidates_from(config: &ailloy::config::Config) -> Vec<CompletionCandidate> {
    let mut out: Vec<CompletionCandidate> = Vec::new();
    for (id, node) in &config.nodes {
        let detail = node
            .model
            .as_deref()
            .or(node.deployment.as_deref())
            .or(node.binary.as_deref());
        let help = match detail {
            Some(d) => format!("{} — {}", node.provider, d),
            None => node.provider.to_string(),
        };
        out.push(CompletionCandidate::new(id.clone()).help(Some(help.into())));
        if let Some(alias) = node.alias.as_deref() {
            out.push(
                CompletionCandidate::new(alias.to_string())
                    .help(Some(format!("alias for {}", id).into())),
            );
        }
    }
    out.sort_by(|a, b| a.get_value().cmp(b.get_value()));
    out
}

/// Completer for `--node`/node-id arguments: reads the merged local+global
/// config and returns node id and alias candidates. Never panics or prints —
/// on any load error it yields no candidates (completion stays silent).
pub(crate) fn complete_node_ids() -> Vec<CompletionCandidate> {
    match ailloy::config::Config::load() {
        Ok(config) => candidates_from(&config),
        Err(_) => Vec::new(),
    }
}

/// Known subcommand names for default command pre-parsing.
pub const KNOWN_SUBCOMMANDS: &[&str] = &[
    "chat",
    "image",
    "video",
    "embed",
    "eval",
    "ai",
    "completion",
    "version",
    "help",
    // Hidden backward-compat aliases:
    "config",
    "nodes",
    "discover",
];

#[cfg(test)]
mod completion_tests {
    use super::*;
    use ailloy::config::{AiNode, Config, ProviderKind};

    fn help_of(c: &CompletionCandidate) -> Option<String> {
        c.get_help().map(|s| s.to_string())
    }

    fn value_of(c: &CompletionCandidate) -> String {
        c.get_value().to_string_lossy().into_owned()
    }

    #[test]
    fn candidates_are_sorted_and_include_aliases_with_help() {
        let mut config = Config::default();

        let mut openai = AiNode::new(ProviderKind::OpenAi);
        openai.model = Some("gpt-5.4-mini".to_string());
        openai.alias = Some("mini".to_string());
        config
            .nodes
            .insert("openai/gpt-5.4-mini".to_string(), openai);

        let mut foundry = AiNode::new(ProviderKind::MicrosoftFoundry);
        foundry.deployment = Some("gpt-image-2".to_string());
        config
            .nodes
            .insert("microsoft-foundry/gpt-image-2".to_string(), foundry);

        let cands = candidates_from(&config);
        let values: Vec<String> = cands.iter().map(value_of).collect();

        // Two node ids + one alias, sorted by value.
        assert_eq!(
            values,
            vec![
                "microsoft-foundry/gpt-image-2".to_string(),
                "mini".to_string(),
                "openai/gpt-5.4-mini".to_string(),
            ]
        );

        // Node id help = "<provider> — <detail>".
        let id_cand = cands
            .iter()
            .find(|c| value_of(c) == "openai/gpt-5.4-mini")
            .unwrap();
        assert_eq!(help_of(id_cand).as_deref(), Some("openai — gpt-5.4-mini"));

        // Deployment used as detail when model is absent.
        let dep_cand = cands
            .iter()
            .find(|c| value_of(c) == "microsoft-foundry/gpt-image-2")
            .unwrap();
        assert_eq!(
            help_of(dep_cand).as_deref(),
            Some("microsoft-foundry — gpt-image-2")
        );

        // Alias help points back to the id.
        let alias_cand = cands.iter().find(|c| value_of(c) == "mini").unwrap();
        assert_eq!(
            help_of(alias_cand).as_deref(),
            Some("alias for openai/gpt-5.4-mini")
        );
    }

    #[test]
    fn empty_config_yields_no_candidates() {
        assert!(candidates_from(&Config::default()).is_empty());
    }
}
