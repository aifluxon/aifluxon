use crate::strategy::{model_is_family, normalize_effort, ModelApiCapabilities};

const LEGACY_PRO: &[&str] = &["low", "high"];
const MINIMAL: &[&str] = &["minimal", "low", "medium", "high"];
const FLASH_25: &[&str] = &["none", "low", "medium", "high"];
const STANDARD: &[&str] = &["low", "medium", "high"];

pub fn capabilities(_model: &str) -> ModelApiCapabilities {
    ModelApiCapabilities::CHAT_ONLY
}

pub fn supported_reasoning_efforts(model: &str) -> &'static [&'static str] {
    let model = model.trim().to_ascii_lowercase();
    if model_is_family(&model, "gemini-3.5") {
        MINIMAL
    } else if model_is_family(&model, "gemini-3.1-pro") {
        STANDARD
    } else if model_is_family(&model, "gemini-3-flash")
        || model_is_family(&model, "gemini-3.1-flash-lite")
    {
        MINIMAL
    } else if model_is_family(&model, "gemini-2.5-pro") {
        STANDARD
    } else if model_is_family(&model, "gemini-2.5") {
        FLASH_25
    } else if model_is_family(&model, "gemini-3-pro") {
        LEGACY_PRO
    } else {
        &[]
    }
}

pub fn normalize_reasoning_effort(model: &str, requested: &str) -> String {
    normalize_effort(supported_reasoning_efforts(model), requested)
}

pub fn native_base_url(base_url: &str) -> String {
    base_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/openai")
        .to_string()
}

pub fn generate_content_url(base_url: &str, model: &str) -> String {
    format!(
        "{}/models/{}:generateContent",
        native_base_url(base_url),
        model.trim()
    )
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
