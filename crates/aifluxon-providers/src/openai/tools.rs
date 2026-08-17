use aifluxon_core::{CapabilityId, ToolDescriptor, ToolEffect};
use serde_json::{json, Value};

pub fn descriptor_from_openai_tool(
    tool: &Value,
    effect: ToolEffect,
    required_capabilities: Vec<CapabilityId>,
    parallel_safe: bool,
) -> Option<ToolDescriptor> {
    let function = tool.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let description = function
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let input_schema = function
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    Some(ToolDescriptor {
        name,
        description,
        input_schema,
        effect,
        required_capabilities,
        parallel_safe,
    })
}

pub fn schema_from_openai_tools<'a>(tools: &'a Value, name: &str) -> Option<&'a Value> {
    tools.as_array()?.iter().find_map(|tool| {
        let function = tool.get("function")?;
        if function.get("name")?.as_str()? == name {
            function.get("parameters")
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_preserves_explicit_descriptor_metadata() {
        let tool = json!({
            "type": "function",
            "function": {
                "name": "query_inventory",
                "description": "Query inventory",
                "parameters": {
                    "type": "object",
                    "properties": { "sku": { "type": "string" } },
                    "required": ["sku"]
                }
            }
        });
        let descriptor = descriptor_from_openai_tool(
            &tool,
            ToolEffect::Network,
            vec![CapabilityId::new("inventory.read")],
            false,
        )
        .unwrap();

        assert_eq!(descriptor.name, "query_inventory");
        assert_eq!(descriptor.effect, ToolEffect::Network);
        assert!(!descriptor.parallel_safe);
        assert_eq!(
            descriptor.required_capabilities[0].as_str(),
            "inventory.read"
        );
        assert_eq!(
            schema_from_openai_tools(&json!([tool]), "query_inventory").unwrap()["required"],
            json!(["sku"])
        );
    }
}
