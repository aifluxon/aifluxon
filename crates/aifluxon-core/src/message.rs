use crate::artifact::ArtifactRef;
use crate::ids::ToolInvocationId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImageContent {
    pub artifact: ArtifactRef,
    pub mime_type: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolCall {
    #[serde(with = "tool_invocation_id_serde")]
    pub id: ToolInvocationId,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentPart {
    Text(String),
    Image(ImageContent),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<ContentPart>,
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, with = "optional_tool_invocation_id_serde")]
    pub tool_call_id: Option<ToolInvocationId>,
    pub provider_state: Option<Value>,
}

mod tool_invocation_id_serde {
    use super::*;

    pub fn serialize<S>(id: &ToolInvocationId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&id.0.hyphenated().to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ToolInvocationId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Uuid::parse_str(&raw)
            .map(ToolInvocationId)
            .map_err(serde::de::Error::custom)
    }
}

mod optional_tool_invocation_id_serde {
    use super::*;

    pub fn serialize<S>(id: &Option<ToolInvocationId>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match id {
            Some(id) => serializer.serialize_some(&id.0.hyphenated().to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<ToolInvocationId>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map(|raw| {
                Uuid::parse_str(&raw)
                    .map(ToolInvocationId)
                    .map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_message_round_trips_without_interpreting_provider_state() {
        let tool_call_id = ToolInvocationId::new();
        let message = Message {
            role: MessageRole::Assistant,
            content: vec![
                ContentPart::Text("inspect the artifact".to_string()),
                ContentPart::Image(ImageContent {
                    artifact: ArtifactRef::new("artifact://run/output-1"),
                    mime_type: "image/png".to_string(),
                }),
            ],
            tool_calls: vec![ToolCall {
                id: tool_call_id,
                name: "inspect".to_string(),
                arguments: json!({ "artifact": "artifact://run/output-1" }),
            }],
            tool_call_id: Some(tool_call_id),
            provider_state: Some(json!({
                "provider_private_cursor": { "value": 42 },
            })),
        };

        let encoded = serde_json::to_value(&message).unwrap();
        let decoded: Message = serde_json::from_value(encoded).unwrap();

        assert_eq!(decoded, message);
    }
}
