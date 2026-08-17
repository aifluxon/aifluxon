use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BeginRunError {
    Busy,
    LockPoisoned,
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[error("{message}")]
pub struct ProviderError {
    pub message: String,
}

impl ProviderError {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
