use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CodexInbound {
    Response {
        id: Value,
        result: Option<Value>,
        error: Option<CodexRpcError>,
    },
    Notification {
        method: String,
        params: Value,
    },
    ServerRequest {
        id: Value,
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodexTranscriptState {
    pub items: Vec<CodexTranscriptItem>,
    pub turn_status: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexTranscriptItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub text: String,
    pub status: String,
    pub raw_json: String,
}

pub fn build_request_line(id: u64, method: &str, params: Value) -> Result<String, String> {
    serde_json::to_string(&json!({
        "id": id,
        "method": method,
        "params": params,
    }))
    .map_err(|err| format!("Failed to serialize Codex request: {err}"))
}

pub fn build_notification_line(method: &str, params: Value) -> Result<String, String> {
    serde_json::to_string(&json!({
        "method": method,
        "params": params,
    }))
    .map_err(|err| format!("Failed to serialize Codex notification: {err}"))
}

pub fn build_response_line(id: &Value, result: Value) -> Result<String, String> {
    serde_json::to_string(&json!({
        "id": id,
        "result": result,
    }))
    .map_err(|err| format!("Failed to serialize Codex response: {err}"))
}

pub fn parse_jsonrpc_line(line: &str) -> Result<CodexInbound, String> {
    let value: Value = serde_json::from_str(line)
        .map_err(|err| format!("Malformed Codex JSON-RPC line: {err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "Codex JSON-RPC message must be an object.".to_string())?;

    let id = object.get("id").cloned();
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);

    match (id, method) {
        (Some(id), Some(method)) => Ok(CodexInbound::ServerRequest {
            id,
            method,
            params: object.get("params").cloned().unwrap_or_else(|| json!({})),
        }),
        (Some(id), None) => {
            let error = object
                .get("error")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|err| format!("Malformed Codex error response: {err}"))?;
            Ok(CodexInbound::Response {
                id,
                result: object.get("result").cloned(),
                error,
            })
        }
        (None, Some(method)) => Ok(CodexInbound::Notification {
            method,
            params: object.get("params").cloned().unwrap_or_else(|| json!({})),
        }),
        (None, None) => Err("Codex JSON-RPC message needs an id or method.".to_string()),
    }
}

impl CodexTranscriptState {
    pub fn apply_notification(&mut self, method: &str, params: &Value) {
        match method {
            "item/started" => {
                if let Some(item) = params.get("item") {
                    self.upsert_item(item_to_transcript(item));
                }
            }
            "item/completed" => {
                if let Some(item) = params.get("item") {
                    self.upsert_item(item_to_transcript(item));
                }
            }
            "item/agentMessage/delta" => {
                self.append_delta(params, "agentMessage", "Agent", extract_delta_text(params));
            }
            "item/plan/delta" => {
                self.append_delta(params, "plan", "Plan", extract_delta_text(params));
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                self.append_delta(params, "reasoning", "Reasoning", extract_delta_text(params));
            }
            "item/commandExecution/outputDelta" => {
                self.append_delta(
                    params,
                    "commandExecution",
                    "Command",
                    extract_delta_text(params),
                );
            }
            "item/fileChange/outputDelta" => {
                self.append_delta(
                    params,
                    "fileChange",
                    "File Change",
                    extract_delta_text(params),
                );
            }
            "turn/started" => self.turn_status = Some("inProgress".to_string()),
            "turn/completed" => {
                self.turn_status = params
                    .pointer("/turn/status")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| Some("completed".to_string()));
                if let Some(message) = params
                    .pointer("/turn/error/message")
                    .and_then(Value::as_str)
                {
                    self.errors.push(message.to_string());
                }
            }
            "error" => {
                if let Some(message) = params.pointer("/error/message").and_then(Value::as_str) {
                    self.errors.push(message.to_string());
                }
            }
            _ => {}
        }
    }

    fn append_delta(&mut self, params: &Value, kind: &str, title: &str, delta: String) {
        if delta.is_empty() {
            return;
        }
        let item_id = params
            .get("itemId")
            .or_else(|| params.get("item_id"))
            .or_else(|| params.get("id"))
            .and_then(Value::as_str)
            .unwrap_or(title)
            .to_string();

        if let Some(existing) = self.items.iter_mut().find(|item| item.id == item_id) {
            existing.text.push_str(&delta);
            existing.raw_json = compact_json(params);
            return;
        }

        self.items.push(CodexTranscriptItem {
            id: item_id,
            kind: kind.to_string(),
            title: title.to_string(),
            text: delta,
            status: "inProgress".to_string(),
            raw_json: compact_json(params),
        });
    }

    fn upsert_item(&mut self, item: CodexTranscriptItem) {
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|existing| existing.id == item.id)
        {
            *existing = item;
        } else {
            self.items.push(item);
        }
    }
}

