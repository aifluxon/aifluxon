use super::executor::ToolInvocation;
use aifluxon_core::{
    AgentFilesystemScope, CapabilityId, ExecutionAuthority, PendingOperationDraft, PermissionMode,
    ToolDescriptor,
};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub struct ParallelToolCall<'a> {
    pub descriptor: &'a ToolDescriptor,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedResourceFacts {
    pub resource: Option<String>,
    pub cwd: Option<String>,
    pub network_target: Option<String>,
    pub prepared_effect: bool,
}

pub struct ToolPolicyInput<'a> {
    pub descriptor: &'a ToolDescriptor,
    pub invocation: &'a ToolInvocation,
    pub facts: &'a ResolvedResourceFacts,
    pub authority: &'a ExecutionAuthority,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { operation: PendingOperationDraft },
}

pub trait ToolPolicy: Send + Sync {
    fn resolve_facts(
        &self,
        _descriptor: &ToolDescriptor,
        _invocation: &ToolInvocation,
        _authority: &ExecutionAuthority,
    ) -> Result<ResolvedResourceFacts, String> {
        Ok(ResolvedResourceFacts::default())
    }

    fn evaluate(&self, input: &ToolPolicyInput<'_>) -> ToolDecision;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AllowAllToolPolicy;

impl ToolPolicy for AllowAllToolPolicy {
    fn evaluate(&self, _input: &ToolPolicyInput<'_>) -> ToolDecision {
        ToolDecision::Allow
    }
}

pub fn missing_required_capabilities(input: &ToolPolicyInput<'_>) -> HashSet<CapabilityId> {
    input
        .descriptor
        .required_capabilities
        .iter()
        .filter(|capability| !input.authority.grants(capability))
        .cloned()
        .collect()
}

pub fn permission_mode_from_str(value: &str) -> PermissionMode {
    match value {
        "managed" => PermissionMode::Managed,
        "trusted" => PermissionMode::Trusted,
        _ => PermissionMode::Default,
    }
}

pub fn filesystem_scope(
    has_scoped_selection: bool,
    permission_mode: PermissionMode,
    has_workspace: bool,
) -> AgentFilesystemScope {
    if has_scoped_selection {
        AgentFilesystemScope::ScopedSelection
    } else if permission_mode == PermissionMode::Trusted {
        AgentFilesystemScope::Trusted
    } else if !has_workspace {
        AgentFilesystemScope::Isolated
    } else {
        AgentFilesystemScope::Workspace
    }
}

pub fn shell_filesystem_scope(
    permission_mode: PermissionMode,
    has_workspace: bool,
) -> AgentFilesystemScope {
    if permission_mode == PermissionMode::Trusted {
        AgentFilesystemScope::Trusted
    } else if !has_workspace {
        AgentFilesystemScope::Isolated
    } else {
        AgentFilesystemScope::Workspace
    }
}

pub fn execution_authority(
    has_scoped_selection: bool,
    permission_mode: PermissionMode,
    has_workspace: bool,
) -> ExecutionAuthority {
    ExecutionAuthority {
        permission_mode,
        filesystem_scope: filesystem_scope(has_scoped_selection, permission_mode, has_workspace),
        capabilities: HashSet::new(),
    }
}

pub fn can_run_tool_calls_in_parallel(calls: &[ParallelToolCall<'_>]) -> bool {
    calls.len() > 1 && calls.iter().all(|call| call.descriptor.parallel_safe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::ToolEffect;
    use serde_json::json;

    struct ScopedPolicy;

    impl ToolPolicy for ScopedPolicy {
        fn evaluate(&self, input: &ToolPolicyInput<'_>) -> ToolDecision {
            if !missing_required_capabilities(input).is_empty() {
                return ToolDecision::Deny {
                    reason: "Missing Host capability grant.".to_string(),
                };
            }
            if input.facts.resource.as_deref() == Some("outside-grant") {
                return ToolDecision::Deny {
                    reason: "Resolved resource is outside the Host grant.".to_string(),
                };
            }
            ToolDecision::Allow
        }
    }

    fn descriptor(name: &str, effect: ToolEffect, parallel_safe: bool) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            description: name.to_string(),
            input_schema: json!({ "type": "object" }),
            effect,
            required_capabilities: Vec::new(),
            parallel_safe,
        }
    }

    #[test]
    fn parallel_safety_comes_only_from_descriptors() {
        let unsafe_read = descriptor("read_alpha", ToolEffect::PureRead, false);
        let safe_write = descriptor("write_beta", ToolEffect::FsWrite, true);
        let safe_external = descriptor("notify_gamma", ToolEffect::ExternalSideEffect, true);

        assert!(!can_run_tool_calls_in_parallel(&[
            ParallelToolCall {
                descriptor: &unsafe_read,
            },
            ParallelToolCall {
                descriptor: &safe_write,
            },
        ]));
        assert!(can_run_tool_calls_in_parallel(&[
            ParallelToolCall {
                descriptor: &safe_write,
            },
            ParallelToolCall {
                descriptor: &safe_external,
            },
        ]));
        assert!(!can_run_tool_calls_in_parallel(&[ParallelToolCall {
            descriptor: &safe_write,
        }]));
    }

    #[test]
    fn workspace_scope_does_not_inherit_scoped_or_trusted_access() {
        assert_eq!(
            filesystem_scope(false, PermissionMode::Default, true),
            AgentFilesystemScope::Workspace
        );
        assert_eq!(
            shell_filesystem_scope(PermissionMode::Default, true),
            AgentFilesystemScope::Workspace
        );
        assert_eq!(
            filesystem_scope(false, PermissionMode::Default, false),
            AgentFilesystemScope::Isolated
        );
        assert_eq!(
            filesystem_scope(true, PermissionMode::Trusted, true),
            AgentFilesystemScope::ScopedSelection
        );
        assert_eq!(
            shell_filesystem_scope(PermissionMode::Trusted, true),
            AgentFilesystemScope::Trusted
        );
        assert_ne!(
            filesystem_scope(true, PermissionMode::Trusted, true),
            shell_filesystem_scope(PermissionMode::Trusted, true)
        );
    }

    #[test]
    fn final_policy_uses_descriptor_invocation_facts_and_host_grants_not_effect_alone() {
        let capability = CapabilityId::new("workspace.read");
        let mut descriptor = descriptor("inspect", ToolEffect::FsRead, true);
        descriptor.required_capabilities = vec![capability.clone()];
        let invocation = ToolInvocation {
            id: aifluxon_core::ToolInvocationId::new(),
            name: descriptor.name.clone(),
            arguments: json!({ "path": "notes.txt" }),
            effect: descriptor.effect,
        };
        let mut authority = execution_authority(false, PermissionMode::Default, true);
        let allowed_facts = ResolvedResourceFacts {
            resource: Some("inside-grant".to_string()),
            ..Default::default()
        };
        let denied_facts = ResolvedResourceFacts {
            resource: Some("outside-grant".to_string()),
            ..Default::default()
        };
        let policy = ScopedPolicy;

        let missing_grant = ToolPolicyInput {
            descriptor: &descriptor,
            invocation: &invocation,
            facts: &allowed_facts,
            authority: &authority,
        };
        assert!(matches!(
            policy.evaluate(&missing_grant),
            ToolDecision::Deny { .. }
        ));

        authority.capabilities.insert(capability);
        let outside = ToolPolicyInput {
            descriptor: &descriptor,
            invocation: &invocation,
            facts: &denied_facts,
            authority: &authority,
        };
        assert!(matches!(
            policy.evaluate(&outside),
            ToolDecision::Deny { .. }
        ));

        let allowed = ToolPolicyInput {
            descriptor: &descriptor,
            invocation: &invocation,
            facts: &allowed_facts,
            authority: &authority,
        };
        assert_eq!(policy.evaluate(&allowed), ToolDecision::Allow);
    }
}
