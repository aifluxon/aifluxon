# A7 Provider and Stream Preservation Gate

- [x] OpenAI cache
- [x] OpenAI reasoning
- [x] DeepSeek thinking and effort
- [x] DeepSeek Responses routing, replay and web-search shape
- [x] Qwen thinking, budget, explicit cache and Responses cache
- [x] Qwen image and document/extraction constraints
- [x] Kimi logical-session cache and thinking replay
- [x] Gemini search shaping
- [x] Codex Responses/session contract
- [x] Custom provider modes and cumulative compatibility
- [x] ChatGPT Web remote continuity and hosted/local tool separation
- [x] DeepSeek Web persistent binding and cache-loss recovery
- [x] Common HTTP tuning, retry/redaction/error bounds
- [x] UTF-8, SSE and delta reconciliation

The checkboxes mean preservation coverage exists. Each A7 subphase reruns the complete gate; they do not mean the corresponding legacy implementation may be removed before its caller has switched and all preservation tests still pass.
