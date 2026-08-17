use aifluxon_core::ToolDescriptor;
use serde_json::{json, Value};

pub fn descriptor_to_openai_tool(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.name,
            "description": descriptor.description,
            "parameters": descriptor.input_schema,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::{CapabilityId, ToolEffect};

    #[test]
    fn descriptor_conversion_does_not_infer_policy_from_name() {
        let descriptor = ToolDescriptor {
            name: "arbitrary_action".to_string(),
            description: "Act".to_string(),
            input_schema: json!({ "type": "object" }),
            effect: ToolEffect::ExternalSideEffect,
            required_capabilities: vec![CapabilityId::new("host.action")],
            parallel_safe: false,
        };
        let wire = descriptor_to_openai_tool(&descriptor);
        assert_eq!(wire["function"]["name"], "arbitrary_action");
        assert!(wire.get("effect").is_none());
    }
}
