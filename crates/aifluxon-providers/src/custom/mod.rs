use crate::openai_compatible::{ApiFamily, OpenAiApiMode, OpenAiCompatibleConfig};
use crate::strategy::normalize_effort;
use aifluxon_core::ProviderId;

const ALL_COMPATIBLE: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

pub fn normalize_reasoning_effort(requested: &str) -> String {
    normalize_effort(ALL_COMPATIBLE, requested)
}

pub fn config(
    provider_id: impl Into<ProviderId>,
    base_url: impl Into<String>,
    api_key: impl Into<String>,
    mode: OpenAiApiMode,
) -> OpenAiCompatibleConfig {
    let mut config = OpenAiCompatibleConfig::new(provider_id, base_url, api_key, mode, true);
    config.family = ApiFamily::Custom;
    config.allow_cumulative_delta = true;
    config
}
