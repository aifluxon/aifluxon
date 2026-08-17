use super::{ApiFamily, OpenAiApiMode, OpenAiCompatibleConfig};
use crate::{codex, deepseek, gemini, kimi, openai, qwen};
use aifluxon_core::{MessageRole, ModelTurnRequest};
use serde_json::{json, Value};

const IMAGE_GENERATION_INSTRUCTION: &str = "When the user asks to generate or edit a raster image, use the native image_generation tool. Do not substitute SVG, HTML, placeholder files, or an unsupported-capability claim. The application will save and display the returned image.";

pub fn decorate_turn_body(
    body: &mut Value,
    config: &OpenAiCompatibleConfig,
    mode: OpenAiApiMode,
    request: &ModelTurnRequest,
) {
    match mode {
        OpenAiApiMode::ChatCompletions => decorate_chat(body, config, request),
        OpenAiApiMode::Responses => decorate_responses(body, config, request),
    }
    if let Some(cache_key) = request
        .features
        .prompt_cache_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["prompt_cache_key"] = json!(cache_key);
    }
}

fn decorate_chat(body: &mut Value, config: &OpenAiCompatibleConfig, request: &ModelTurnRequest) {
    if !config.include_stream_usage {
        if let Some(object) = body.as_object_mut() {
            object.remove("stream_options");
        }
    }
    match config.family {
        ApiFamily::OpenAi | ApiFamily::Custom => {
            openai::apply_chat_reasoning(
                body,
                &request.model,
                request.features.reasoning_effort.as_deref(),
            );
        }
        ApiFamily::Gemini => {
            gemini::apply_chat_reasoning(
                body,
                &request.model,
                request.features.reasoning_effort.as_deref(),
            );
        }
        ApiFamily::DeepSeek => {
            if deepseek::supports_thinking_toggle(&request.model) {
                let effort = deepseek::normalize_reasoning_effort(
                    &request.model,
                    request
                        .features
                        .reasoning_effort
                        .as_deref()
                        .unwrap_or("high"),
                );
                deepseek::apply_chat_thinking(body, thinking_enabled(&request.features), &effort);
            }
            replay_assistant_reasoning_content(body, request);
        }
        ApiFamily::Qwen => {
            qwen::apply_chat_thinking(
                body,
                &request.model,
                request
                    .features
                    .thinking_mode
                    .as_deref()
                    .unwrap_or("disabled"),
                request.features.thinking_budget.as_deref().unwrap_or(""),
            );
            if request.features.explicit_cache {
                if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
                    qwen::mark_latest_visible_user(messages, true);
                    qwen::refresh_cache_markers(messages, true);
                }
            }
        }
        ApiFamily::Kimi => {
            kimi::apply_chat_thinking(body, &request.model);
            kimi::apply_chat_limits(body, 32_768);
            replay_assistant_reasoning_content(body, request);
        }
        ApiFamily::Codex => {}
    }
    apply_tool_flags(body, config);
}

fn replay_assistant_reasoning_content(body: &mut Value, request: &ModelTurnRequest) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for (wire, original) in messages.iter_mut().zip(request.messages.iter()) {
        if original.role != MessageRole::Assistant {
            continue;
        }
        if let Some(reasoning) = original
            .provider_state
            .as_ref()
            .and_then(|state| state.get("reasoning_content"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            wire["reasoning_content"] = json!(reasoning);
        }
    }
}

fn decorate_responses(
    body: &mut Value,
    config: &OpenAiCompatibleConfig,
    request: &ModelTurnRequest,
) {
    if config.family != ApiFamily::DeepSeek {
        body["store"] = json!(false);
    } else {
        body.as_object_mut().map(|object| object.remove("store"));
    }
    match config.family {
        ApiFamily::OpenAi | ApiFamily::Custom => {
            let include_summary = request
                .features
                .reasoning_effort
                .as_deref()
                .is_none_or(|effort| effort != "none")
                && (config.family == ApiFamily::OpenAi
                    || request.features.reasoning_effort.is_some());
            openai::apply_responses_reasoning(
                body,
                &request.model,
                request.features.reasoning_effort.as_deref(),
                include_summary
                    && (!openai::supported_reasoning_efforts(&request.model).is_empty()
                        || (config.family == ApiFamily::Custom
                            && request.features.reasoning_effort.is_some())),
            );
        }
        ApiFamily::Qwen => {
            qwen::apply_responses_reasoning(
                body,
                request
                    .features
                    .thinking_mode
                    .as_deref()
                    .unwrap_or("disabled"),
            );
        }
        ApiFamily::DeepSeek => {
            let effort = deepseek::normalize_reasoning_effort(
                &request.model,
                request
                    .features
                    .reasoning_effort
                    .as_deref()
                    .unwrap_or("high"),
            );
            deepseek::apply_responses_reasoning(body, thinking_enabled(&request.features), &effort);
        }
        ApiFamily::Codex => {
            let effort = request
                .features
                .reasoning_effort
                .as_deref()
                .unwrap_or("default");
            openai::apply_responses_reasoning(body, &request.model, Some(effort), false);
            codex::apply_responses_contract(body, effort);
        }
        ApiFamily::Kimi | ApiFamily::Gemini => {}
    }
    apply_hosted_tools(body, config, request);
    apply_tool_flags(body, config);
}

