# ailloy 2.0 — Foundry image support, image parameters, video generation, multimodal chat, per-node defaults, ratatui config TUI

Date: 2026-07-15
Status: Approved by Kristofer (design review in session)

## Motivation

- `gpt-image-2` deployed on Microsoft Foundry is not recognized as an image
  model: `ProviderKind::MicrosoftFoundry` hard-codes no `image` task support
  and `FoundryClient` has no `generate_image` implementation.
- `ImageOptions` lacks the current gpt-image parameter surface (output format,
  compression, n variants, background, moderation, reference images / edits).
- No video generation support (sora-2 is deployed and job-based on Azure).
- Chat messages are text-only; current models accept image/file attachments.
- Per-node default parameter values exist only as an ad-hoc map used for
  embedding dimensions; the config TUI does not expose them.
- Dependent projects need deprecation signals on superseded APIs.

Breaking change (message content enum) ⇒ this ships as **ailloy 2.0.0**.

## Verified API surface (Microsoft Learn, July 2026)

Base: unified v1 surface `https://{resource}.services.ai.azure.com/openai/v1/`
(or `*.openai.azure.com`), auth `api-key: <key>` or `Authorization: Bearer
<entra token>`, `model` in bodies = deployment name. Matches ailloy's existing
Azure/Foundry endpoint rule (no `api_version` on node → v1 surface).

### Images (gpt-image-2 GA; gpt-image-1/1.5/1-mini limited access)

- `POST /openai/v1/images/generations` — JSON body. Params: `model`, `prompt`
  (≤32k chars), `size` (`auto` | `1024x1024` | `1536x1024` | `1024x1536`;
  gpt-image-2 additionally arbitrary WxH: multiples of 16 px, long edge ≤ 3840,
  aspect ≤ 3:1, 655,360–8,294,400 total pixels), `quality`
  (`low`/`medium`/`high`/`auto`), `n` (1–10), `output_format` (`png`/`jpeg`;
  **webp not supported on Azure**), `output_compression` (0–100, jpeg only),
  `background` (`transparent`/`opaque`/`auto`; transparent requires png),
  `moderation` (`auto`/`low`), `stream` + `partial_images` (0–3).
  `response_format` is NOT supported for gpt-image models — responses are
  always `data[].b64_json`. Response includes token `usage`.
- `POST /openai/v1/images/edits` — **multipart/form-data** (GA). Fields:
  `image[]` (1..n files, png/jpg < 50 MB), `prompt`, `model`, optional `mask`
  (png, same dims as first image, transparent = editable), `input_fidelity`
  (`high`/`low`, not on gpt-image-1-mini), plus the same size/quality/n/
  output_format/output_compression/background params. Same response shape.
- Legacy dated endpoints (`/openai/deployments/{d}/images/...?api-version=...`)
  remain for nodes with explicit `api_version`.
- dall-e-3 retired on Azure 2026-03-04 (add to retirement table).

### Video (sora-2, preview) — Azure jobs API

Chosen over the OpenAI-style `/videos` surface because it supports
`n_variants` (Foundry playground parity), a wider size set, and thumbnails;
OpenAI has deprecated its platform Videos API (shutdown 2026-09-24).

- `POST /openai/v1/video/generations/jobs?api-version=preview` — JSON
  (or multipart with input files, out of scope for v1 of this feature).
  Params: `prompt`, `model`, `width`+`height` (480x480, 854x480, 720x720,
  1280x720, 1080x1080, 1920x1080, both orientations), `n_seconds` (1–20,
  default 5), `n_variants` (1–5; 720p max 2, 1080p only 1).
- `GET .../jobs/{id}?api-version=preview` — status: `queued`/`preprocessing`/
  `running`/`processing`/`succeeded`/`failed`/`cancelled`; `generations[]`
  each with an id; `failure_reason`; artifacts expire after ~24 h.
- `GET /openai/v1/video/generations/{gen-id}/content/video?api-version=preview`
  → `video/mp4` bytes. Thumbnail variant available.
- `DELETE .../jobs/{id}?api-version=preview`.

