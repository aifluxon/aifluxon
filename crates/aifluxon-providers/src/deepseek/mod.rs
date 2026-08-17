use crate::strategy::ModelApiCapabilities;
use serde_json::{json, Value};

pub fn capabilities(model: &str) -> ModelApiCapabilities {
    if model.trim().eq_ignore_ascii_case("deepseek-v4-flash") {
        ModelApiCapabilities::CHAT_AND_RESPONSES
    } else {
        ModelApiCapabilities::CHAT_ONLY
    }
}

pub fn supports_low_reasoning_effort(model: &str) -> bool {
    capabilities(model).supports_responses
}

pub fn supports_thinking_toggle(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("deepseek-v4")
}

pub fn normalize_reasoning_effort(model: &str, requested: &str) -> String {
    if requested == "low" && !supports_low_reasoning_effort(model) {
        "high".to_string()
    } else {
        requested.to_string()
    }
}

pub fn apply_responses_reasoning(body: &mut Value, thinking_enabled: bool, effort: &str) {
    body["reasoning"] = json!({
        "effort": if thinking_enabled { effort } else { "none" }
    });
}

pub fn apply_chat_thinking(body: &mut Value, thinking_enabled: bool, effort: &str) {
    body["thinking"] = json!({
        "type": if thinking_enabled { "enabled" } else { "disabled" }
    });
    if thinking_enabled {
        body["reasoning_effort"] = json!(effort);
    }
}

pub fn tool_mode_error_status(status: u16) -> bool {
    matches!(status, 400 | 422 | 500)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_v4_flash_supports_responses_and_low_effort() {
        assert!(capabilities("deepseek-v4-flash").supports_responses);
        assert_eq!(normalize_reasoning_effort("deepseek-chat", "low"), "high");
    }
}
