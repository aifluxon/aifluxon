use serde::{Deserialize, Serialize};

/// An opaque reference to an artifact owned by an embedding application.
///
/// AIFLUXON preserves this reference but does not provide artifact storage.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ArtifactRef(pub String);

impl ArtifactRef {
    pub fn new(reference: impl Into<String>) -> Self {
        Self(reference.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ArtifactRef {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ArtifactRef {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}