## Design

### 1. Capability model & routing fix

- `Capability::Video`, `Task::VideoGeneration` (config key `video`), added to
  `ALL_CAPABILITIES`, capability parsing, labels.
- `ProviderKind::supports_task`: add `(MicrosoftFoundry, "image")`,
  `(MicrosoftFoundry | AzureOpenAi, "video")`.
- `azure_discover.rs`: tag discovered `gpt-image-*` deployments with Image,
  `sora-*` with Video, in the wizard's capability pre-selection.
- `defaults.video` routes the default video node; `Client::for_capability`
  and CLI node resolution work unchanged once the capability exists.

### 2. Image options & multi-image results

`ImageOptions` (existing struct, additive fields + builder methods):

```rust
pub struct ImageOptions {
    pub size: Option<(u32, u32)>,          // existing
    pub quality: Option<String>,           // existing (low/medium/high/auto)
    pub style: Option<String>,             // existing; DALL·E-only, documented
    pub output_format: Option<ImageFormat>,// png | jpeg | webp
    pub compression: Option<u8>,           // 0–100, jpeg/webp only
    pub n: Option<u8>,                     // 1–10
    pub background: Option<Background>,    // Transparent | Opaque | Auto
    pub moderation: Option<Moderation>,    // Auto | Low
    pub input_fidelity: Option<Fidelity>,  // High | Low (edits only)
    pub reference_images: Vec<PathBuf>,    // non-empty → edits endpoint
    pub mask: Option<PathBuf>,             // edits only
}
```

- Validation happens client-side with actionable errors: webp on
  Azure/Foundry rejected; compression without jpeg/webp rejected; mask or
  input_fidelity without reference images rejected; n out of range rejected.
- Provider trait: new `generate_images(&self, prompt, options)
  -> Result<Vec<ImageResponse>>` (default `Unsupported`). Existing
  `generate_image` becomes a default-method wrapper returning the first image.
- `Client::generate_images` / `generate_images_with` added (and blocking
  equivalents). `Client::generate_image_with` + `blocking` counterpart marked
  `#[deprecated(since = "2.0.0", note = "use generate_images_with; models can
  return multiple variants")]`. `generate_image` (no options) stays
  undeprecated as the simple-case convenience.
- `ImageResponse` unchanged (per-image); multi-image = `Vec<ImageResponse>`.
  Response `usage` added as `Option<Usage>` on `ImageResponse`.
- Implementation: shared internal `openai_images` request-builder module used
  by OpenAI, Azure, and (new) Foundry clients — builds JSON generations body
  or multipart edits form (reqwest `multipart::Form`, reading reference/mask
  files async). Azure's current unconditional `response_format: "b64_json"`
  is dropped for gpt-image models (kept for dall-e on OpenAI only).
- Foundry: `image_url()`/`edits_url()` following the existing v1-vs-dated
  pattern; full `generate_images` implementation.
- CLI `ailloy image`: new flags `--format png|jpeg|webp`, `--compression N`,
  `--variants N` (writes `name.png`, `name-2.png`, …), `--background ...`,
  `--ref FILE` (repeatable), `--mask FILE`, `--fidelity high|low`.
  Existing `--size/--quality/--style` unchanged.

### 3. Video generation

New types in `types.rs`:

```rust
pub struct VideoOptions { pub size: Option<(u32,u32)>, pub seconds: Option<u32>,
                          pub variants: Option<u8> }        // + builder
pub enum VideoJobStatus { Queued, Preprocessing, Running, Processing,
                          Succeeded, Failed, Cancelled }
pub struct VideoJob { pub id: String, pub status: VideoJobStatus,
                      pub generation_ids: Vec<String>,
                      pub failure_reason: Option<String> }
pub struct VideoResponse { pub data: Vec<u8>, pub width: u32, pub height: u32,
                           pub duration_seconds: u32 }       // format: MP4
pub type ProgressFn = Box<dyn Fn(&VideoJob) + Send + Sync>;
```

Provider trait additions (all default `Unsupported`):

