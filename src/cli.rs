use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ailloy",
    version,
    about = "Vendor-flexible AI: chat, images, video, embeddings, and evaluation from your terminal",
    after_help = "\
Examples:
  # Quick chat with the default model
  ailloy \"What is Rust?\"

  # Chat about an attached file
  ailloy chat \"Summarize this\" --attach doc.pdf

  # Generate an image / a video (sora) / an embedding vector
  ailloy image \"A sunset over mountains\" -o sunset.png
  ailloy video \"Waves on a beach\" -o waves.mp4
  ailloy embed \"some text\"

  # Configure providers (dashboard)
  ailloy ai config

Run 'ailloy <command> --help' for command-specific examples."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Increase verbosity (use -vv for trace)
    #[arg(short, long, action = clap::ArgAction::Count, global = true, display_order = 900)]
    pub verbose: u8,

    /// Suppress non-essential output
    #[arg(short, long, global = true, display_order = 901)]
    pub quiet: bool,

    /// Disable colored output
    #[arg(long, global = true, display_order = 902)]
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

    /// Ask typed questions about input with an AI judge (exit 0 pass, 1 fail, 4 unsure)
    ///
    /// Built for scripts and integration tests:
    ///   my-tool run | ailloy eval --yes-no "Does the output mention the order id?"
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
#[command(group(
    clap::ArgGroup::new("mode")
        .required(true)
        .args(["yes_no", "choice", "score", "questions"])
))]
#[command(after_help = "Examples:
  # Yes/no (exit 0 when p(yes) >= threshold, default 0.5)
  my-tool run | ailloy eval --yes-no \"Does the output mention the order id?\"
  ailloy eval \"$out\" --yes-no \"Is the tone polite?\" --threshold 0.8

  # Choice (exit 1 unless the answer is one of --expect)
  ailloy eval -f ticket.txt --choice \"Which team should handle this?\" \\
    --option billing=\"payments, refunds\" --option technical=\"bugs, outages\" --expect billing

  # Score (exit 1 when outside --min/--max)
  ailloy eval \"$reply\" --score \"How frustrated is the customer?\" \\
    --level Calm --level Frustrated --level \"Very angry\" --max 1.0

  # Many questions over one input, JSON out
  ailloy eval -f ticket.txt --questions checks.yaml --json

Exit codes: 0 pass, 1 a gate failed, 2 usage/config error, 3 provider error,
4 gates passed but an answer is below --min-confidence")]
pub struct EvalArgs {
    /// The input to evaluate (or pipe via stdin / use --file)
    pub input: Option<String>,

    /// Read the input to evaluate from a file
    #[arg(short, long)]
    pub file: Option<String>,

    /// Extra context for the judge (what produced the input, expectations)
    #[arg(long)]
    pub context: Option<String>,

    /// Judge node (defaults to defaults.eval, then the default chat node)
    #[arg(short, long, add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
    pub node: Option<String>,

    /// Print the answers as JSON
    #[arg(long)]
    pub json: bool,

    /// Ask a yes/no question
    #[arg(long, value_name = "QUESTION")]
    pub yes_no: Option<String>,

    /// What a yes means (with --yes-no)
    #[arg(long, requires = "yes_no", value_name = "TEXT")]
    pub yes_means: Option<String>,

    /// What a no means (with --yes-no)
    #[arg(long, requires = "yes_no", value_name = "TEXT")]
    pub no_means: Option<String>,

    /// Pass when p(yes) >= threshold, 0.0-1.0 (with --yes-no; default 0.5)
    #[arg(short, long, requires = "yes_no")]
    pub threshold: Option<f64>,

    /// Ask a choice question
    #[arg(long, value_name = "QUESTION")]
    pub choice: Option<String>,

    /// A choice option, `key` or `key=description` (repeatable, with --choice)
    #[arg(long, requires = "choice", value_name = "KEY[=DESC]")]
    pub option: Vec<String>,

    /// Pass only when the chosen option is one of these (repeatable, with --choice)
    #[arg(long, requires = "choice", value_name = "KEY")]
    pub expect: Vec<String>,

    /// Ask a score question
    #[arg(long, value_name = "QUESTION")]
    pub score: Option<String>,

    /// A score level, lowest first (repeatable, 2-10, with --score)
    #[arg(long, requires = "score", value_name = "TEXT")]
    pub level: Vec<String>,

    /// Fail when the score is below this (with --score)
    #[arg(long, requires = "score")]
    pub min: Option<f64>,

    /// Fail when the score is above this (with --score)
    #[arg(long, requires = "score")]
    pub max: Option<f64>,

    /// Read several questions from a YAML or JSON file
    #[arg(long, value_name = "FILE")]
    pub questions: Option<String>,

