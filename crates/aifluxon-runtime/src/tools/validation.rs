use aifluxon_core::{
    PreparedToolCall, ToolDescriptor, ToolValidationError, MAX_TOOL_ARGUMENT_BYTES,
};
use jsonschema::error::ValidationErrorKind;
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
        return Err(ToolValidationError::SchemaViolation {
            kind: "type".to_string(),
            path: String::new(),
            message: "Tool arguments must be a JSON object.".to_string(),
        });
    }
    let mut relaxed = schema.clone();
    if let Some(object) = relaxed.as_object_mut() {
        object.insert("additionalProperties".to_string(), json!(true));
        object.entry("type").or_insert_with(|| json!("object"));
    }
    match jsonschema::validator_for(&relaxed) {
        Ok(validator) => validator
            .iter_errors(arguments)
            .next()
            .map(schema_violation)
            .map_or(Ok(()), Err),
        Err(_) => Ok(()),
    }
}

fn schema_violation(error: jsonschema::ValidationError<'_>) -> ToolValidationError {
    let mut path = error.instance_path().as_str().to_string();
    let kind = match error.kind() {
        ValidationErrorKind::Required { property } => {
            if let Some(property) = property.as_str() {
                path.push('/');
                path.push_str(&escape_json_pointer_segment(property));
            }
            "required"
        }
        ValidationErrorKind::Type { .. } => "type",
        ValidationErrorKind::Enum { .. } => "enum",
        ValidationErrorKind::AdditionalProperties { .. }
        | ValidationErrorKind::UnevaluatedProperties { .. } => "additional_properties",
        ValidationErrorKind::MinLength { .. } => "min_length",
        ValidationErrorKind::MaxLength { .. } => "max_length",
        ValidationErrorKind::Minimum { .. } | ValidationErrorKind::ExclusiveMinimum { .. } => {
            "minimum"
        }
        ValidationErrorKind::Maximum { .. } | ValidationErrorKind::ExclusiveMaximum { .. } => {
            "maximum"
        }
        ValidationErrorKind::MinItems { .. } => "min_items",
        ValidationErrorKind::MaxItems { .. } => "max_items",
        ValidationErrorKind::Pattern { .. } => "pattern",
        ValidationErrorKind::Format { .. } => "format",
        _ => "schema",
    };
    ToolValidationError::SchemaViolation {
        kind: kind.to_string(),
        path,
        message: error.to_string(),
    }
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
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
        let missing = prepare_tool_call(&descriptor, r#"{"mode":"brief"}"#).unwrap_err();
        assert_eq!(missing.kind(), "required");
        assert_eq!(missing.path(), Some("/key"));
        let wrong_type = prepare_tool_call(&descriptor, r#"{"key":1}"#).unwrap_err();
        assert_eq!(wrong_type.kind(), "type");
        assert_eq!(wrong_type.path(), Some("/key"));
        let unknown_enum =
            prepare_tool_call(&descriptor, r#"{"key":"a","mode":"other"}"#).unwrap_err();
        assert_eq!(unknown_enum.kind(), "enum");
        assert_eq!(unknown_enum.path(), Some("/mode"));
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

    #[test]
    fn nested_schema_errors_keep_the_exact_json_pointer_path() {
        let mut descriptor = descriptor(ToolEffect::PureRead);
        descriptor.input_schema = json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "summary": { "type": "string" } },
                        "required": ["summary"]
                    }
                }
            },
            "required": ["edits"]
        });

        let wrong_type =
            prepare_tool_call(&descriptor, r#"{"edits":[{"summary":null}]}"#).unwrap_err();
        assert_eq!(wrong_type.kind(), "type");
        assert_eq!(wrong_type.path(), Some("/edits/0/summary"));

        let missing = prepare_tool_call(&descriptor, r#"{"edits":[{}]}"#).unwrap_err();
        assert_eq!(missing.kind(), "required");
        assert_eq!(missing.path(), Some("/edits/0/summary"));
    }
}
