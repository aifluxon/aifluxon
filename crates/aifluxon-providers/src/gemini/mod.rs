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
    if model_is_family(&model, "gemini-3.8")
        || model_is_family(&model, "gemini-3.7")
        || model_is_family(&model, "gemini-4")
    {
        STANDARD
    } else if model_is_family(&model, "gemini-3.5") || model_is_family(&model, "gemini-3.6") {
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
    if supported_reasoning_efforts(model).is_empty() {
        return;
    }
    // Use Gemini's explicit controls so minimal is passed as a thinking level,
    // rather than depending on the compatibility endpoint's effort mapping.
    let effort = normalize_reasoning_effort(model, requested.unwrap_or("default").trim());
    let mut thinking = serde_json::json!({ "include_thoughts": true });
    let model = model.trim().to_ascii_lowercase();
    if effort != "default" {
        if model.starts_with("gemini-2.5") {
            let budget = match effort.as_str() {
                "none" => 0,
                "low" | "minimal" => 1024,
                "medium" => 8192,
                _ => 24576,
            };
            thinking["thinking_budget"] = serde_json::json!(budget);
        } else {
            thinking["thinking_level"] = serde_json::json!(effort);
        }
    }
    if let Some(object) = body.as_object_mut() {
        object.remove("reasoning_effort");
    }
    body["extra_body"]["google"]["thinking_config"] = thinking;
}

/// Extension objects are snapshots, including opaque signatures. Never trim,
/// decode, concatenate or attach a signature to a different tool call.
pub(crate) fn merge_extra_content(into: &mut serde_json::Value, from: &serde_json::Value) {
    if let (Some(target), Some(source)) = (into.as_object_mut(), from.as_object()) {
        for (key, value) in source {
            if value.is_object() {
                let entry = target
                    .entry(key.clone())
                    .or_insert_with(|| serde_json::json!({}));
                merge_extra_content(entry, value);
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    } else {
        *into = from.clone();
    }
}
