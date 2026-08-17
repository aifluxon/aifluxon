use crate::controlled::{text_turn, tool_turn, ControlledProvider};
use crate::{AifluxonError, AifluxonErrorKind, ModelProvider, ProviderId, ProviderRegistry};
use aifluxon_providers::openai_compatible::{OpenAiApiMode, OpenAiCompatibleConfig};
use aifluxon_providers::{custom, OpenAiCompatibleProvider};
use serde_json::{json, Value};

pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEEPSEEK_DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
pub const QWEN_DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
pub const KIMI_DEFAULT_BASE_URL: &str = "https://api.moonshot.cn/v1";
pub const GEMINI_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai";
pub const CODEX_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderBinding {
    pub provider_id: ProviderId,
    pub model: String,
}

pub fn register_provider_from_json(
    registry: &ProviderRegistry,
    spec: &Value,
) -> Result<ProviderBinding, AifluxonError> {
    let kind = required_str(spec, "kind")?.to_ascii_lowercase();
    match kind.as_str() {
        "controlled" => register_controlled(registry, spec),
        "openai" => register_compatible(
            registry,
            spec,
            "openai",
            OPENAI_DEFAULT_BASE_URL,
            OpenAiApiMode::ChatCompletions,
            false,
        ),
        "deepseek" => register_compatible(
            registry,
            spec,
            "deepseek",
            DEEPSEEK_DEFAULT_BASE_URL,
            OpenAiApiMode::ChatCompletions,
            false,
        ),
        "qwen" => register_compatible(
            registry,
            spec,
            "qwen",
            QWEN_DEFAULT_BASE_URL,
            OpenAiApiMode::ChatCompletions,
            false,
        ),
        "kimi" => register_compatible(
            registry,
            spec,
            "kimi",
            KIMI_DEFAULT_BASE_URL,
            OpenAiApiMode::ChatCompletions,
            false,
        ),
        "gemini" => register_compatible(
            registry,
            spec,
            "gemini",
            GEMINI_DEFAULT_BASE_URL,
            OpenAiApiMode::ChatCompletions,
            false,
        ),
        "codex" => register_compatible(
            registry,
            spec,
            "codex",
            CODEX_DEFAULT_BASE_URL,
            OpenAiApiMode::Responses,
            false,
        ),
        "custom" => register_custom(registry, spec),
        other => Err(AifluxonError::new(
            AifluxonErrorKind::InvalidConfiguration,
            format!("Unsupported provider kind `{other}`."),
        )),
    }
}

fn register_controlled(
    registry: &ProviderRegistry,
    spec: &Value,
) -> Result<ProviderBinding, AifluxonError> {
    let provider_id = spec
        .get("provider_id")
        .and_then(Value::as_str)
        .unwrap_or("controlled");
    let model = spec
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("controlled-model")
        .to_string();
    let turns = controlled_turns(spec)?;
    let mut provider = ControlledProvider::new(ProviderId::new(provider_id), turns);
    if let Some(millis) = spec.get("delay_ms").and_then(Value::as_u64) {
        provider = provider.with_delay(std::time::Duration::from_millis(millis));
    }
    registry
        .register(provider.id().clone(), provider)
        .map_err(|error| {
            AifluxonError::new(AifluxonErrorKind::InvalidConfiguration, error.to_string())
        })?;
    Ok(ProviderBinding {
        provider_id: ProviderId::new(provider_id),
        model,
    })
}

fn controlled_turns(spec: &Value) -> Result<Vec<aifluxon_core::ModelTurn>, AifluxonError> {
    if let Some(responses) = spec.get("responses").and_then(Value::as_array) {
        return responses
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|text| text_turn(text.to_string()))
                    .ok_or_else(|| {
                        AifluxonError::new(
                            AifluxonErrorKind::InvalidConfiguration,
                            "Controlled provider responses must be strings.",
                        )
                    })
            })
            .collect();
    }
    if let Some(turns) = spec.get("turns").and_then(Value::as_array) {
        return turns.iter().map(parse_controlled_turn).collect();
    }
    Err(AifluxonError::new(
        AifluxonErrorKind::InvalidConfiguration,
        "Controlled provider requires `responses` or `turns`.",
    ))
}