- `generate_video(prompt, options, on_progress: Option<&ProgressFn>)
  -> Result<Vec<VideoResponse>>` — create job, poll (2 s → 10 s backoff,
  overall timeout 15 min), download all generations.
- `create_video_job(prompt, options) -> Result<VideoJob>`
- `get_video_job(id) -> Result<VideoJob>`
- `download_video(generation_id) -> Result<VideoResponse>`
- `delete_video_job(id) -> Result<()>`

Implemented for Azure + Foundry via the jobs API. On the v1 surface the jobs
endpoints append `?api-version=preview`; nodes with explicit dated
`api_version` use it instead. `Client` + `blocking::Client` expose all five.

CLI: new `ailloy video` subcommand — `ailloy video "prompt" [-o clip.mp4]
[--size 1280x720] [--seconds 8] [--variants 2] [-n node] [--raw]` with
progress spinner showing job status; multiple variants write `clip.mp4`,
`clip-2.mp4`, …. `ailloy chat "..." -o out.mp4` routes to video generation
(same pattern as image routing). Job expiry (24 h) noted in `--help`.

### 4. Multimodal chat messages

```rust
pub struct Message { pub role: Role, pub content: MessageContent }

#[serde(untagged)]
pub enum MessageContent { Text(String), Parts(Vec<ContentPart>) }

pub enum ContentPart {
    Text { text: String },
    Image { data: Vec<u8>, media_type: String },   // sent as data: URI / block
    File  { data: Vec<u8>, media_type: String, filename: String },
}
```

- Untagged serde: plain strings round-trip exactly as today (stored
  histories, YAML fixtures, wire formats keep working).
- `Message::user/system/assistant(impl Into<MessageContent>)`;
  `From<String>`/`From<&str>` for `MessageContent`; `Message::user_with_attachments
  (text, files: &[PathBuf]) -> Result<Message>` reads files, infers media type
  from extension (png/jpeg/webp/gif images; pdf + text-like files as File).
- `MessageContent::as_text() -> Option<&str>` and `text() -> String`
  (concatenated text parts) ease migration; MIGRATION.md documents the
  pattern. This is the 2.0 breaking change.
- Provider mapping: OpenAI/Azure/Foundry chat → content-part arrays
  (`image_url` with data URI, `file` with base64); Anthropic → `image` /
  `document` content blocks; Ollama → `images` array; Vertex → `inline_data`
  parts; local agents → error `Unsupported("attachments")` with actionable
  message. Providers that get plain text messages serialize exactly as before.
- CLI: `ailloy chat --attach FILE` (repeatable), works in single-shot,
  stdin, and interactive modes.
- gpt-5.6-luna: no code expected beyond the existing sampling-rejection
  retry; verified live (chat, stream, JSON schema, attachment) in Phase 4
  testing. Any incompatibility found is fixed within this phase.

### 5. Per-node defaults & parameter registry

- `AiNode.node_defaults` (existing `defaults:` map, `BTreeMap<String,String>`)
  becomes capability-scoped: keys `image.size`, `image.quality`,
  `image.format`, `image.compression`, `video.size`, `video.seconds`,
  `video.variants`, `chat.temperature`, `chat.max_tokens`,
  `embedding.dimensions` (existing key kept as alias, read both).
- New `params.rs` (library, no feature gate): static parameter registry —
  for each (capability, param): key, label, type (enum/int/size/float),
  allowed values or range, provider applicability, default. Drives TUI
  editing, CLI validation, and defaults parsing; single source of truth.
- Resolution order (implemented in `Client::from_config`/`with_node`/
  `for_capability` construction path, so library consumers get it too):
  explicit options > node defaults > provider defaults. Explicit options
  structs stay pure; merging happens where the client knows its node.

### 6. ratatui config TUI

Replace the interactive parts of `config_tui.rs` with a full-screen ratatui
app (new dependency `ratatui`, behind the existing `config-tui` feature;
`inquire` dropped from the interactive path, `crossterm` stays as ratatui's
backend). Non-TTY invocations fall back to printing status (as today).