pub fn item_to_transcript(item: &Value) -> CodexTranscriptItem {
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(kind.as_str())
        .to_string();
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let title = item_title(&kind, item);
    let text = item_text(&kind, item);

    CodexTranscriptItem {
        id,
        kind,
        title,
        text,
        status,
        raw_json: compact_json(item),
    }
}

fn item_title(kind: &str, item: &Value) -> String {
    match kind {
        "userMessage" => "You".to_string(),
        "agentMessage" => "Agent".to_string(),
        "reasoning" => "Reasoning".to_string(),
        "plan" => "Plan".to_string(),
        "commandExecution" => item
            .get("command")
            .and_then(Value::as_str)
            .map(|command| format!("Command: {command}"))
            .unwrap_or_else(|| "Command".to_string()),
        "fileChange" => "File Changes".to_string(),
        "mcpToolCall" => {
            let server = item.get("server").and_then(Value::as_str).unwrap_or("mcp");
            let tool = item.get("tool").and_then(Value::as_str).unwrap_or("tool");
            format!("{server}/{tool}")
        }
        "dynamicToolCall" => item
            .get("tool")
            .and_then(Value::as_str)
            .map(|tool| format!("Tool: {tool}"))
            .unwrap_or_else(|| "Tool".to_string()),
        "webSearch" => item
            .get("query")
            .and_then(Value::as_str)
            .map(|query| format!("Web Search: {query}"))
            .unwrap_or_else(|| "Web Search".to_string()),
        "enteredReviewMode" => "Review Started".to_string(),
        "exitedReviewMode" => "Review".to_string(),
        "contextCompaction" => "Compaction".to_string(),
        other => other.to_string(),
    }
}

fn item_text(kind: &str, item: &Value) -> String {
    match kind {
        "userMessage" => input_content_text(item.get("content")),
        "agentMessage" | "plan" => item
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "reasoning" => reasoning_text(item),
        "commandExecution" => command_text(item),
        "fileChange" => file_change_text(item),
        "mcpToolCall" | "dynamicToolCall" => tool_call_text(item),
        "enteredReviewMode" | "exitedReviewMode" => item
            .get("review")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "contextCompaction" => "Conversation history was compacted.".to_string(),
        _ => pretty_json(item),
    }
}

