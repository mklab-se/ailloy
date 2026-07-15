# Migrating from ailloy 1.x to 2.0

This guide is for maintainers of Rust tools that depend on `ailloy`. It covers
every breaking or newly-visible change in 2.0 and what to do about it.

## 1. Message content is now an enum

`Message.content` used to be a plain `String`. It is now `MessageContent`, an
enum with two variants:

```rust
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>), // text interleaved with image/file attachments
}
```

**Constructors are unchanged** — `Message::user("hi")`, `Message::system(..)`,
`Message::assistant(..)` still take anything `Into<MessageContent>`, and a
`&str`/`String` argument still produces plain-text content. No call site that
only *constructs* messages needs to change.

**Reading `.content` as a `String` no longer compiles.** Update call sites
that read message content:

```rust
// Before (1.x)
let text: &str = &msg.content;
println!("{}", msg.content);

// After (2.0)
let text: String = msg.content.text();       // joins text parts, ignores attachments
let text: Option<&str> = msg.content.as_text(); // Some(..) only for plain-text content
println!("{}", msg.content);                  // Display still works (prints .text())
```

If you need to branch on whether a message carries attachments:

```rust
match &msg.content {
    MessageContent::Text(s) => { /* plain text, s: &String */ }
    MessageContent::Parts(parts) => {
        for part in parts {
            match part {
                ContentPart::Text { text } => { /* .. */ }
                ContentPart::Image { data, media_type } => { /* .. */ }
                ContentPart::File { data, media_type, filename } => { /* .. */ }
            }
        }
    }
}
```

Or use the convenience helpers: `msg.content.text()`, `.as_text()`,
`.has_attachments()`.

**Serialization and stored histories are unaffected.** `MessageContent` is
`#[serde(untagged)]`: a text-only message still serializes to (and
deserializes from) a bare JSON/YAML string —
`{"role":"user","content":"hello"}` — byte-for-byte identical to 1.x. Existing
on-disk conversation histories, JSON logs, or database rows load without any
migration step. Only messages that actually carry attachments serialize to
the new tagged-array shape (`[{"type":"text",...},{"type":"image",...}]`).

## 2. Deprecated APIs

All of the following still compile and work in 2.0 — they are marked
`#[deprecated]` (removal not scheduled before 3.0) so `cargo build` prints a
warning but nothing breaks.

| Deprecated | Replacement | Notes |
|---|---|---|
| `Client::generate_image_with(prompt, &opts)` | `Client::generate_images_with(prompt, &opts)` | Returns `Vec<ImageResponse>` instead of a single `ImageResponse` — some image models (gpt-image with `n > 1`) return multiple variants. The deprecated method still returns just the first one. |
| `blocking::Client::generate_image_with(prompt, &opts)` | `blocking::Client::generate_images_with(prompt, &opts)` | Same change, sync wrapper. |
| `ImageOptionsBuilder::style(..)` | `ImageOptionsBuilder::output_format` / `.background` / `.moderation` / `.input_fidelity` | `style` (`"natural"`/`"vivid"`) is a DALL·E-only hint; gpt-image models ignore it. Prefer the other builder methods, which map to gpt-image's actual parameter surface. |

```rust
// Before (1.x)
let image = client.generate_image_with(prompt, &options).await?;

// After (2.0)
let images = client.generate_images_with(prompt, &options).await?;
let image = images.into_iter().next().context("no image returned")?;
```

## 3. New capability: `video`

`Capability::Video` and the `Task::VideoGeneration` task join `Chat` /
`Image` / `Embedding`. This means:

- `defaults.video` is a valid key in `Config` (routes to a node with video
  support, same shape as `defaults.chat` / `defaults.image`).
- `Client::for_capability("video")` / `Client::for_task(Task::VideoGeneration)`
  work like they do for the other capabilities.
- `AiNode.capabilities` can include `Capability::Video`.
- New `Client::generate_video` / `generate_video_with` /
  `generate_video_with_progress`, and the `ailloy video` CLI command (see
  `ailloy video --help`).

Currently only Azure OpenAI and Microsoft Foundry nodes with a Sora
deployment support video generation; other providers return
`ClientError::Unsupported`.

## 4. Node-level default parameters (schema preview)

Each `AiNode` gained an optional `defaults` map (`node_defaults` in Rust,
serialized as `defaults:` under the node in YAML) for per-node parameter
defaults — distinct from the capability-routing `defaults:` map at the top of
`Config`:

```yaml
nodes:
  openai/gpt-image-2:
    provider: openai
    model: gpt-image-2
    capabilities: [image]
    defaults:
      image.quality: high
      image.format: png
      image.variants: "2"
```

Recognized keys in this release: `image.size`, `image.quality`,
`image.format`, `image.compression`, `image.background`, `image.variants`,
`video.size`, `video.seconds`, `video.variants`, `chat.temperature`,
`chat.max_tokens`, `embedding.dimensions` (also accepts the legacy bare
`dimensions` key).

**Note for early adopters:** the `defaults` field on `AiNode` and its YAML
shape are part of this 2.0 release, but the resolution logic that actually
fills unset `ImageOptions`/`VideoOptions`/`ChatOptions`/`EmbedOptions` fields
from these keys ships slightly later in the same 2.0 release (tracked as
Phase 4 internally). Explicit values passed to `*_with` calls always take
precedence once resolution lands — nothing here changes behavior for callers
who always pass explicit options.

## 5. Attachment support

New: `Message::user_with_attachments(text, &[PathBuf]) -> Result<Message>`
builds a user message with a text part plus one attachment part per file. The
media type is inferred from the file extension:

- Images → `ContentPart::Image`: `png`, `jpg`/`jpeg`, `gif`, `webp`
- `pdf` → `ContentPart::File` (`application/pdf`)
- Text documents → `ContentPart::File`: `txt`, `md`, `csv`, `json`, `yaml`/`yml`, `xml`, `html`

An unsupported extension or an unreadable file returns an actionable
`anyhow::Error` (extension allow-list / path in the message) rather than a
raw I/O error.

```rust
use ailloy::types::Message;

let msg = Message::user_with_attachments(
    "What's in this image?",
    &[std::path::PathBuf::from("screenshot.png")],
)?;
let response = client.chat(&[msg]).await?;
```

**CLI:** `ailloy chat` gained `--attach FILE` (repeatable). In
`-i`/`--interactive` mode, `--attach` files are attached to the first user
message only (subsequent turns in the session are plain text).

```bash
ailloy chat "What's in this image?" --attach screenshot.png
ailloy chat "Summarize these" --attach report.pdf --attach notes.txt
```

**Provider support:**

| Provider | Images | PDF | Text files |
|---|---|---|---|
| OpenAI | yes | yes | yes |
| Azure OpenAI | yes | yes | yes |
| Microsoft Foundry | yes | yes | yes |
| Anthropic | yes | yes | yes |
| Vertex AI (Gemini) | yes | yes (inline data) | yes |
| Ollama | yes | no — errors | inlined as text into the prompt |
| Local agents (claude/codex/copilot CLI) | no — errors | no — errors | no — errors |

Local agents and Ollama-with-non-text-files return
`ClientError::Unsupported`/an actionable error rather than silently dropping
the attachment — check `msg.content.has_attachments()` before routing to
those providers if you build tools that might send attachments to any
configured node.
