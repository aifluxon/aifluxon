mod tools;

pub use tools::{descriptor_from_openai_tool, schema_from_openai_tools};

use crate::strategy::{model_is_family, normalize_effort, ModelApiCapabilities};

const NONE_LOW_TO_MAX: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];
const NONE_LOW_TO_XHIGH: &[&str] = &["none", "low", "medium", "high", "xhigh"];
const PRO: &[&str] = &["medium", "high", "xhigh"];
const CODEX: &[&str] = &["low", "medium", "high", "xhigh"];
const GPT_51: &[&str] = &["none", "low", "medium", "high"];
const LEGACY_GPT_5: &[&str] = &["minimal", "low", "medium", "high"];
const STANDARD: &[&str] = &["low", "medium", "high"];

pub fn model_requires_responses(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model == "codex-mini-latest"
        || (model.starts_with("gpt-5")
            && (model.contains("-codex")
                || ["gpt-5-pro", "gpt-5.2-pro", "gpt-5.4-pro", "gpt-5.5-pro"]
                    .iter()
                    .any(|family| model_is_family(&model, family))))
}

pub fn capabilities(model: &str) -> ModelApiCapabilities {
    if model_requires_responses(model) {
        ModelApiCapabilities::RESPONSES_ONLY
    } else {
        ModelApiCapabilities::CHAT_AND_RESPONSES
    }
}

pub fn supported_reasoning_efforts(model: &str) -> &'static [&'static str] {
    let model = model.trim().to_ascii_lowercase();
    if model_is_family(&model, "gpt-5.6") {
        NONE_LOW_TO_MAX
    } else if model_is_family(&model, "gpt-5.5-pro")
        || model_is_family(&model, "gpt-5.4-pro")
        || model_is_family(&model, "gpt-5.2-pro")
    {
        PRO
    } else if model_is_family(&model, "gpt-5.3-codex")
        || model_is_family(&model, "gpt-5.2-codex")
        || model_is_family(&model, "gpt-5.1-codex")
    {
        CODEX
    } else if model_is_family(&model, "gpt-5.5")
        || model_is_family(&model, "gpt-5.4")
        || model_is_family(&model, "gpt-5.2")
    {
        NONE_LOW_TO_XHIGH
    } else if model_is_family(&model, "gpt-5.1") {
        GPT_51
    } else if model == "gpt-5-pro"
        || model_is_family(&model, "o1-pro")
        || model_is_family(&model, "o3-pro")
    {
        &["high"]
    } else if model_is_family(&model, "gpt-5") {
        LEGACY_GPT_5
    } else if model_is_family(&model, "o1")
        || model_is_family(&model, "o3")
        || model_is_family(&model, "o4")
    {
        STANDARD
    } else {
        &[]
    }
}

pub fn normalize_reasoning_effort(model: &str, requested: &str) -> String {
    normalize_effort(supported_reasoning_efforts(model), requested)
}

pub fn apply_chat_reasoning(body: &mut serde_json::Value, model: &str, requested: Option<&str>) {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let effort = normalize_reasoning_effort(model, requested);
    if effort == "default" {
        return;
    }
    body["reasoning_effort"] = serde_json::json!(effort);
}

pub fn apply_responses_reasoning(
    body: &mut serde_json::Value,
    model: &str,
    requested: Option<&str>,
    include_summary: bool,
) {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        if include_summary {
            if !body
                .get("reasoning")
                .is_some_and(serde_json::Value::is_object)
            {
                body["reasoning"] = serde_json::json!({});
            }
            body["reasoning"]["summary"] = serde_json::json!("auto");
        }
        return;
    };
    let effort = normalize_reasoning_effort(model, requested);
    if effort != "default" {
        if !body
            .get("reasoning")
            .is_some_and(serde_json::Value::is_object)
        {
            body["reasoning"] = serde_json::json!({});
        }
        body["reasoning"]["effort"] = serde_json::json!(effort);
        if include_summary && effort != "none" {
            body["reasoning"]["summary"] = serde_json::json!("auto");
        }
    } else if include_summary {
        if !body
            .get("reasoning")
            .is_some_and(serde_json::Value::is_object)
        {
            body["reasoning"] = serde_json::json!({});
        }
        body["reasoning"]["summary"] = serde_json::json!("auto");
    }
}

#[cfg(test)]
mod strategy_tests {
    use super::*;

    #[test]
    fn required_models_route_to_responses() {
        assert!(!capabilities("gpt-5.4-codex").supports_chat_completions);
        assert!(capabilities("gpt-4.1").supports_chat_completions);
    }

    #[test]
    fn reasoning_effort_falls_back_to_supported_value() {
        assert_eq!(normalize_reasoning_effort("gpt-5.4-pro", "max"), "xhigh");
    }
}
