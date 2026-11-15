use std::time::Duration;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BeginRunError {
    Busy,
    LockPoisoned,
}

/// Machine-readable failure classification, independent of display text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProviderErrorCode {
    Other,
    Cancelled,
    RateLimited,
    TransientHttp,
    TransientTransport,
    Transport,
    Http,
    StreamClosed,
    InvalidToolCall,
    EmptyResponse,
    MalformedResponse,
    ResourceUnavailable,
    OutputLimit,
    OutputIncomplete,
    FinishError,
    Deadline,
    TerminalError,
    InvalidTurn,
    BudgetExceeded,
}

impl ProviderErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Other => "PROVIDER_RESPONSE_ERROR",
            Self::Cancelled => "PROVIDER_CANCELLED",
            Self::RateLimited => "PROVIDER_RATE_LIMITED",
            Self::TransientHttp => "PROVIDER_TRANSIENT_HTTP",
            Self::TransientTransport => "PROVIDER_TRANSIENT_TRANSPORT",
            Self::Transport => "PROVIDER_TRANSPORT",
            Self::Http => "PROVIDER_HTTP",
            Self::StreamClosed => "PROVIDER_STREAM_CLOSED",
            Self::InvalidToolCall => "PROVIDER_INVALID_TOOL_CALL",
            Self::EmptyResponse => "PROVIDER_EMPTY_RESPONSE",
            Self::MalformedResponse => "PROVIDER_MALFORMED_RESPONSE",
            Self::ResourceUnavailable => "PROVIDER_RESOURCE_UNAVAILABLE",
            Self::OutputLimit => "PROVIDER_OUTPUT_LIMIT",
            Self::OutputIncomplete => "PROVIDER_OUTPUT_INCOMPLETE",
            Self::FinishError => "PROVIDER_FINISH_ERROR",
            Self::Deadline => "PROVIDER_DEADLINE",
            Self::TerminalError => "PROVIDER_TERMINAL_ERROR",
            Self::InvalidTurn => "PROVIDER_INVALID_TURN",
            Self::BudgetExceeded => "RUN_BUDGET_EXCEEDED",
        }
    }

    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited
                | Self::TransientHttp
                | Self::TransientTransport
                | Self::StreamClosed
                | Self::InvalidToolCall
                | Self::EmptyResponse
                | Self::MalformedResponse
                | Self::ResourceUnavailable
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[error("{message}")]
pub struct ProviderError {
    pub message: String,
    pub code: ProviderErrorCode,
    pub http_status: Option<u16>,
    pub retry_after: Option<Duration>,
}

impl ProviderError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::new(ProviderErrorCode::Other, message)
    }

    pub fn new(code: ProviderErrorCode, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code,
            http_status: None,
            retry_after: None,
        }
    }

    pub fn cancelled() -> Self {
        Self::new(ProviderErrorCode::Cancelled, "Run was cancelled.")
    }

    pub fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self.code = match status {
            429 => ProviderErrorCode::RateLimited,
            408 | 500 | 502 | 503 | 504 => ProviderErrorCode::TransientHttp,
            _ => ProviderErrorCode::Http,
        };
        self
    }

    pub fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_independent_of_display_text() {
        let message = "PROVIDER_STREAM_CLOSED: merely quoted by a remote server";
        let plain = ProviderError::message(message);
        assert_eq!(plain.code, ProviderErrorCode::Other);
        assert!(!plain.code.is_retryable());
        let structured = ProviderError::new(ProviderErrorCode::StreamClosed, "localized failure");
        assert!(structured.code.is_retryable());
        assert_eq!(structured.to_string(), "localized failure");
        assert_eq!(plain.to_string(), message);
        assert_eq!(
            ProviderError::cancelled().code,
            ProviderErrorCode::Cancelled
        );
    }

    #[test]
    fn http_metadata_classifies_rate_limits_and_transient_statuses() {
        for (status, code, retryable) in [
            (429, ProviderErrorCode::RateLimited, true),
            (503, ProviderErrorCode::TransientHttp, true),
            (401, ProviderErrorCode::Http, false),
            (400, ProviderErrorCode::Http, false),
        ] {
            let error = ProviderError::message("redacted")
                .with_http_status(status)
                .with_retry_after(Some(Duration::from_secs(2)));
            assert_eq!(error.code, code);
            assert_eq!(error.http_status, Some(status));
            assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
            assert_eq!(error.code.is_retryable(), retryable);
            assert_eq!(error.to_string(), "redacted");
        }
    }
}
