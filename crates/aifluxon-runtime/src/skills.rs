use aifluxon_core::{CapabilityId, SkillContribution, SkillId, SkillManifest};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredSkill {
    pub manifest: SkillManifest,
    pub contribution: SkillContribution,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MergedSkillContribution {
    pub active_skills: Vec<SkillId>,
    pub instructions: Vec<String>,
    pub tools: Vec<String>,
    pub required_capabilities: Vec<CapabilityId>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SkillRegistryError {
    #[error("The Skill identifier is already registered.")]
    Duplicate(SkillId),
    #[error("The Skill is not registered.")]
    Unknown(SkillId),
}

#[derive(Clone, Default)]
pub struct SkillRegistry {
    skills: HashMap<SkillId, RegisteredSkill>,
    active: Vec<SkillId>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        manifest: SkillManifest,
        contribution: SkillContribution,
    ) -> Result<(), SkillRegistryError> {
        if self.skills.contains_key(&manifest.id) {
            return Err(SkillRegistryError::Duplicate(manifest.id));
        }
        self.skills.insert(
            manifest.id.clone(),
            RegisteredSkill {
                manifest,
                contribution,
            },
        );
        Ok(())
    }

    pub fn resolve(&self, id: &SkillId) -> Option<&RegisteredSkill> {
        self.skills.get(id)
    }

    pub fn activate(&mut self, id: &SkillId) -> Result<(), SkillRegistryError> {
        if !self.skills.contains_key(id) {
            return Err(SkillRegistryError::Unknown(id.clone()));
        }
        if !self.active.contains(id) {
            self.active.push(id.clone());
        }
        Ok(())
    }

    pub fn merge_active(&self) -> MergedSkillContribution {
        let mut merged = MergedSkillContribution {
            active_skills: self.active.clone(),
            ..Default::default()
        };
        let mut instructions = HashSet::new();
        let mut tools = HashSet::new();
        let mut capabilities = HashSet::new();
        for id in &self.active {
            if let Some(skill) = self.skills.get(id) {
                for instruction in &skill.contribution.instructions {
                    if instructions.insert(instruction.clone()) {
                        merged.instructions.push(instruction.clone());
                    }
                }
                for tool in &skill.contribution.tools {
                    if tools.insert(tool.clone()) {
                        merged.tools.push(tool.clone());
                    }
                }
                for capability in &skill.contribution.required_capabilities {
                    if capabilities.insert(capability.clone()) {
                        merged.required_capabilities.push(capability.clone());
                    }
                }
            }
        }
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str) -> SkillManifest {
        SkillManifest {
            id: SkillId::parse(id).unwrap(),
            name: id.to_string(),
            version: None,
        }
    }

    #[test]
    fn registry_activates_and_merges_without_product_storage_fields() {
        let mut registry = SkillRegistry::new();
        registry
            .register(
                manifest("analysis"),
                SkillContribution {
                    instructions: vec!["Inspect evidence".to_string()],
                    tools: vec!["read_data".to_string()],
                    required_capabilities: vec![CapabilityId::new("data.read")],
                },
            )
            .unwrap();
        registry
            .activate(&SkillId::parse("analysis").unwrap())
            .unwrap();
        registry
            .activate(&SkillId::parse("analysis").unwrap())
            .unwrap();
        let merged = registry.merge_active();
        assert_eq!(merged.active_skills.len(), 1);
        assert_eq!(merged.tools, vec!["read_data"]);
        assert_eq!(merged.required_capabilities[0].as_str(), "data.read");
    }

    #[test]
    fn duplicate_and_unknown_skills_fail_explicitly() {
        let mut registry = SkillRegistry::new();
        registry
            .register(manifest("one"), SkillContribution::default())
            .unwrap();
        assert!(matches!(
            registry.register(manifest("one"), SkillContribution::default()),
            Err(SkillRegistryError::Duplicate(_))
        ));
        assert!(matches!(
            registry.activate(&SkillId::parse("missing").unwrap()),
            Err(SkillRegistryError::Unknown(_))
        ));
    }
}