fn parse_controlled_turn(value: &Value) -> Result<aifluxon_core::ModelTurn, AifluxonError> {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        if value.get("tool").is_none() {
            return Ok(text_turn(text.to_string()));
        }
    }
    if let Some(name) = value.get("tool").and_then(Value::as_str) {
        let id = value.get("id").and_then(Value::as_str).unwrap_or(name);
        let arguments = value.get("arguments").cloned().unwrap_or_else(|| json!({}));
        return Ok(tool_turn(name, id, arguments));
    }
    if let Some(text) = value.as_str() {
        return Ok(text_turn(text.to_string()));
    }
    Err(AifluxonError::new(
        AifluxonErrorKind::InvalidConfiguration,
        "Controlled turns require `text` or `tool`.",
    ))
}

fn register_compatible(
    registry: &ProviderRegistry,
    spec: &Value,
    default_id: &str,
    default_base_url: &str,
    default_mode: OpenAiApiMode,
    allow_cumulative_delta: bool,
) -> Result<ProviderBinding, AifluxonError> {
    let provider_id = spec
        .get("provider_id")
        .and_then(Value::as_str)
        .unwrap_or(default_id);
    let model = required_str(spec, "model")?.to_string();
    let api_key = spec
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let base_url = spec
        .get("base_url")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default_base_url)
        .to_string();
    let api_mode = parse_api_mode(spec.get("api_mode").and_then(Value::as_str), default_mode)?;
    let cumulative = spec
        .get("allow_cumulative_delta")
        .and_then(Value::as_bool)
        .unwrap_or(allow_cumulative_delta);
    let config = OpenAiCompatibleConfig::new(
        ProviderId::new(provider_id),
        base_url,
        api_key,
        api_mode,
        cumulative,
    );
    let provider = OpenAiCompatibleProvider::configured(config);
    let id = ProviderId::new(provider_id);
    registry.register(id.clone(), provider).map_err(|error| {
        AifluxonError::new(AifluxonErrorKind::InvalidConfiguration, error.to_string())
    })?;
    Ok(ProviderBinding {
        provider_id: id,
        model,
    })
}

fn register_custom(
    registry: &ProviderRegistry,
    spec: &Value,
) -> Result<ProviderBinding, AifluxonError> {
    let provider_id = spec
        .get("provider_id")
        .and_then(Value::as_str)
        .unwrap_or("custom");
    let model = required_str(spec, "model")?.to_string();
    let api_key = spec
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let base_url = required_str(spec, "base_url")?;
    if base_url.trim().is_empty() {
        return Err(AifluxonError::new(
            AifluxonErrorKind::InvalidConfiguration,
            "Custom providers require an explicit base_url.",
        ));
    }
    let api_mode = parse_api_mode(
        spec.get("api_mode").and_then(Value::as_str),
        OpenAiApiMode::ChatCompletions,
    )?;
    let config = custom::config(ProviderId::new(provider_id), base_url, api_key, api_mode);
    let id = ProviderId::new(provider_id);
    registry
        .register(id.clone(), OpenAiCompatibleProvider::configured(config))
        .map_err(|error| {
            AifluxonError::new(AifluxonErrorKind::InvalidConfiguration, error.to_string())
        })?;
    Ok(ProviderBinding {
        provider_id: id,
        model,
    })
}

fn parse_api_mode(
    raw: Option<&str>,
    default: OpenAiApiMode,
) -> Result<OpenAiApiMode, AifluxonError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(default),
        Some("chat_completions") | Some("chat") => Ok(OpenAiApiMode::ChatCompletions),
        Some("responses") => Ok(OpenAiApiMode::Responses),
        Some(other) => Err(AifluxonError::new(
            AifluxonErrorKind::InvalidConfiguration,
            format!("Unsupported api_mode `{other}`."),
        )),
    }
}

fn required_str<'a>(spec: &'a Value, field: &str) -> Result<&'a str, AifluxonError> {
    spec.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AifluxonError::new(
                AifluxonErrorKind::InvalidConfiguration,
                format!("Provider spec requires `{field}`."),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controlled_spec_registers_without_network_configuration() {
        let registry = ProviderRegistry::new();
        let binding = register_provider_from_json(
            &registry,
            &serde_json::json!({
                "kind": "controlled",
                "responses": ["hello"]
            }),
        )
        .unwrap();
        assert_eq!(binding.provider_id.as_str(), "controlled");
        assert!(registry.contains(&binding.provider_id));
    }

    #[test]
    fn custom_spec_requires_base_url() {
        let registry = ProviderRegistry::new();
        let error = register_provider_from_json(
            &registry,
            &serde_json::json!({
                "kind": "custom",
                "model": "local-model"
            }),
        )
        .unwrap_err();
        assert_eq!(error.kind(), AifluxonErrorKind::InvalidConfiguration);
    }
}
