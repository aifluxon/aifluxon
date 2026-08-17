use crate::capability::CapabilityId;
use serde_json::Value;

pub const MAX_TOOL_ARGUMENT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolEffect {
    PureRead,
    FsRead,
    FsWrite,
    ProcessSpawn,
    ProcessControl,
    Network,
    SettingsWrite,
    ExternalSideEffect,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub effect: ToolEffect,
    pub required_capabilities: Vec<CapabilityId>,
    pub parallel_safe: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolValidationError {
    InvalidJson,
    OversizedArgument,
    UnknownTool,
    MissingRequiredField { field: String },
    WrongFieldType { field: String },
    UnknownEnum { field: String },
    SchemaInvalid { message: String },
}

impl ToolValidationError {
    pub fn message(&self) -> String {
        match self {
            Self::InvalidJson => "Tool arguments must be valid JSON.".to_string(),
            Self::OversizedArgument => {
                format!("Tool arguments exceed the {MAX_TOOL_ARGUMENT_BYTES}-byte limit.")
            }
            Self::UnknownTool => "Unknown tool.".to_string(),
            Self::MissingRequiredField { field } => {
                format!("Tool arguments are missing required field `{field}`.")
            }
            Self::WrongFieldType { field } => {
                format!("Tool argument `{field}` has the wrong type.")
            }
            Self::UnknownEnum { field } => {
                format!("Tool argument `{field}` is not an allowed enum value.")
            }
            Self::SchemaInvalid { message } => message.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreparedToolCall {
    pub name: String,
    pub arguments: Value,
    pub effect: ToolEffect,
}
