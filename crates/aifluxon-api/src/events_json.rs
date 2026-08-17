use aifluxon_core::{
    Message, OperationSnapshot, PendingOperation, RunEvent, RunEventEnvelope, RunState,
};
use serde_json::{json, Value};

pub fn envelope_to_json(envelope: &RunEventEnvelope) -> Value {
    let mut object = event_to_json(&envelope.event);
    object["sequence"] = json!(envelope.sequence);
    object["run_id"] = json!(envelope.run_id.hyphenated());
    object
}

pub fn event_to_json(event: &RunEvent) -> Value {
    match event {
        RunEvent::RunStarted { context } => json!({
            "type": "run_started",
            "session_id": context.session_id.map(|id| id.hyphenated()),
            "parent_run_id": context.parent_run_id.map(|id| id.hyphenated()),
        }),
        RunEvent::StateChanged { state } => json!({
            "type": "state_changed",
            "state": run_state_name(*state),
        }),
        RunEvent::ModelDelta { delta } => json!({
            "type": "text_delta",
            "delta": delta,
        }),
        RunEvent::ReasoningDelta { delta } => json!({
            "type": "reasoning_delta",
            "delta": delta,
        }),
        RunEvent::ToolStarted {
            invocation_id,
            name,
            arguments,
        } => json!({
            "type": "tool_started",
            "invocation_id": invocation_id.hyphenated(),
            "name": name,
            "arguments": arguments,
        }),
        RunEvent::ToolFinished {
            invocation_id,
            name,
            result,
        } => json!({
            "type": "tool_finished",
            "invocation_id": invocation_id.hyphenated(),
            "name": name,
            "result": result,
        }),
        RunEvent::OperationRequested { operation } => json!({
            "type": "operation_requested",
            "operation": pending_operation_to_json(operation),
        }),
        RunEvent::UsageUpdated { usage } => json!({
            "type": "usage_updated",
            "usage": usage,
        }),
        RunEvent::ArtifactProduced { artifact } => json!({
            "type": "artifact_produced",
            "artifact": artifact.as_str(),
        }),
        RunEvent::Completed { output } => json!({
            "type": "completed",
            "output": output.iter().map(message_to_json).collect::<Vec<_>>(),
        }),
        RunEvent::Failed { message } => json!({
            "type": "failed",
            "message": message,
        }),
        RunEvent::Cancelled => json!({
            "type": "cancelled",
        }),
    }
}

pub fn pending_operation_to_json(operation: &PendingOperation) -> Value {
    json!({
        "id": operation.id.hyphenated(),
        "run_id": operation.run_id.hyphenated(),
        "invocation_id": operation.invocation_id.map(|id| id.hyphenated()),
        "effect": format!("{:?}", operation.effect),
        "mode": match operation.mode {
            aifluxon_core::OperationMode::BlockingApproval => "blocking_approval",
            aifluxon_core::OperationMode::DeferredCommit => "deferred_commit",
        },
        "summary": operation.summary,
        "payload": operation.payload,
    })
}

pub fn operation_snapshot_to_json(snapshot: &OperationSnapshot) -> Value {
    let mut value = pending_operation_to_json(&snapshot.operation);
    value["state"] = json!(format!("{:?}", snapshot.state));
    value
}

fn run_state_name(state: RunState) -> &'static str {
    match state {
        RunState::Running => "running",
        RunState::AwaitingOperation => "awaiting_operation",
        RunState::Completed => "completed",
        RunState::Failed => "failed",
        RunState::Cancelled => "cancelled",
    }
}

fn message_to_json(message: &Message) -> Value {
    serde_json::to_value(message).unwrap_or(Value::Null)
}
