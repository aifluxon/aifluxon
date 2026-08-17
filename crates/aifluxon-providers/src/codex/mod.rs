use crate::strategy::ModelApiCapabilities;
use serde_json::{json, Value};

pub fn capabilities() -> ModelApiCapabilities {
    ModelApiCapabilities::RESPONSES_ONLY
}

pub fn apply_responses_contract(body: &mut Value, reasoning_effort: &str) {
    body["include"] = json!(["reasoning.encrypted_content"]);
    if !body.get("reasoning").is_some_and(Value::is_object) {
        body["reasoning"] = json!({});
    }
    body["reasoning"]["summary"] = json!(if reasoning_effort == "none" {
        "none"
    } else {
        "auto"
    });
    if !body.get("text").is_some_and(Value::is_object) {
        body["text"] = json!({});
    }
    body["text"]["verbosity"] = json!("medium");
}

pub fn should_continue_end_turn(opaque: &Value) -> bool {
    opaque
        .get("end_turn")
        .and_then(Value::as_bool)
        .is_some_and(|end_turn| !end_turn)
}
