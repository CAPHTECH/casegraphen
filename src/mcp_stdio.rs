//! External stdio MCP adapter for the transport-neutral control plane.
//!
//! The adapter owns JSON-RPC framing and session state only. Every resource
//! projection and tool decision is delegated; it does not schedule, retry, or
//! call models.

use crate::control_plane::{
    read_resource, ControlPlaneNotification, ControlPlaneRequest, ControlPlaneState,
    ControlPlaneTool, DecisionDelegate, ResourceDelegate, CONTROL_PLANE_REQUEST_SCHEMA,
    RESOURCE_TEMPLATES, TOOLS,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

/// MCP protocol revision implemented by the reference stdio adapter.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolArguments {
    request_id: String,
    idempotency_key: String,
    #[serde(default)]
    base_revision_id: Option<String>,
    #[serde(default)]
    caller_declared_audit_context: Option<crate::control_plane::CallerDeclaredAuditContext>,
    payload: Value,
}

/// One-process MCP session around a caller-supplied decision/resource owner.
pub struct McpStdioServer<D> {
    state: ControlPlaneState,
    delegate: D,
    initialize_completed: bool,
    initialized: bool,
    state_path: Option<PathBuf>,
    authorization_token: Option<String>,
}

impl<D: DecisionDelegate + ResourceDelegate> McpStdioServer<D> {
    /// Creates a session. Protocol replay is intentionally process-local.
    pub fn new(delegate: D) -> Self {
        Self {
            state: ControlPlaneState::new(),
            delegate,
            initialize_completed: false,
            initialized: false,
            state_path: None,
            authorization_token: None,
        }
    }

