mod chatgpt_web;
pub mod codex;
pub mod common;
pub mod custom;
pub mod deepseek;
mod deepseek_web;
pub mod gemini;
pub mod kimi;
pub mod openai;
pub mod openai_compatible;
pub mod qwen;
pub mod session;
pub mod sse;
pub mod strategy;
pub mod tool_deltas;

pub use chatgpt_web::chatgpt_web_capabilities;
pub use common::{
    build_http_client, is_transient_reqwest_error, retry_backoff, sanitize_provider_error,
    send_with_retry, DeltaMode, HttpClientTuning, HttpTransport, IncrementalSseParser, SseEvent,
    TextDeltaReconciler, TransportFailure, TransportFailureKind, Utf8ChunkDecoder,
};
pub use deepseek_web::deepseek_web_capabilities;
pub use openai::{descriptor_from_openai_tool, schema_from_openai_tools};
pub use openai_compatible::OpenAiCompatibleProvider;
pub use session::*;
pub use strategy::{ModelApiCapabilities, PromptCacheStrategy};
pub use tool_deltas::{classify_chat_completion_chunk, ToolCallAssembler};

#[cfg(test)]
mod crate_boundary {
    #[test]
    fn providers_manifest_does_not_depend_on_tauri() {
        let manifest = include_str!("../Cargo.toml");
        for forbidden in ["tauri", "windows-sys", "nix", "portable-pty"] {
            assert!(
                !manifest.contains(forbidden),
                "aifluxon-providers must not depend on {forbidden}"
            );
        }
        assert!(manifest.contains("aifluxon-auth"));
    }
}
