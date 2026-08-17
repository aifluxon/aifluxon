use aifluxon_core::{
    PreparedToolCall, ToolDescriptor, ToolValidationError, MAX_TOOL_ARGUMENT_BYTES,
};
use serde_json::{json, Value};

pub fn parse_tool_arguments(raw: &str) -> Result<Value, ToolValidationError> {
    if raw.len() > MAX_TOOL_ARGUMENT_BYTES {
        return Err(ToolValidationError::OversizedArgument);
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(raw).map_err(|_| ToolValidationError::InvalidJson)
}

pub fn validate_tool_arguments(
    schema: &Value,
    arguments: &Value,
) -> Result<(), ToolValidationError> {
    if !arguments.is_object() {
        return Err(ToolValidationError::SchemaInvalid {
            message: "Tool arguments must be a JSON object.".to_string(),
        });
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required.iter().filter_map(Value::as_str) {
            if arguments.get(field).is_none() {
                return Err(ToolValidationError::MissingRequiredField {
                    field: field.to_string(),
                });
            }
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (field, property) in properties {
            let Some(value) = arguments.get(field) else {
                continue;
            };
            if let Some(allowed) = property.get("enum").and_then(Value::as_array) {
                if !allowed.iter().any(|candidate| candidate == value) {
                    return Err(ToolValidationError::UnknownEnum {
                        field: field.clone(),
                    });
                }
            }
            if !json_type_matches(property.get("type"), value) {
                return Err(ToolValidationError::WrongFieldType {
                    field: field.clone(),
                });
            }
        }
    }
    let mut relaxed = schema.clone();
    if let Some(object) = relaxed.as_object_mut() {
        object.insert("additionalProperties".to_string(), json!(true));
        object.entry("type").or_insert_with(|| json!("object"));
    }
    match jsonschema::validator_for(&relaxed) {
        Ok(validator) => {
            validator
                .validate(arguments)
                .map_err(|error| ToolValidationError::SchemaInvalid {
                    message: error.to_string(),
                })
        }
        Err(_) => Ok(()),
    }
}

pub fn prepare_tool_call(
    descriptor: &ToolDescriptor,
    raw_arguments: &str,
) -> Result<PreparedToolCall, ToolValidationError> {
    let arguments = parse_tool_arguments(raw_arguments)?;
    validate_tool_arguments(&descriptor.input_schema, &arguments)?;
    Ok(PreparedToolCall {
        name: descriptor.name.clone(),
        arguments,
        effect: descriptor.effect,
    })
}

fn json_type_matches(expected: Option<&Value>, value: &Value) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    match expected {
        Value::String(kind) => json_kind_matches(kind, value),
        Value::Array(kinds) => kinds.iter().any(|kind| {
            kind.as_str()
                .is_some_and(|kind| json_kind_matches(kind, value))
        }),
        _ => true,
    }
}

fn json_kind_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::ToolEffect;

    fn descriptor(effect: ToolEffect) -> ToolDescriptor {
        ToolDescriptor {
            name: "lookup_record".to_string(),
            description: "Look up one record".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "mode": { "type": "string", "enum": ["brief", "full"] }
                },
                "required": ["key"]
            }),
            effect,
            required_capabilities: Vec::new(),
            parallel_safe: true,
        }
    }

    #[test]
    fn schema_validation_rejects_malformed_missing_and_invalid_fields() {
        let descriptor = descriptor(ToolEffect::PureRead);
        assert_eq!(
            prepare_tool_call(&descriptor, "{").unwrap_err(),
            ToolValidationError::InvalidJson
        );
        assert_eq!(
            prepare_tool_call(&descriptor, r#"{"mode":"brief"}"#).unwrap_err(),
            ToolValidationError::MissingRequiredField {
                field: "key".to_string()
            }
        );
        assert_eq!(
            prepare_tool_call(&descriptor, r#"{"key":1}"#).unwrap_err(),
            ToolValidationError::WrongFieldType {
                field: "key".to_string()
            }
        );
        assert_eq!(
            prepare_tool_call(&descriptor, r#"{"key":"a","mode":"other"}"#).unwrap_err(),
            ToolValidationError::UnknownEnum {
                field: "mode".to_string()
            }
        );
        assert_eq!(
            prepare_tool_call(&descriptor, &"x".repeat(MAX_TOOL_ARGUMENT_BYTES + 1)).unwrap_err(),
            ToolValidationError::OversizedArgument
        );
    }

    #[test]
    fn prepared_effect_comes_only_from_the_descriptor() {
        let first =
            prepare_tool_call(&descriptor(ToolEffect::PureRead), r#"{"key":"same-name"}"#).unwrap();
        let second = prepare_tool_call(
            &descriptor(ToolEffect::ExternalSideEffect),
            r#"{"key":"same-name"}"#,
        )
        .unwrap();
        assert_eq!(first.effect, ToolEffect::PureRead);
        assert_eq!(second.effect, ToolEffect::ExternalSideEffect);
    }

    #[test]
    fn empty_arguments_are_an_empty_object() {
        let mut descriptor = descriptor(ToolEffect::PureRead);
        descriptor.input_schema = json!({ "type": "object" });
        assert_eq!(
            prepare_tool_call(&descriptor, "").unwrap().arguments,
            json!({})
        );
    }
}