    /// Creates an operational session with durable replay state and explicit
    /// bearer-style request authorization. The token is retained in memory
    /// only and is never serialized into protocol state or responses.
    pub fn new_durable_authenticated(
        delegate: D,
        state_path: impl AsRef<Path>,
        authorization_token: String,
    ) -> io::Result<Self> {
        if authorization_token.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "authorization token must not be empty",
            ));
        }
        let state_path = state_path.as_ref().to_path_buf();
        Ok(Self {
            state: ControlPlaneState::load_durable(&state_path)?,
            delegate,
            initialize_completed: false,
            initialized: false,
            state_path: Some(state_path),
            authorization_token: Some(authorization_token),
        })
    }

    /// Handles one compact JSON-RPC message and returns zero or one response.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => {
                return Some(rpc_error(
                    Value::Null,
                    -32700,
                    "Parse error",
                    json!({"detail": error.to_string()}),
                ))
            }
        };
        let id = message.get("id").cloned();
        if message.get("jsonrpc") != Some(&json!("2.0"))
            || message.get("method").and_then(Value::as_str).is_none()
            || matches!(
                id,
                Some(Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_))
            )
        {
            return Some(rpc_error(
                id.unwrap_or(Value::Null),
                -32600,
                "Invalid Request",
                Value::Null,
            ));
        }
        let method = message["method"].as_str().expect("validated method");
        let mut params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        if id.is_none() {
            if method == "notifications/initialized" && self.initialize_completed {
                self.initialized = true;
            }
            return None;
        }
        let id = id.expect("request id exists");
        if method == "initialize" {
            self.initialize_completed = true;
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(MCP_PROTOCOL_VERSION);
            let negotiated = if requested == MCP_PROTOCOL_VERSION {
                requested
            } else {
                MCP_PROTOCOL_VERSION
            };
            return Some(rpc_result(
                id,
                json!({
                    "protocolVersion": negotiated,
                    "capabilities": {"resources": {"listChanged": false}, "tools": {"listChanged": false}},
                    "serverInfo": {"name": "casegraphen-mcp", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "Bearer authentication authorizes host-tool access. Caller-declared audit context is attribution only and never a CaseGraphen operation gate. Acceptance-ledger mutations remain subject to canonical CaseGraphen gates and review."
                }),
            ));
        }
        if !self.initialized && method != "ping" {
            return Some(rpc_error(id, -32002, "Server not initialized", Value::Null));
        }
        if let Some(expected) = &self.authorization_token {
            let supplied = params.get("authorization").and_then(Value::as_str);
            if !supplied.is_some_and(|value| constant_time_equal(value, expected)) {
                return Some(rpc_error(
                    id,
                    -32001,
                    "Unauthorized",
                    json!({"detail": "an exact operational-host authorization token is required"}),
                ));
            }
            if let Some(object) = params.as_object_mut() {
                object.remove("authorization");
            }
        }

        let result = match method {
            "ping" => Ok(json!({})),
            // Concrete identifiers are state-dependent. The declared catalog
            // is exposed through the standard URI-template method below.
            "resources/list" => Ok(json!({"resources": []})),
            "resources/templates/list" => Ok(json!({
                "resourceTemplates": RESOURCE_TEMPLATES.iter().map(|uri| json!({
                    "uriTemplate": uri,
                    "name": uri,
                    "mimeType": "application/json"
                })).collect::<Vec<_>>()
            })),
            "resources/read" => self.read_resource_request(&params),
            "tools/list" => Ok(json!({
                "tools": TOOLS.iter().map(|tool| tool_definition(tool, &self.delegate)).collect::<Vec<_>>()
            })),
            "tools/call" => self.call_tool(&params),
            "casegraphen/replay" => replay_result(&self.state, &params, false),
            "casegraphen/notifications/replay" => replay_result(&self.state, &params, true),
            "casegraphen/notifications/publish" => self.publish_notification(&params),
            _ => {
                return Some(rpc_error(
                    id,
                    -32601,
                    "Method not found",
                    json!({"method": method}),
                ))
            }
        };
        Some(match result {
            Ok(value) => rpc_result(id, value),
            Err(detail) => rpc_error(id, -32602, "Invalid params", json!({"detail": detail})),
        })
    }

    fn read_resource_request(&mut self, params: &Value) -> Result<Value, String> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| "resources/read requires string uri".to_owned())?;
        match read_resource(uri, &mut self.delegate) {
            Ok(value) => Ok(json!({"contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": serde_json::to_string(&value).expect("JSON value serializes")
            }]})),
            Err(refusal) => Ok(json!({"contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": serde_json::to_string(&json!({"refusal": refusal})).expect("refusal serializes")
            }]})),
        }
    }

    fn call_tool(&mut self, params: &Value) -> Result<Value, String> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "tools/call requires string name".to_owned())?;
        let tool = serde_json::from_value::<ControlPlaneTool>(json!(name))
            .map_err(|_| format!("unknown control-plane tool: {name}"))?;
        if !TOOLS.contains(&tool) {
            return Err(format!("tool is not in the control-plane catalog: {name}"));
        }
        let arguments = serde_json::from_value::<ToolArguments>(
            params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .map_err(|error| error.to_string())?;
        let request = ControlPlaneRequest {
            schema: CONTROL_PLANE_REQUEST_SCHEMA.to_owned(),
            request_id: arguments.request_id,
            idempotency_key: arguments.idempotency_key,
            tool,
            base_revision_id: arguments.base_revision_id,
            caller_declared_audit_context: arguments.caller_declared_audit_context,
            payload: arguments.payload,
        };
        let response = if let Some(path) = &self.state_path {
            self.state
                .execute_durable(&request, &mut self.delegate, path)
        } else {
            self.state.execute(&request, &mut self.delegate)
        };
        let is_error = response.refusal.is_some();
        let structured = serde_json::to_value(&response).expect("response serializes");
        Ok(json!({
            "content": [{"type": "text", "text": serde_json::to_string(&structured).expect("response JSON serializes")}],
            "structuredContent": structured,
            "transport_authentication": {
                "mechanism": if self.authorization_token.is_some() { "bearer_token" } else { "none" },
                "authenticated": self.authorization_token.is_some(),
                "authorizes_host_tool_access": self.authorization_token.is_some(),
                "canonical_casegraphen_authorization": "not_evaluated"
            },
            "isError": is_error
        }))
    }

    fn publish_notification(&mut self, params: &Value) -> Result<Value, String> {
        let notification = serde_json::from_value::<ControlPlaneNotification>(params.clone())
            .map_err(|error| error.to_string())?;
        let published = if let Some(path) = &self.state_path {
            self.state.publish_notification_durable(notification, path)
        } else {
            self.state.publish_notification(notification)
        };
        published
            .map(|notification| json!({"notification": notification}))
            .map_err(|refusal| serde_json::to_string(&refusal).expect("refusal serializes"))
    }
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut difference = left.len() ^ right.len();
    let maximum = left.len().max(right.len());
    for index in 0..maximum {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

/// Runs the newline-delimited stdio transport until EOF.
pub fn serve_stdio<D: DecisionDelegate + ResourceDelegate>(
    server: &mut McpStdioServer<D>,
    input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    for line in input.lines() {
        if let Some(response) = server.handle_line(&line?) {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn replay_result(
    state: &ControlPlaneState,
    params: &Value,
    notifications: bool,
) -> Result<Value, String> {
    let after = params
        .get("after_sequence")
        .and_then(Value::as_u64)
        .ok_or_else(|| "replay requires unsigned after_sequence".to_owned())?;
    if notifications {
        Ok(json!({"notifications": state.notifications_after(after)}))
    } else {
        Ok(json!({"responses": state.replay_after(after)}))
    }
}

fn tool_definition(tool: &ControlPlaneTool, delegate: &impl DecisionDelegate) -> Value {
    let name = serde_json::to_value(tool).expect("tool serializes");
    let mut required = vec!["request_id", "idempotency_key", "payload"];
    if tool.requires_base_revision() {
        required.push("base_revision_id");
    }
    if tool.changes_managed_state() {
        required.push("caller_declared_audit_context");
    }
    json!({
        "name": name,
        "description": tool.description(),
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "request_id": {"type": "string", "minLength": 1},
                "idempotency_key": {"type": "string", "minLength": 1},
                "base_revision_id": {"type": "string", "minLength": 1},
                "caller_declared_audit_context": {"type": "object"},
                "payload": delegate.payload_schema(*tool)
            },
            "required": required
        }
    })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str, data: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message, "data": data}})
}
