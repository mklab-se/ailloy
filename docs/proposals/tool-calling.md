# Proposal: tool-use signaling for library consumers

**Status:** proposed (deferred from the 1.0 scope on 2026-07-07)

## Motivation

Ailloy deliberately has no tool/function-calling support: `Message` is a plain
role+string and the `Provider` trait exposes chat/stream/image/embed only.
That keeps the library small, and none of the dependent tools (rigg, cosq,
mdeck, pidge) needs it today.

The interesting version of this feature is NOT ailloy executing tools itself.
It is ailloy **letting the calling tool know that the model wants a tool**, and
letting the caller decide what to do:

```rust
let tools = &[ToolSpec { name: "search_docs", description: "...", parameters: schema }];
match client.chat_with_tools(&messages, tools, &options).await? {
    ChatOutcome::Message(response) => println!("{}", response.content),
    ChatOutcome::ToolCalls(calls) => {
        for call in calls {
            let result = my_tool_router(&call.name, &call.arguments)?; // caller's code
            messages.push(Message::tool_result(call.id, result));
        }
        // loop: send messages back until ChatOutcome::Message
    }
}
```

## Sketch

- `ToolSpec { name, description, parameters: serde_json::Value }` (JSON Schema).
- `ChatOutcome::{Message(ChatResponse), ToolCalls(Vec<ToolCall>)}` where
  `ToolCall { id, name, arguments: serde_json::Value }`.
- `Message` grows `Role::Tool` + a `tool_result` constructor (content stays
  string-typed; the provider adapters map to each API's native shape).
- Providers: OpenAI-family `tools`/`tool_calls`, Anthropic `tools`/`tool_use`
  blocks, Gemini `functionDeclarations`/`functionCall` (+ `thoughtSignature`
  echo!), Ollama `tools`.
- Streaming: a `StreamEvent::ToolCallDelta` variant, or (simpler) documenting
  that tool rounds are non-streaming.

## Open questions

- Multimodal/tool blocks push `Message.content` toward a block list — a
  breaking change; likely an additive `Vec<ContentBlock>` alongside `content`.
- Anthropic requires echoing thinking blocks with tool results on the newest
  models; ailloy would need to carry opaque provider state per conversation.
- Should `Conversation` orchestrate the tool loop (callback-based)?

## Why not now

Large API-design surface, no current consumer. Revisit when a dependent tool
actually needs agentic behavior through ailloy.
