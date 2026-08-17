#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub struct RunId(pub Uuid);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub struct TurnId(pub Uuid);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub struct TaskId(pub Uuid);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub struct ToolInvocationId(pub Uuid);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash, Serialize)]
#[serde(transparent)]
pub struct OperationId(pub Uuid);

macro_rules! uuid_id_impl {
    ($name:ident) => {
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn hyphenated(&self) -> String {
                self.0.hyphenated().to_string()
            }
        }
    };
}

uuid_id_impl!(SessionId);
uuid_id_impl!(RunId);
uuid_id_impl!(TurnId);
uuid_id_impl!(TaskId);
uuid_id_impl!(ToolInvocationId);
uuid_id_impl!(OperationId);

fn parse_uuid_id(raw: &str, label: &str) -> Result<Uuid, String> {
    Uuid::parse_str(raw.trim()).map_err(|_| format!("{label} must be a valid UUID."))
}

impl SessionId {
    pub fn parse(raw: &str) -> Result<Self, String> {
        parse_uuid_id(raw, "SessionId").map(Self)
    }

    /// Parses a canonical UUID, or maps any other non-empty key to a stable SessionId.
    pub fn parse_or_stable_key(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("Session identifiers must not be empty.".to_string());
        }
        Ok(Uuid::parse_str(raw)
            .map(Self)
            .unwrap_or_else(|_| Self::from_stable_key(raw)))
    }
}

impl RunId {
    pub fn parse(raw: &str) -> Result<Self, String> {
        parse_uuid_id(raw, "RunId").map(Self)
    }
}

impl OperationId {
    pub fn parse(raw: &str) -> Result<Self, String> {
        parse_uuid_id(raw, "OperationId").map(Self)
    }
}

impl ToolInvocationId {
    pub fn from_stable_key(key: &str) -> Self {
        Self(Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes()))
    }
}

impl SessionId {
    /// Maps a host-owned logical session key to a stable canonical identity.
    /// This is intentionally session-scoped and must never be derived from a RunId.
    pub fn from_stable_key(key: &str) -> Self {
        Self(Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes()))
    }
}

pub fn canonicalize_chat_session_id(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Chat requests require a backend-owned chatSessionId.".to_string());
    }
    let parsed = Uuid::parse_str(raw)
        .map_err(|_| "Chat requests require a valid UUID chatSessionId.".to_string())?;
    let canonical = parsed.hyphenated().to_string();
    if raw != canonical {
        return Err("Chat requests require a canonical lowercase UUID chatSessionId.".to_string());
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_session_ids_must_be_canonical_uuids() {
        assert_eq!(
            canonicalize_chat_session_id("7dc513ee-45ed-4efe-a8f4-17e811c7459d").unwrap(),
            "7dc513ee-45ed-4efe-a8f4-17e811c7459d"
        );
        assert!(canonicalize_chat_session_id("7DC513EE-45ED-4EFE-A8F4-17E811C7459D").is_err());
        assert!(canonicalize_chat_session_id("not-a-uuid").is_err());
        assert!(canonicalize_chat_session_id("").is_err());
    }

    #[test]
    fn bug_agent_035_runtime_ids_are_uuid_newtypes_not_timestamp_strings() {
        let session = SessionId::new();
        let run = RunId::new();
        let turn = TurnId::new();
        let task = TaskId::new();
        let tool = ToolInvocationId::new();
        let operation = OperationId::new();
        assert_ne!(session.hyphenated(), run.hyphenated());
        assert_eq!(session.hyphenated().len(), 36);
        assert!(Uuid::parse_str(&session.hyphenated()).is_ok());
        assert_ne!(turn.0, task.0);
        assert_ne!(tool.0, operation.0);
        for id in [
            session.hyphenated(),
            run.hyphenated(),
            turn.hyphenated(),
            task.hyphenated(),
            tool.hyphenated(),
            operation.hyphenated(),
        ] {
            assert!(!id.chars().all(|ch| ch.is_ascii_digit()));
        }
    }

    #[test]
    fn bug_agent_033_run_ids_do_not_collide_across_10k_allocations() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for _ in 0..10_000 {
            assert!(seen.insert(RunId::new().hyphenated()));
        }
        assert_eq!(seen.len(), 10_000);
        let cancel_target = seen.iter().next().unwrap().clone();
        let remaining: Vec<_> = seen
            .iter()
            .filter(|id| *id != &cancel_target)
            .cloned()
            .collect();
        assert_eq!(remaining.len(), 9_999);
        assert!(!remaining.contains(&cancel_target));
    }

    #[test]
    fn external_tool_identity_maps_to_a_stable_canonical_id() {
        let first = ToolInvocationId::from_stable_key("provider-call-1");
        let replay = ToolInvocationId::from_stable_key("provider-call-1");
        let second = ToolInvocationId::from_stable_key("provider-call-2");

        assert_eq!(first, replay);
        assert_ne!(first, second);
    }

    #[test]
    fn logical_session_identity_is_stable_and_distinct_from_runs() {
        let first = SessionId::from_stable_key("conversation-a");
        let resumed = SessionId::from_stable_key("conversation-a");
        let other = SessionId::from_stable_key("conversation-b");

        assert_eq!(first, resumed);
        assert_ne!(first, other);
        assert_ne!(first.0, RunId::new().0);
    }
}
