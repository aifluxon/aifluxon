# A10 continuation semantics

Characterization of the three legacy Agent-loop continuation heuristics, and the
canonical AIFLUXON mapping used after A10R.9.

Ordinary Agent live path is `Aifluxon::start_on` → Runtime model/tool loop →
`ModelProvider::next_turn`. Runtime never branches on provider name. A provider
that needs another model turn returns generic `ProviderTerminal::Continue(reason)`.

## A. No-tool continuation

### Why it existed

Some models emit a visible promise to inspect, search, read, run, verify, or
edit, then finish the turn **without a tool call**. The legacy loop treated that
as an incomplete Agent turn, not as a final answer.

This is **not** "any answer without tools continues". A normal final answer such
as `检查结果：语法检查通过` or `The answer is 42.` must complete immediately.

### Legacy trigger

From `agent_stream::should_continue_without_tool_call` /
`assistant_content_needs_tool_continuation`:

- tools are enabled
- the turn has no valid tool call
- visible assistant text matches an opening-clause tool-intent heuristic
  (English `I'll inspect/read/search...` or Chinese `我先查看/先看一下...`)
- continuation count `< 2` (`NO_TOOL_CONTINUATION_LIMIT`)
- **DeepSeek API is excluded** (`route != DeepSeek`), because thinking traces
  often contain the same intent language after the turn is already complete

On continue, legacy appended the assistant text plus a system instruction telling
the model to actually emit tool calls. Hitting the limit completed with the
visible text (it did not fail). A later successful tool round reset the counter.

The legacy system prompt mentioned EasyPhy/`rg`/`shell`. That wording is Host
product policy and is **not** copied into Runtime.

### Classification

**Category B + C**

- Detection stays in `aifluxon-providers` (family policy: every family except
  DeepSeek may return `Continue(Incomplete)`).
- Orchestration is Runtime generic: honor `Continue`, consume the same Run
  budget, keep Session/`ProviderSessionKey`/ToolLedger.

DeepSeek Web's separate protocol-repair loop in the private web driver is **not**
this heuristic. It remains Host-owned leftover, not A10R.9 Runtime logic.

## B. Qwen summary continuation

### Why it existed

Qwen sometimes opens a turn with a leading `<summary>...</summary>` block and no
user-facing answer. The legacy `QwenSummaryFilter` hid that block from the
visible stream. If the visible remainder was empty, the loop continued **once**
(`QWEN_SUMMARY_CONTINUATION_LIMIT = 1`) with the summary as hidden context.

If visible text followed the summary (`<summary>internal</summary>\n\n已完成。`),
the turn completed normally. A normal answer with no summary never continued.

### New mapping

The Qwen family assembler strips a leading summary from visible text, stores it
as opaque `hidden_context`, and returns `Continue(SummaryOnly)` only when the
visible remainder is empty and there are no tool calls. Runtime does not know
this is Qwen.

Qwen explicit-cache marker refresh on the next request is already done by
family decorate in `next_turn` (Category A, not re-implemented in Runtime).

## C. Codex end_turn continuation

### Why it existed

Codex Responses can finish a remote HTTP turn with `end_turn=false`, meaning the
logical assistant turn is not terminal. Legacy continued up to **4** times,
keeping `responses_replay_items` on the next assistant message so encrypted
reasoning / hosted activity could be replayed. A system prompt was added only
when both visible text and replay items were empty.

`end_turn=true` or missing `end_turn` completed.

### New mapping

The Codex family maps `opaque.end_turn == false` to
`Continue(ProviderRequested)`. Runtime never stores `end_turn` as a domain
field. Replay items travel in `ModelTurn.opaque` / `Message.provider_state`
(`response_items`) and are injected into the next Responses `input` by the
provider.

ChatGPT Web `end_turn` is a **message-scoped completion signal inside one
`next_turn`**, not this Runtime continuation. Category A for that adapter.

## Ownership

| Legacy behavior             | Classification                         | New owner                         | New mechanism                                      | Preserved? |
| --------------------------- | -------------------------------------- | --------------------------------- | -------------------------------------------------- | ---------- |
| no-tool continuation        | B (detect) + C (loop)                  | Provider family + Runtime         | `Continue(Incomplete)`, limit 2                    | Yes        |
| Qwen summary continuation   | B                                      | Qwen family in `aifluxon-providers` | strip `<summary>`, `Continue(SummaryOnly)`, limit 1 | Yes        |
| Codex end_turn continuation | B                                      | Codex family in `aifluxon-providers` | `Continue(ProviderRequested)`, limit 4           | Yes        |

Priority:

```text
RunLimits.max_model_rounds
        >
per-reason continuation limit
        >
Provider Continue recommendation
```

Hitting a continuation limit completes the current text (legacy success path).
Hitting `max_model_rounds` fails closed (existing Runtime budget).

## Runtime contract

Continuation does not create a new Run, Session, `ProviderSessionKey`, budget,
or ToolLedger. Event sequence stays monotonic. Exactly one terminal event is
emitted after continuations finish. Cancellation checked before each provider
request, including the request after a Continue signal.

## Out of scope (recorded leftovers)

- `legacy_stream_agent_chat_api` remains a dead characterization oracle.
- Public ChatGPT Web / DeepSeek Web capability stubs: removed from the `aifluxon-providers` crate root in A13.1. EasyPhy private adapters remain.
  pre-cutover cleanup.
- `AllowAllToolPolicy` on the Host strangler: A12.
- DeepSeek Web local tool-protocol repair loop: Host private adapter.
- EasyPhy-specific continuation wording (Chinese final-answer instruction,
  `rg`/`shell` reminders): Host product prompts, not Runtime.
