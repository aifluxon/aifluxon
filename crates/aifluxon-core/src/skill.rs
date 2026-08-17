use crate::CapabilityId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SkillId(String);

impl SkillId {
    pub fn parse(value: impl Into<String>) -> Result<Self, SkillIdError> {
        let value = value.into();
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 64
            || (!bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit())
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
            || value.ends_with('-')
            || value.contains("--")
        {
            return Err(SkillIdError);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Skill identifiers must be 1-64 lowercase ASCII letters, digits, or single hyphens.")]
pub struct SkillIdError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillManifest {
    pub id: SkillId,
    pub name: String,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillContribution {
    pub instructions: Vec<String>,
    pub tools: Vec<String>,
    pub required_capabilities: Vec<CapabilityId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_skill_ids_are_rejected_without_host_paths() {
        for invalid in ["", "Upper", "../escape", "double--dash", "ends-"] {
            assert!(SkillId::parse(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            SkillId::parse("data-analysis-2").unwrap().as_str(),
            "data-analysis-2"
        );
    }
}
