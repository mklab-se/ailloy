# Eval capability and TypeSafe provider: design

Date: 2026-10-06
Status: approved in conversation, pending written-spec review
Target release: ailloy 3.0.0 (breaking)

## Goal

Add a new AI capability, `eval`, for typed judgments over text: a yes/no
probability, a choice among options, or a score on ordered levels. Any chat
model can serve it (with self-reported probabilities), and TypeSafe's Jev model
serves it natively (with calibrated probabilities). The existing `ailloy eval`
CLI command is rebuilt on top of this capability.

The three question types mirror TypeSafe's System One primitives (Noul, Choice,
Score), so TypeSafe's documentation and cookbooks transfer to ailloy with a
single rename (Noul is called `YesNo` in ailloy).

## Decisions made

| Decision | Choice |
|---|---|
| Capability name | `eval` (config key), `Capability::Eval`, `Task::Evaluation` |
| Yes/no type name | `YesNo` (wire value `noul` for TypeSafe) |
| Chat-model backend | Structured JSON output with model-reported probabilities, flagged `Calibration::SelfReported`. No logprobs. |
| Library architecture | New `Provider::evaluate` method whose default implementation emulates eval over `chat`; the TypeSafe provider overrides it natively |
| CLI shape | Inline flags for one question of any type, plus a `--questions` file for batches |
| Compatibility | Breaking changes accepted for both library and CLI; released as 3.0.0 |

## 1. Library types and API

New public types (in `src/types.rs`, or a new `src/eval.rs` re-exported from
`types` if the code grows large):

```rust
#[non_exhaustive]
pub enum Question {
    YesNo  { instructions: Value, criteria: Option<YesNoCriteria> },
    Choice { instructions: Value, options: BTreeMap<String, Option<Value>> },
    Score  { instructions: Value, levels: Vec<Value> },
}

pub struct YesNoCriteria { pub yes: Option<Value>, pub no: Option<Value> }

#[non_exhaustive]
pub enum Answer {
    YesNo  { probability: f64 },
    Choice { choice: String, probabilities: BTreeMap<String, f64>, confidence: f64 },
    Score  { score: f64, probabilities: Vec<f64>, confidence: f64 },
}

pub enum Calibration { Measured, SelfReported }

pub struct EvalResponse {
    pub answers: BTreeMap<String, Answer>,
    pub model: String,
    pub usage: Option<Usage>,
    pub calibration: Calibration,
    /// Per-question rationale; filled by chat backends, empty for Jev.
    pub rationale: BTreeMap<String, String>,
}
```

`Value` is `serde_json::Value`, so instructions, option descriptions and level
descriptions can be plain strings or structured JSON, matching TypeSafe's API.

Entry points on `Client` (mirrored on `blocking::Client`):

- `client.eval(state, &questions) -> Result<EvalResponse>` where `state: impl
  Into<serde_json::Value>` and `questions: &BTreeMap<String, Question>`.
- `client.eval_one(state, question) -> Result<Answer>` convenience wrapper.

Builders:

- `Question::yes_no(instructions)` with `.yes(meaning)` / `.no(meaning)`
- `Question::choice(instructions)` with `.option(key, description)` /
  `.option_bare(key)`
- `Question::score(instructions)` with `.levels(iter)` / `.level(text)`

Validation, client side, with actionable errors naming the question ID:

- Choice: 2 to 255 options, no empty keys.
- Score: 2 to 10 levels.
- Instructions must not be empty.
- At least one question per request.

Answer helpers:

- `Answer::confidence() -> f64` for all three types. For YesNo it is
  `|p - 0.5| * 2` (the two-option case of the Choice formula), so code can gate
  uniformly.
- `Answer::as_yes_no()`, `as_choice()`, `as_score()` accessors returning
  `Option<...>`.

Confidence formulas (used by chat emulation; TypeSafe returns its own values),
taken from TypeSafe's Confidence documentation so both backends are comparable:

