#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelApiCapabilities {
    pub supports_chat_completions: bool,
    pub supports_responses: bool,
}

impl ModelApiCapabilities {
    pub const CHAT_ONLY: Self = Self {
        supports_chat_completions: true,
        supports_responses: false,
    };
    pub const CHAT_AND_RESPONSES: Self = Self {
        supports_chat_completions: true,
        supports_responses: true,
    };
    pub const RESPONSES_ONLY: Self = Self {
        supports_chat_completions: false,
        supports_responses: true,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptCacheStrategy {
    CodexSessionState,
    OpenAiPromptCacheKey,
    KimiSessionCacheKey,
    QwenExplicitBreakpoints,
    QwenResponsesSessionCache,
    AutomaticPrefix,
    PortableDefault,
}

pub(crate) fn model_is_family(model: &str, family: &str) -> bool {
    model == family
        || model
            .strip_prefix(family)
            .is_some_and(|suffix| suffix.starts_with('-'))
}

pub(crate) fn normalize_effort(supported: &[&'static str], requested: &str) -> String {
    if requested.is_empty() || requested == "default" {
        return "default".to_string();
    }
    if supported.contains(&requested) {
        return requested.to_string();
    }
    let fallbacks: &[&str] = match requested {
        "none" => &["minimal", "low", "medium", "high", "xhigh", "max"],
        "minimal" => &["low", "none", "medium", "high", "xhigh", "max"],
        "low" => &["minimal", "none", "medium", "high", "xhigh", "max"],
        "medium" => &["low", "high", "minimal", "none", "xhigh", "max"],
        "high" => &["medium", "xhigh", "max", "low", "minimal", "none"],
        "xhigh" => &["max", "high", "medium", "low", "minimal", "none"],
        "max" => &["xhigh", "high", "medium", "low", "minimal", "none"],
        _ => &[],
    };
    fallbacks
        .iter()
        .find(|candidate| supported.contains(candidate))
        .copied()
        .unwrap_or("default")
        .to_string()
}
