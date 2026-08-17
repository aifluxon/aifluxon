use crate::strategy::{model_is_family, ModelApiCapabilities};
use serde_json::{json, Value};

fn normalized_model(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn is_v4_flash(model: &str) -> bool {
    model_is_family(&normalized_model(model), "deepseek-v4-flash")
}

fn is_v4_pro(model: &str) -> bool {
    model_is_family(&normalized_model(model), "deepseek-v4-pro")
}

pub fn capabilities(model: &str) -> ModelApiCapabilities {
    if is_v4_flash(model) || is_v4_pro(model) {
        ModelApiCapabilities::CHAT_AND_RESPONSES
    } else {
        ModelApiCapabilities::CHAT_ONLY
    }
}

pub fn supports_low_reasoning_effort(model: &str) -> bool {
    is_v4_flash(model)
}

pub fn supports_thinking_toggle(model: &str) -> bool {
    normalized_model(model).starts_with("deepseek-v4")
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
    fn v4_flash_and_pro_support_responses_but_only_flash_keeps_low_effort() {
        assert!(capabilities("deepseek-v4-flash").supports_responses);
        assert!(capabilities("deepseek-v4-pro").supports_responses);
        assert!(capabilities("DeepSeek-V4-Pro").supports_responses);
        assert!(!capabilities("deepseek-chat").supports_responses);
        assert!(supports_low_reasoning_effort("deepseek-v4-flash"));
        assert!(!supports_low_reasoning_effort("deepseek-v4-pro"));
        assert_eq!(normalize_reasoning_effort("deepseek-v4-pro", "low"), "high");
        assert_eq!(
            normalize_reasoning_effort("deepseek-v4-flash", "low"),
            "low"
        );
        assert_eq!(normalize_reasoning_effort("deepseek-chat", "low"), "high");
    }
}