- Choice with n options and peak probability `peak`:
  `clamp((n * peak - 1) / (n - 1), 0, 1)`.
- Score with level probabilities `p` and peak index `k`:
  `spread = sum_i p_i * |i - k|`,
  `evenSpread = (sum_i |i - (n - 1) / 2|) / n`,
  confidence `clamp(1 - spread / evenSpread, 0, 1)`.
- YesNo: `|p - 0.5| * 2`.

## 2. Providers, config and routing

### TypeSafe provider (`src/typesafe.rs`)

- `ProviderKind::TypeSafe`, serialized as `typesafe`.
- Node ID `typesafe/<model>`; `model` defaults to `jev-latest`. Users can pin a
  versioned ID such as `jev-1.13.0` when they have tuned thresholds.
- Auth: `env` (`TYPESAFE_API_KEY`), `api_key`, `keychain`.
- Endpoint defaults to `https://api.typesafe.ai`; a node's `endpoint` overrides it.
- `supported_capabilities()` returns `[Eval]` only. Chat, image, embedding and
  video return `ClientError::Unsupported` with a message pointing to a chat node.
- `evaluate()` sends one `POST {endpoint}/v1/systemone` with
  `{state, model, questions}`. `YesNo` maps to `{"type": "noul",
  "instructions", "criteria": {"true", "false"}}`; Choice criteria map to the
  options map; Score criteria to the levels array. Answers map back: `noul` to
  `Answer::YesNo { probability }`, Score `probabilities` (string-keyed level
  indexes) to a `Vec<f64>` in level order. `calibration = Measured`, `rationale`
  empty, `usage` from the response.
- State with attachments is rejected: Jev accepts text only.
- Retries: HTTP 429 and 529 retry with exponential backoff, up to 3 attempts,
  honoring `retry-after` when present.