    /// Exit 4 when an answer's confidence is below this (0.0-1.0)
    #[arg(long, value_name = "F")]
    pub min_confidence: Option<f64>,
}

#[derive(clap::Args)]
#[command(after_help = "\
Examples:
  ailloy chat \"Explain lifetimes in Rust\"
  ailloy chat \"What's in this picture?\" --attach photo.jpg
  ailloy chat \"List 3 cities as JSON\" --json --raw
  ailloy chat \"Extract the order\" --schema order.schema.json
  echo \"long text here\" | ailloy chat \"Summarize the piped input\"

  # Interactive session on a specific node
  ailloy chat -i --node claude

  # -o routes by extension: .png/.jpg/.webp image, .svg vector, .mp4 video
  ailloy chat \"A rocket logo\" -o logo.svg")]
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

  # Portrait orientation at the highest quality
  ailloy image \"Fashion portrait\" -o portrait.png --quality high --size 1024x1536

  # Wide banner as compressed JPEG (format inferred from the .jpg extension)
  ailloy image \"Wide banner art\" -o banner.jpg --size 1536x1024 --compression 85

  # Three options to choose from: writes icon.png, icon-2.png, icon-3.png
  ailloy image \"3 icon options\" -o icon.png --variants 3

  # Edit or compose from reference images (--mask limits where edits apply)
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

    /// Image quality (hd/standard apply to DALL·E models only)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["low", "medium", "high", "auto", "hd", "standard"]))]
    pub quality: Option<String>,

    /// Image style — DALL·E models only, ignored by gpt-image models
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["natural", "vivid"]))]
    pub style: Option<String>,

    /// Output image format
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["png", "jpeg", "webp"]))]
    pub format: Option<String>,

    /// Compression level 0-100 (only with --format jpeg or webp)
    #[arg(long)]
    pub compression: Option<u8>,

    /// Number of image variants to generate, 1-10
    #[arg(long)]
    pub variants: Option<u8>,

    /// Background transparency (transparent requires png output)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["transparent", "opaque", "auto"]))]
    pub background: Option<String>,

    /// Content moderation strictness
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(
        ["auto", "low"]))]
    pub moderation: Option<String>,

    /// How closely edits preserve details from reference images
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

  # Landscape, 8 seconds
  ailloy video \"Logo animation\" -o logo.mp4 --size 1280x720 --seconds 8

  # Portrait / vertical
  ailloy video \"Dancer on stage\" -o dancer.mp4 --size 720x1280

  # Two takes to choose from: writes waves.mp4 and waves-2.mp4
  ailloy video \"Waves rolling in\" -o waves.mp4 --variants 2

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
  # Dimensions + vector preview; --full prints the whole vector as JSON
  ailloy embed \"text to embed\"
  ailloy embed \"text to embed\" --full

  # Show the embedding node's metadata / Azure AI Search vectorizer JSON
  ailloy embed --info
  ailloy embed --azure-vectorizer my-vectorizer")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn eval_args(argv: &[&str]) -> Result<EvalArgs, clap::Error> {
        let mut full = vec!["ailloy", "eval"];
        full.extend_from_slice(argv);
        match Cli::try_parse_from(full)?.command {
            Commands::Eval(args) => Ok(args),
            _ => panic!("not an eval command"),
        }
    }

    #[test]
    fn eval_requires_exactly_one_mode() {
        assert!(eval_args(&["text"]).is_err());
        assert!(eval_args(&["text", "--yes-no", "a?", "--score", "b?"]).is_err());
        let a = eval_args(&["text", "--yes-no", "ok?", "--threshold", "0.8"]).unwrap();
        assert_eq!(a.yes_no.as_deref(), Some("ok?"));
        assert_eq!(a.threshold, Some(0.8));
    }

    #[test]
    fn eval_choice_and_score_flags_repeat() {
        let a = eval_args(&[
            "t",
            "--choice",
            "team?",
            "--option",
            "billing=payments",
            "--option",
            "other",
            "--expect",
            "billing",
        ])
        .unwrap();
        assert_eq!(a.option, vec!["billing=payments", "other"]);
        assert_eq!(a.expect, vec!["billing"]);
        let a = eval_args(&[
            "t", "--score", "mood?", "--level", "Calm", "--level", "Angry", "--max", "1.0",
        ])
        .unwrap();
        assert_eq!(a.level.len(), 2);
        assert_eq!(a.max, Some(1.0));
    }

    #[test]
    fn eval_criteria_flag_is_gone() {
        assert!(eval_args(&["t", "--yes-no", "q", "-c", "x"]).is_err());
        assert!(eval_args(&["t", "--yes-no", "q", "--criteria", "x"]).is_err());
    }
}
