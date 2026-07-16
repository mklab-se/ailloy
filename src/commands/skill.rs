const GUIDE: &str = "\
ailloy AI Skill Setup
=====================

ailloy is a vendor-flexible AI integration library and CLI. A skill helps
AI agents configure and use AI providers through ailloy.

To create the skill file, run:

  ailloy ai skill --emit > ~/.claude/skills/ailloy.md

Or ask your AI agent:

  \"Use `ailloy ai skill --emit` to set up a skill for managing AI providers\"

The skill instructs the AI agent to run `ailloy ai skill --reference` at
runtime to fetch full documentation, so the agent always has up-to-date
command details without bloating the skill file itself.
";

const SKILL_MARKDOWN: &str = r#"---
name: ailloy
description: Vendor-flexible AI integration CLI — configure and use multiple AI providers (OpenAI, Anthropic, Azure, Foundry, Ollama, etc.) for chat, image generation, video generation, and embeddings, with file attachments, structured JSON output, and per-node defaults.
---

# ailloy — Vendor-Flexible AI CLI

Use ailloy when the user needs to chat with AI models (optionally with file
attachments or structured JSON output), generate images or videos, create
embeddings, judge text against criteria, or configure multi-provider AI setups.

## Getting current documentation

Run this command to get full, up-to-date reference documentation:

```bash
ailloy ai skill --reference
```

Read the output carefully — it covers every command and flag, provider types,
configuration format, per-node default parameters, and common workflows.

## Quick command reference

Chat (text in, text out):

- `ailloy "message"` — send a message (shorthand for `ailloy chat`)
- `ailloy chat "message" --raw` — script-friendly: only the model's reply, no metadata
- `ailloy chat "message" --json` — force a single JSON object reply
- `ailloy chat "extract X" --schema file.json` — reply must match a JSON Schema (strict)
- `ailloy chat "message" --attach FILE` — attach an image/pdf/text file (repeatable)
- `ailloy chat --stream "message"` — stream response; `-i` for interactive mode
- `echo "text" | ailloy chat` — reads piped stdin
- `... --node ID-or-alias` — any command: use a specific configured node

Generation:

- `ailloy image "description" -o out.png` — generate an image
  (`--size WxH --quality low|medium|high|auto --format png|jpeg --compression 0-100
  --variants 1-10 --background transparent|opaque|auto --ref FILE --mask FILE`;
  `--ref` edits/composes from reference images)
- `ailloy video "description" -o out.mp4` — generate a video
  (`--size 1280x720 --seconds 4|8|12 --variants 1-5`; needs an Azure/Foundry sora node;
  takes minutes, polls automatically)
- `ailloy embed "text"` — embedding vector (`--full` prints the whole vector as JSON)

Judge / test:

- `cmd | ailloy eval -c "criteria"` — LLM-as-judge; exit 0 pass, 1 fail (`--json` for verdict)
- `ailloy ai test --all` — ping every configured node; exit 1 if any fails

Configuration:

- `ailloy ai status` — show configured defaults per capability (chat, image, video, embedding)
- `ailloy ai config` — full-screen node configuration dashboard (TTY)
- `ailloy ai config list-nodes` / `show-node ID` — inspect nodes non-interactively
- `ailloy ai config set-default NODE --task chat|image|video|embedding` — set defaults

Nodes can carry per-node default parameters (e.g. `image.quality`,
`video.seconds`, `chat.temperature`) under a `defaults:` map; explicit flags
always win. See `ailloy ai skill --reference` for the full key list.
"#;

const REFERENCE: &str = include_str!("../doc/ai-reference.md");

pub fn run(emit: bool, reference: bool) {
    if emit {
        print!("{SKILL_MARKDOWN}");
    } else if reference {
        print!("{REFERENCE}");
    } else {
        print!("{GUIDE}");
    }
}
