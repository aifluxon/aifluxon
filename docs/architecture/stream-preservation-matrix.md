# Stream preservation matrix

| Input/transition | Required result | Executable evidence |
|---|---|---|
| UTF-8 scalar split across byte chunks | decode without replacement characters | `aifluxon_providers::common::utf8` and EasyPhy stream decoder tests |
| SSE line/event split across chunks | assemble one ordered event | `aifluxon_providers::common::sse` all-split fixtures |
| CRLF, multiline data and EOF without separator | preserve SSE semantics and flush | common SSE tests |
| Incremental content | append each delta once | standard provider delta test |
| Cumulative full-text compatibility | append missing suffix only for opted-in compatible endpoints | custom cumulative-delta test |
| Duplicate Responses event | suppress by stable event identity | Responses event-key test |
| Interleaved reasoning/text | emit on separate channels in source order | reasoning/visible classification and Responses part tests |
| Split tool id/name/arguments | assemble by index and stable call identity | common tool delta tests |
| Multiple tool calls | preserve order and result pairing | common tool assembler and EasyPhy persisted-turn tests |
| Usage-only event | update usage without faking visible activity | common delta classification test |
| Terminal snapshot and usage ordering | reconcile missing text/tool calls, require explicit terminal | EasyPhy Responses terminal tests |
| Cancellation mid-frame | no terminal data fabrication; Run cancellation remains authoritative | canonical Run cancellation tests plus provider wait tests |
| Malformed partial frame | fail closed at protocol layer | private Web malformed SSE tests and Responses parse tests |

Canonical Runtime consumes `ModelTurn`/`ModelEventSink`; provider wire JSON and reconciliation remain provider-owned.