- Errors, all actionable. Verified live on 2026-10-06, the error body is always
  `{"detail": ...}` in one of three shapes, and the parser handles all three:
  - object `{"error_type", "message"}` (seen on 401 `authentication_error` and
    400 `api_usage_error`, e.g. "Unknown model: no-such-model");
  - array of validation items `{"type", "loc", "msg"}` (422), where `loc` such
    as `["body","questions","q","choice","criteria"]` names the question ID;
  - plain string (400, e.g. "Too many score levels. Must have at most 10
    levels.").

  Mapped messages:
  - 401: "TypeSafe rejected the API key for node '<id>'. Check TYPESAFE_API_KEY or
    run 'ailloy ai config set-key <id>'."
  - 400 unknown model: names the model and suggests `jev-latest`.
  - 400/422 validation: the message plus the question ID taken from `loc`.
  - 429/529 after retries: says the rate limit or overload persisted and to retry
    later.

  Every error includes the `x-typesafe-request-id` response header when present,
  for support requests.
- Live observations that shape the implementation: a three-question request
  answered in about 0.26 s using 462 input tokens; Choice `probabilities` keys come
  back alphabetically sorted (not in request order), so mapping is by key; the
  server accepted a one-level Score that its docs say needs two, so ailloy's
  client-side limits (2 to 10 levels) stay the source of truth.

### Chat emulation (default `Provider::evaluate`)

Every provider that implements `chat` gets eval through the default method:

- One chat call per `evaluate()`, with a system prompt explaining the task and a
  JSON schema generated from the questions:
  - YesNo: `{probability: number 0..1, rationale: string}`
  - Choice: `{probabilities: {<every option key>: number}, rationale: string}`
    with every option key required and no additional keys
  - Score: `{probabilities: array of exactly n numbers, rationale: string}`
- The user message contains the state (string as-is; JSON pretty-printed) and
  each question with its ID, instructions and criteria.
- Code, never the model, computes derived values: probabilities are clamped to
  [0, 1] and normalized to sum to 1 (uniform if all zero); the Choice winner is
  the most probable option (ties broken by option order); Score is
  `sum_i i * p_i`; confidence uses the formulas in section 1.
- Missing or extra question IDs or option keys produce an error naming the
  question ID and the node.
- `calibration = SelfReported`; `rationale` filled per question.
- Reuses existing structured-output support (native JSON schema on
  OpenAI-family, Ollama and Vertex; prompted JSON on Anthropic) and the
  sampling-rejection retry. The local-agent provider overrides `evaluate()`
  to return `Unsupported`, since CLI agents cannot honor a JSON schema.

### Config and routing

- `Capability::Eval`, config key `eval`, label "Evaluation".
  `Task::Evaluation`, config key `eval`. `ALL_CAPABILITIES` gains the entry.
- Every chat-capable `ProviderKind` adds `Eval` to `supported_capabilities()`,
  so chat nodes can be tagged `eval`.
- `Client::for_capability(Eval)` resolves `defaults.eval`; if unset, it falls
  back to the default chat node. Eval works with zero extra config.
- No `params.rs` entries for eval in this release.
- Config TUI: TypeSafe appears in the provider selector with model, endpoint
  and auth fields and the `eval` capability toggle.
- `ailloy ai test` on a TypeSafe node sends a single YesNo question instead of
  a chat message and prints the probability.
- `discover_env_keys()` detects `TYPESAFE_API_KEY` and proposes
  `typesafe/jev-latest`.

## 3. CLI: `ailloy eval`

### Input

Unchanged: positional argument, then `--file`, then stdin. `--context` stays;
with context, state becomes `{"context": <ctx>, "input": <input>}`.

### Question modes

Exactly one of these is required (a clap argument group):

| Mode | Extra flags | Gate (exit 0 or 1) |
|---|---|---|
| `--yes-no "<q>"` | `--true "<meaning>"`, `--false "<meaning>"` | pass when probability >= `--threshold` (default 0.5) |
| `--choice "<q>"` | `--option key[=description]` (repeatable) | `--expect <key>` (repeatable, any match passes); no `--expect` means always pass |
| `--score "<q>"` | `--level "<text>"` (repeatable, ordered) | `--min <f>` and/or `--max <f>` on the score; neither means always pass |
| `--questions <file>` | none | every question's own gate must pass |

All modes accept `--min-confidence <f>`, `--node <id>` and `--json`.

### Questions file (YAML or JSON, detected by extension, YAML default)

```yaml
questions:
  mentions_order:
    yes_no: "Does the output mention the order id?"
    true: "The order id appears verbatim"   # optional
    threshold: 0.8
  team:
    choice: "Which team should handle this?"
    options: { billing: "payments, refunds", technical: "bugs, outages", other: null }
    expect: [billing]
  frustration:
    score: "How frustrated is the customer?"
    levels: [Calm, Frustrated, Very angry]
    max: 1.0
    min_confidence: 0.6
```

Each entry has exactly one of `yes_no`, `choice`, `score`. Per-question
`min_confidence` overrides the global `--min-confidence`. All questions are sent
in one `evaluate()` call.

### Output

Text, one line per question (single-question mode uses the type name as ID):

```
PASS  yes-no  p=0.93  confidence 0.86  (jev-1.13.0)
PASS  choice  billing  p=0.88  confidence 0.81  (jev-1.13.0)
        billing 0.88 · technical 0.12
FAIL  score   1.05 of 0..2 (Frustrated)  confidence 0.92  max 1.0  (jev-1.13.0)
```

Chat judges show `(gpt-5.4, self-reported)` and print rationale lines under
each question. Batches prefix each line with the question ID. Uncertain
answers (below the confidence floor) are labelled `UNSURE`.

`--json`:

```json
{
  "model": "jev-1.13.0",
  "calibration": "measured",
  "pass": true,
  "answers": {
    "team": { "type": "choice", "choice": "billing",
              "probabilities": {"billing": 0.88, "technical": 0.12},
              "confidence": 0.81, "pass": true }
  },
  "usage": { "input_tokens": 318, "output_tokens": 34 }
}
```

`rationale` appears on an answer only when the backend provides one.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | every gate passed and every answer met its confidence floor |
| 1 | at least one gate failed (takes precedence over 4) |
| 2 | usage or config error |
| 3 | provider error |
| 4 | all gates passed, but at least one answer is below its confidence floor |

### Node selection

`--node`, else `defaults.eval`, else the default chat node.

## 4. Breaking changes (release 3.0.0)

### Library

| Change | Impact | Migration |
|---|---|---|
| `Capability::Eval` added | exhaustive `match` fails to compile | add an arm or `_ =>` |
| `ProviderKind::TypeSafe` added | same | same |
| `Task::Evaluation` added | same | same |
| `Capability`, `ProviderKind`, `Task` become `#[non_exhaustive]` | downstream exhaustive matches need `_ =>` | add `_ =>`; future additions become non-breaking |
| `supported_capabilities()` no longer always contains `Chat` | code assuming every node chats gets `Unsupported` | check capabilities before calling `chat` |
| `ALL_CAPABILITIES` / `ALL_TASKS` gain `eval` | iterating code sees a new entry | none, or filter |
| `Provider` gains `evaluate()` with a default | custom providers get chat-based eval automatically | override to opt out |

`Question` and `Answer` are new and `#[non_exhaustive]` from the start.

### CLI

- `-c/--criteria` and `--criteria-file` removed; use `--yes-no` and
  `--questions`.
- `--threshold` now gates the YesNo probability instead of the judge's
  self-reported score; the judge's boolean verdict is gone.
- `--json` output shape changed (section 3).
- New exit code 4.

### Versioning and docs

- Bump to 3.0.0. CHANGELOG gets a "Breaking changes" section split into
  Library and CLI with the tables above, plus "Added" entries for the eval
  capability and TypeSafe provider.
- Dependency snippets in `README.md` and `CLAUDE.md` move from `"2.0"` to `"3.0"`.

## 5. Testing

Unit tests, no network:

- Question builders and validation limits (options 2 to 255, levels 2 to 10,
  empty instructions, empty question map).
- TypeSafe wire mapping: request serialization for all three types (`YesNo` to
  `noul`), parsing of the documented example responses, Score probability order,
  401/422/429/529 handling and retry behavior (mocked HTTP).
- Chat emulation: schema generation, normalization (out-of-range, not summing to
  1, all zeros), Choice winner and ties, Score value, confidence formulas against
  hand-computed values, missing and extra IDs or option keys.
- Routing: `defaults.eval`, fallback to the default chat node, TypeSafe node
  rejecting `chat`.
- CLI: mode exclusivity, `--option key=desc` and bare-key parsing, questions
  file parsing (YAML and JSON, invalid entries), gate evaluation, exit-code
  precedence, JSON output shape.

Manual live check (not in CI): one batch with all three types against a real
Jev node and against a Foundry chat node, compared side by side.

CI gate as usual: `cargo fmt --all -- --check && cargo clippy -- -D warnings &&
cargo test`, plus `cargo build --no-default-features --lib`.

## 6. Documentation

Updated in the same change:

- `README.md`: rewritten eval section, TypeSafe provider, 3.0 version snippets.
- `src/doc/ai-reference.md`: full `ailloy eval` reference, questions file format,
  exit codes, TypeSafe node setup.
- `src/commands/skill.rs`: skill description and eval cheat-sheet lines.
- `CLAUDE.md`: `typesafe.rs` and eval types in the architecture map, the
  eval routing rule, the chat-emulation pattern, `TYPESAFE_API_KEY`.
- `examples/eval.sh` rewritten for the new flags; new `examples/eval.rs` library
  example showing a batch with all three types and confidence gating.
- `INSTALL.md` reviewed; `CHANGELOG.md` 3.0.0 entry.

## Out of scope

- Logprobs-based measured probabilities for chat models.
- Node-level default parameters for eval.
- Rebuilding other heuristics (interactive image prompt extraction, etc.) on the
  eval capability; possible follow-ups once it exists.