Layout:

- **Left pane — node table**: rows = nodes; columns = id/alias, provider,
  and a capability matrix (chat ✓, image ✓, embed ✓, video ✓) with a star
  marking capability defaults; retirement warnings inline (from
  `retirement.rs`).
- **Right pane — detail**: selected node's connection info (endpoint,
  deployment, auth kind), capabilities as toggles, and a **Defaults** section
  listing registry parameters valid for this node's provider+capabilities
  with current values; enum params edited via select popup, numeric/size via
  validated input popup. Shows "not configurable for this provider" hints —
  making what can/can't be configured explicit.
- **Keys**: ↑/↓ select node, Tab switch pane, Enter edit, `a` add node
  (form-based flow rendered in ratatui, including Azure/Foundry discovery
  consent + pick lists), `e` edit node, `x` delete (confirm), `d` set as
  default for a chosen capability, `k` set keychain key, `t` test node,
  `q`/Esc quit. Footer shows the key legend.
- Consent flows (azure-cli/gcloud-cli) render as modal confirms; consents
  remain global-only.
- Non-interactive commands (`ailloy ai config show/set/get/unset`, add-node
  etc. via flags) are unchanged; `config_tui.rs` shrinks to the ratatui app +
  status printing; discovery stays in `azure_discover.rs`.

### 7. Versioning, deprecation, docs

- Version 2.0.0. CHANGELOG entry + **MIGRATION.md** (1.x → 2.0: MessageContent
  match patterns, deprecated image methods, new capability key).
- `#[deprecated(since = "2.0.0")]`: `Client::generate_image_with`,
  `blocking::Client::generate_image_with`, `ImageOptionsBuilder::style`
  (note: DALL·E-only). Deprecated CLI aliases unchanged.
- Retirement table: add dall-e-3 (Azure, 2026-03-04).
- Docs updated in the same phases: README, INSTALL (nothing new), CHANGELOG,
  CLAUDE.md architecture, `doc/ai-reference.md`, `ailloy ai skill` reference
  output, examples (`chat.rs` gains attachment sample; new `video.rs`
  example; `configure.rs` gains node_defaults sample).

## Implementation phases (one release)

1. **Capabilities & image params**: §1 + §2 (+ retirement entry). Unit tests:
   supports_task matrix, options validation, JSON/multipart request builders,
   response parsing (incl. usage), CLI flag plumbing.
2. **Video**: §3. Unit tests: job lifecycle parsing, URL building (v1 vs
   dated), poll/backoff logic (mocked), CLI routing for `.mp4`.
3. **Multimodal chat**: §4 + MIGRATION.md. Unit tests: untagged serde
   round-trips (string ⇄ parts), per-provider request mapping, media-type
   inference, CLI --attach.
4. **Defaults & registry**: §5. Unit tests: resolution order, registry
   validation, legacy `dimensions` alias.
5. **ratatui TUI**: §6. Unit tests for state/reducer logic (selection,
   edit buffers, validation); manual TTY walkthrough.
6. **Release prep**: §7 docs, full CI (`fmt`, `clippy -D warnings`, `test`),
   live smoke tests (cost-conscious): one gpt-image-2 generation (low,
   1024x1024), one edits call with a small reference image, one sora-2 job
   (480x480, 2 s, 1 variant), gpt-5.6-luna chat/stream/schema/attachment.

## Error handling

All new failure paths follow the house rule (actionable messages): missing
video capability → "node X has no video capability — run 'ailloy ai config'";
job failed → include `failure_reason` and job id; job expired/404 → mention
24 h retention; webp on Azure → say png/jpeg are supported; attachment on
non-multimodal provider → name the provider and suggest a capable node.

## Out of scope (explicitly)

- Streaming image generation (`partial_images`) and image job API.
- Video-jobs multipart inputs (`files`, `inpaint_items`) and thumbnails;
  sora remix; OpenAI-platform `/videos` (deprecated by OpenAI).
- Transcription capability (Task::Transcription exists but remains unrouted).
- Vertex/Ollama video; Anthropic image generation.
