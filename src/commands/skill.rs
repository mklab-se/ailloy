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
description: Vendor-flexible AI integration CLI — configure and use multiple AI providers (OpenAI, Anthropic, Azure, Ollama, etc.) for chat and image generation.
---

# ailloy — Vendor-Flexible AI CLI

Use ailloy when the user needs to configure AI providers, chat with AI models,
generate images, or manage multi-provider AI setups.

## Getting current documentation

Run this command to get full, up-to-date reference documentation:

```bash
ailloy ai skill --reference
```

Read the output carefully — it covers every command, provider type,
configuration format, node management, and common workflows.

## Quick command reference

- `ailloy "message"` — send a message (shorthand for `ailloy chat`)
- `ailloy chat "message"` — chat with configured AI
- `ailloy chat -i` — interactive conversation
- `ailloy chat --stream "message"` — stream response
- `ailloy image "description"` — generate an image
- `ailloy ai status` — show AI configuration status
- `ailloy ai config` — interactive configuration wizard
- `ailloy ai test` — test AI connectivity
- `ailloy ai config add-node` — add a new provider node
- `ailloy ai config list-nodes` — list all configured nodes
- `ailloy ai config set-default NODE --task chat` — set default
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