fn apply_hosted_tools(
    body: &mut Value,
    config: &OpenAiCompatibleConfig,
    request: &ModelTurnRequest,
) {
    let mut tools = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let model_selected_search = request.features.web_search
        && matches!(
            config.family,
            ApiFamily::OpenAi | ApiFamily::Codex | ApiFamily::DeepSeek
        );
    if model_selected_search
        && !tools
            .iter()
            .any(|tool| tool.get("type").and_then(Value::as_str) == Some("web_search"))
    {
        tools.push(hosted_web_search_tool(config.family));
    }
    if request.features.image_generation
        && !tools
            .iter()
            .any(|tool| tool.get("type").and_then(Value::as_str) == Some("image_generation"))
    {
        tools.push(if config.family == ApiFamily::Codex {
            json!({ "type": "image_generation" })
        } else {
            json!({
                "type": "image_generation",
                "output_format": "png",
            })
        });
        let instructions = body
            .get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let instructions = if instructions.is_empty() {
            IMAGE_GENERATION_INSTRUCTION.to_string()
        } else {
            format!("{instructions}\n\n{IMAGE_GENERATION_INSTRUCTION}")
        };
        body["instructions"] = json!(instructions);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
}

fn apply_tool_flags(body: &mut Value, config: &OpenAiCompatibleConfig) {
    if body.get("tools").and_then(Value::as_array).is_none() {
        return;
    }
    if config.send_tool_choice_auto {
        body["tool_choice"] = json!("auto");
    } else if let Some(object) = body.as_object_mut() {
        object.remove("tool_choice");
    }
    if config.parallel_tool_calls && config.family != ApiFamily::Kimi {
        body["parallel_tool_calls"] = json!(true);
    } else if let Some(object) = body.as_object_mut() {
        object.remove("parallel_tool_calls");
    }
}

fn hosted_web_search_tool(family: ApiFamily) -> Value {
    if matches!(family, ApiFamily::Qwen | ApiFamily::DeepSeek) {
        json!({ "type": "web_search" })
    } else {
        json!({
            "type": "web_search",
            "external_web_access": true,
        })
    }
}

fn thinking_enabled(features: &aifluxon_core::ProviderFeatureRequest) -> bool {
    !matches!(
        features.thinking_mode.as_deref().unwrap_or("disabled"),
        "disabled" | "none" | "off"
    )
}

pub fn effective_api_mode(config: &OpenAiCompatibleConfig, model: &str) -> OpenAiApiMode {
    let capabilities = match config.family {
        ApiFamily::OpenAi => openai::capabilities(model),
        ApiFamily::DeepSeek => deepseek::capabilities(model),
        ApiFamily::Qwen => qwen::capabilities(model),
        ApiFamily::Gemini => gemini::capabilities(model),
        ApiFamily::Codex => codex::capabilities(),
        ApiFamily::Kimi | ApiFamily::Custom => {
            return config.api_mode;
        }
    };
    match config.api_mode {
        OpenAiApiMode::Responses if capabilities.supports_responses => OpenAiApiMode::Responses,
        OpenAiApiMode::Responses if capabilities.supports_chat_completions => {
            OpenAiApiMode::ChatCompletions
        }
        OpenAiApiMode::ChatCompletions if !capabilities.supports_chat_completions => {
            OpenAiApiMode::Responses
        }
        OpenAiApiMode::ChatCompletions => OpenAiApiMode::ChatCompletions,
        OpenAiApiMode::Responses => OpenAiApiMode::Responses,
    }
}
