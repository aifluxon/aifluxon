use crate::CapabilityId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillScope {
    Project,
    Global,
}

impl SkillScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "project" => Ok(Self::Project),
            "global" => Ok(Self::Global),
            _ => Err("Skill scope must be `project` or `global`.".to_string()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillScopeOption {
    pub scope: SkillScope,
    pub target_path: String,
    pub available: bool,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentFilesystemScope {
    Workspace,
    Isolated,
    ScopedSelection,
    Trusted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionMode {
    Default,
    Managed,
    Trusted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionAuthority {
    pub permission_mode: PermissionMode,
    pub filesystem_scope: AgentFilesystemScope,
    pub capabilities: HashSet<CapabilityId>,
}

impl ExecutionAuthority {
    pub fn grants(&self, capability: &CapabilityId) -> bool {
        self.capabilities.contains(capability)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_scope_round_trips_project_and_global() {
        assert_eq!(SkillScope::parse("project").unwrap(), SkillScope::Project);
        assert_eq!(SkillScope::parse("GLOBAL").unwrap(), SkillScope::Global);
        assert!(SkillScope::parse("workspace").is_err());
        assert_eq!(SkillScope::Project.as_str(), "project");
    }
}