fn input_content_text(content: Option<&Value>) -> String {
    let Some(Value::Array(items)) = content else {
        return String::new();
    };

    items
        .iter()
        .filter_map(|item| {
            item.get("text")
                .or_else(|| item.get("url"))
                .or_else(|| item.get("path"))
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn reasoning_text(item: &Value) -> String {
    let mut parts = Vec::new();
    for key in ["summary", "content"] {
        if let Some(value) = item.get(key) {
            match value {
                Value::String(text) if !text.is_empty() => parts.push(text.clone()),
                Value::Array(values) => {
                    for value in values {
                        if let Some(text) = value.as_str() {
                            if !text.is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    parts.join("\n")
}

fn command_text(item: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(command) = item.get("command").and_then(Value::as_str) {
        lines.push(format!("$ {command}"));
    }
    if let Some(cwd) = item.get("cwd").and_then(Value::as_str) {
        lines.push(format!("cwd: {cwd}"));
    }
    if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str) {
        if !output.is_empty() {
            lines.push(output.to_string());
        }
    }
    if let Some(exit_code) = item.get("exitCode").and_then(Value::as_i64) {
        lines.push(format!("exit code: {exit_code}"));
    }
    lines.join("\n")
}

fn file_change_text(item: &Value) -> String {
    let Some(Value::Array(changes)) = item.get("changes") else {
        return pretty_json(item);
    };

    changes
        .iter()
        .map(|change| {
            let path = change
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let kind = change
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("change");
            let diff = change.get("diff").and_then(Value::as_str).unwrap_or("");
            if diff.is_empty() {
                format!("{kind}: {path}")
            } else {
                format!("{kind}: {path}\n{diff}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn tool_call_text(item: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(arguments) = item.get("arguments") {
        lines.push(format!("arguments: {}", compact_json(arguments)));
    }
    if let Some(result) = item.get("result") {
        lines.push(format!("result: {}", pretty_json(result)));
    }
    if let Some(error) = item.get("error") {
        lines.push(format!("error: {}", pretty_json(error)));
    }
    lines.join("\n")
}

fn extract_delta_text(params: &Value) -> String {
    for key in ["delta", "text", "output", "chunk"] {
        if let Some(text) = params.get(key).and_then(Value::as_str) {
            return text.to_string();
        }
    }
    String::new()
}

pub fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_responses_notifications_and_server_requests() {
        let response = parse_jsonrpc_line(r#"{"id":10,"result":{"ok":true}}"#).unwrap();
        assert!(matches!(
            response,
            CodexInbound::Response {
                id: Value::Number(_),
                result: Some(_),
                error: None
            }
        ));

        let notification =
            parse_jsonrpc_line(r#"{"method":"turn/started","params":{"turn":{"id":"t1"}}}"#)
                .unwrap();
        assert_eq!(
            notification,
            CodexInbound::Notification {
                method: "turn/started".to_string(),
                params: json!({"turn":{"id":"t1"}}),
            }
        );

        let request = parse_jsonrpc_line(
            r#"{"id":"req-1","method":"mcpServer/elicitation/request","params":{"message":"Approve?"}}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            CodexInbound::ServerRequest {
                id: json!("req-1"),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"message":"Approve?"}),
            }
        );
    }

    #[test]
    fn appends_agent_deltas_to_existing_item() {
        let mut state = CodexTranscriptState::default();
        state.apply_notification(
            "item/started",
            &json!({"item":{"type":"agentMessage","id":"msg1","text":"","status":"inProgress"}}),
        );
        state.apply_notification(
            "item/agentMessage/delta",
            &json!({"itemId":"msg1","delta":"hello"}),
        );
        state.apply_notification(
            "item/agentMessage/delta",
            &json!({"itemId":"msg1","delta":" world"}),
        );

        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].text, "hello world");
    }

    #[test]
    fn completed_item_replaces_streamed_state() {
        let mut state = CodexTranscriptState::default();
        state.apply_notification(
            "item/agentMessage/delta",
            &json!({"itemId":"msg1","delta":"draft"}),
        );
        state.apply_notification(
            "item/completed",
            &json!({"item":{"type":"agentMessage","id":"msg1","text":"final","status":"completed"}}),
        );

        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].text, "final");
        assert_eq!(state.items[0].status, "completed");
    }

    #[test]
    fn unknown_items_fall_back_to_json() {
        let mut state = CodexTranscriptState::default();
        state.apply_notification(
            "item/started",
            &json!({"item":{"type":"newThing","id":"n1","custom":true}}),
        );

        assert_eq!(state.items[0].title, "newThing");
        assert!(state.items[0].text.contains("\"custom\": true"));
    }
}
