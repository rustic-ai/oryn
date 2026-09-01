//! First-party MCP stdio adapter over Oryn's shared typed agent contract.
//!
//! The adapter does not accept OIL or raw JavaScript. `oryn_perform` validates
//! the same versioned `IntentRequest` used by Rust and Python, then delegates to
//! `PageHandle::perform` through the context session.

use oryn_common::agent::{AGENT_INTENT_CONTRACT_VERSION, IntentRequest};
use oryn_native::{Browser, oil::NativeContextOilSession};
use serde_json::{Value, json};

pub const MCP_SERVER_NAME: &str = "oryn";
pub const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MCP_DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
pub const MCP_SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct McpServer {
    session: Option<NativeContextOilSession>,
    secret_fd: Option<i32>,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    pub const fn new() -> Self {
        Self {
            session: None,
            secret_fd: None,
        }
    }

    pub const fn with_secret_fd(secret_fd: Option<i32>) -> Self {
        Self {
            session: None,
            secret_fd,
        }
    }

    /// Handle one JSON-RPC message. Notifications deliberately return no
    /// response, as required by JSON-RPC and MCP stdio framing.
    pub async fn handle(&mut self, request: Value) -> Option<Value> {
        let id = request.get("id").cloned()?;
        let method = request.get("method").and_then(Value::as_str);
        let response = match method {
            Some("initialize") => Ok(initialize_result(
                request
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str),
            )),
            Some("ping") => Ok(json!({})),
            Some("tools/list") => Ok(tools_list_result()),
            Some("tools/call") => self.call_tool(request.get("params")).await,
            Some(method) => Err(RpcError::new(-32601, format!("method not found: {method}"))),
            None => Err(RpcError::new(-32600, "request has no method")),
        };
        Some(match response {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": error.code, "message": error.message}
            }),
        })
    }

    async fn call_tool(&mut self, params: Option<&Value>) -> Result<Value, RpcError> {
        let params = params
            .and_then(Value::as_object)
            .ok_or_else(|| RpcError::new(-32602, "tools/call params must be an object"))?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(-32602, "tools/call requires a tool name"))?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        match name {
            "oryn_perform" => {
                let request: IntentRequest = serde_json::from_value(arguments)
                    .map_err(|error| RpcError::new(-32602, error.to_string()))?;
                let intent = request
                    .validate()
                    .map_err(|error| RpcError::new(-32602, error))?;
                let session = self.ensure_session().await?;
                match session.perform(intent).await {
                    Ok(result) => {
                        let structured = serde_json::to_value(result)
                            .map_err(|error| RpcError::new(-32603, error.to_string()))?;
                        let text = serde_json::to_string(&structured)
                            .map_err(|error| RpcError::new(-32603, error.to_string()))?;
                        Ok(json!({
                            "content": [{"type": "text", "text": text}],
                            "structuredContent": structured,
                            "isError": false
                        }))
                    }
                    Err(error) => Ok(tool_error(error.to_string())),
                }
            }
            _ => Err(RpcError::new(-32602, format!("unknown tool: {name}"))),
        }
    }

    async fn ensure_session(&mut self) -> Result<&mut NativeContextOilSession, RpcError> {
        if self.session.is_none() {
            let context = Browser::new().new_context();
            if let Some(descriptor) = self.secret_fd.take() {
                crate::secret_input::provision_context_from_fd(&context, descriptor)
                    .map_err(|error| RpcError::new(-32000, error.to_string()))?;
            }
            let page = context.new_page();
            page.load_html("about:blank", "<!doctype html>")
                .await
                .map_err(|error| RpcError::new(-32000, error.to_string()))?;
            self.session = Some(NativeContextOilSession::new(context));
        }
        self.session
            .as_mut()
            .ok_or_else(|| RpcError::new(-32603, "failed to create native session"))
    }
}

fn initialize_result(requested_protocol: Option<&str>) -> Value {
    let protocol = requested_protocol
        .filter(|requested| MCP_SUPPORTED_PROTOCOL_VERSIONS.contains(requested))
        .unwrap_or(MCP_DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": protocol,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": MCP_SERVER_NAME, "version": MCP_SERVER_VERSION},
        "instructions": "Use oryn_perform with the versioned Oryn Intent/Target contract. Secret inputs contain parent-owned labels only; never send secret values."
    })
}

pub fn tools_list_result() -> Value {
    json!({
        "tools": [{
            "name": "oryn_perform",
            "title": "Perform an Oryn browser intent",
            "description": "Executes Oryn's shared typed agent intent against the active native page. This is not an OIL or raw-JavaScript tunnel.",
            "inputSchema": intent_request_schema(),
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": true
            }
        }]
    })
}

pub fn intent_request_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["contract_version", "intent"],
        "properties": {
            "contract_version": {
                "type": "integer",
                "const": AGENT_INTENT_CONTRACT_VERSION
            },
            "intent": {"$ref": "#/$defs/intent"}
        },
        "$defs": {
            "semantic_ref": {
                "type": "object",
                "additionalProperties": false,
                "required": ["page_id", "document_generation", "node"],
                "properties": {
                    "page_id": {"type": "integer", "minimum": 0},
                    "document_generation": {"type": "integer", "minimum": 0},
                    "node": {"type": "integer", "minimum": 0}
                }
            },
            "target": {
                "oneOf": [
                    {"type":"object","required":["kind","reference"],"properties":{"kind":{"const":"reference"},"reference":{"$ref":"#/$defs/semantic_ref"}},"additionalProperties":false},
                    {"type":"object","required":["kind","alias"],"properties":{"kind":{"const":"alias"},"alias":{"type":"integer","minimum":1}},"additionalProperties":false},
                    {"type":"object","required":["kind","text"],"properties":{"kind":{"const":"text"},"text":{"type":"string"}},"additionalProperties":false},
                    {"type":"object","required":["kind","role"],"properties":{"kind":{"const":"role"},"role":{"type":"string"}},"additionalProperties":false},
                    {"type":"object","required":["kind","selector"],"properties":{"kind":{"const":"css"},"selector":{"type":"string"}},"additionalProperties":false},
                    {"type":"object","required":["kind","expression"],"properties":{"kind":{"const":"xpath"},"expression":{"type":"string"}},"additionalProperties":false},
                    {"type":"object","required":["kind","target","anchor"],"properties":{"kind":{"enum":["near","after","before"]},"target":{"$ref":"#/$defs/target"},"anchor":{"$ref":"#/$defs/target"}},"additionalProperties":false},
                    {"type":"object","required":["kind","target","container"],"properties":{"kind":{"const":"inside"},"target":{"$ref":"#/$defs/target"},"container":{"$ref":"#/$defs/target"}},"additionalProperties":false},
                    {"type":"object","required":["kind","target","content"],"properties":{"kind":{"const":"contains"},"target":{"$ref":"#/$defs/target"},"content":{"$ref":"#/$defs/target"}},"additionalProperties":false}
                ]
            },
            "input": {
                "oneOf": [
                    {"type":"object","required":["kind","value"],"properties":{"kind":{"const":"text"},"value":{"type":"string"}},"additionalProperties":false},
                    {"type":"object","required":["kind","label"],"properties":{"kind":{"const":"secret"},"label":{"type":"string","minLength":1}},"additionalProperties":false}
                ]
            },
            "observe_options": {
                "type":"object",
                "additionalProperties":false,
                "properties":{
                    "profile":{"enum":["compact","full","scoped","delta"]},
                    "since":{"type":["integer","null"],"minimum":0},
                    "include_geometry":{"type":"boolean"},
                    "viewport_only":{"type":"boolean"},
                    "include_hidden":{"type":"boolean"},
                    "include_structures":{"type":"boolean"},
                    "semantic_contract_version":{"type":["integer","null"],"enum":[6,null]},
                    "scope":{
                        "type":["object","null"],
                        "additionalProperties":false,
                        "required":["root"],
                        "properties":{
                            "root":{"type":"object"},
                            "ancestor_depth":{"type":"integer","minimum":0,"maximum":64},
                            "descendant_depth":{"type":"integer","minimum":0,"maximum":64},
                            "relationship_depth":{"type":"integer","minimum":0,"maximum":64},
                            "include_structural_neighbors":{"type":"boolean"},
                            "include_spatial_neighbors":{"type":"boolean"},
                            "max_nodes":{"type":"integer","minimum":1,"maximum":10000}
                        }
                    }
                }
            },
            "intent": {
                "oneOf": [
                    {"type":"object","required":["intent","url"],"properties":{"intent":{"const":"navigate"},"url":{"type":"string"}},"additionalProperties":false},
                    {"type":"object","required":["intent"],"properties":{"intent":{"enum":["back","forward","refresh"]}},"additionalProperties":false},
                    {"type":"object","required":["intent"],"properties":{"intent":{"const":"observe"},"options":{"$ref":"#/$defs/observe_options"}},"additionalProperties":false},
                    {"type":"object","required":["intent","target"],"properties":{"intent":{"const":"click"},"target":{"$ref":"#/$defs/target"},"force":{"type":"boolean"}},"additionalProperties":false},
                    {"type":"object","required":["intent","target","input"],"properties":{"intent":{"const":"type"},"target":{"$ref":"#/$defs/target"},"input":{"$ref":"#/$defs/input"}},"additionalProperties":false},
                    {"type":"object","required":["intent","target"],"properties":{"intent":{"enum":["clear","submit","focus","hover"]},"target":{"$ref":"#/$defs/target"}},"additionalProperties":false},
                    {"type":"object","required":["intent"],"properties":{"intent":{"const":"clipboard_read"}},"additionalProperties":false},
                    {"type":"object","required":["intent","value"],"properties":{"intent":{"const":"clipboard_write"},"value":{"type":"string"}},"additionalProperties":false},
                    {"type":"object","required":["intent","target","files"],"properties":{"intent":{"const":"upload"},"target":{"$ref":"#/$defs/target"},"files":{"type":"array","minItems":1,"items":{"type":"string","minLength":1}}},"additionalProperties":false},
                    {"type":"object","required":["intent","target"],"properties":{"intent":{"const":"check"},"target":{"$ref":"#/$defs/target"},"checked":{"type":"boolean"}},"additionalProperties":false},
                    {"type":"object","required":["intent","target","value"],"properties":{"intent":{"const":"select"},"target":{"$ref":"#/$defs/target"},"value":{"type":"string"}},"additionalProperties":false},
                    {"type":"object","required":["intent","kind"],"properties":{"intent":{"const":"extract"},"kind":{"enum":["text","html","links","images","tables","url","title"]},"selector":{"type":["string","null"]}},"additionalProperties":false},
                    {"type":"object","required":["intent","after","timeout_ms"],"properties":{"intent":{"const":"wait_revision"},"after":{"type":"integer","minimum":0},"timeout_ms":{"type":"integer","minimum":0,"maximum":30000}},"additionalProperties":false}
                ]
            }
        }
    })
}

fn tool_error(message: String) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initialize_negotiates_the_requested_protocol() {
        let mut server = McpServer::new();
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": "2025-03-26"}
            }))
            .await
            .unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(response["result"]["serverInfo"]["name"], MCP_SERVER_NAME);
    }

    #[tokio::test]
    async fn initialize_does_not_echo_an_unknown_protocol() {
        let mut server = McpServer::new();
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "initialize",
                "params": {"protocolVersion": "2099-01-01"}
            }))
            .await
            .unwrap();
        assert_eq!(
            response["result"]["protocolVersion"],
            MCP_DEFAULT_PROTOCOL_VERSION
        );
    }

    #[tokio::test]
    async fn tools_list_exposes_only_the_typed_contract() {
        let mut server = McpServer::new();
        let response = server
            .handle(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
            .await
            .unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "oryn_perform");
        assert_eq!(
            tools[0]["inputSchema"]["properties"]["contract_version"]["const"],
            AGENT_INTENT_CONTRACT_VERSION
        );
        let input_variants = tools[0]["inputSchema"]["$defs"]["input"]["oneOf"]
            .as_array()
            .unwrap();
        let secret = input_variants
            .iter()
            .find(|variant| variant["properties"]["kind"]["const"] == "secret")
            .unwrap();
        assert!(secret["properties"].get("label").is_some());
        assert!(secret["properties"].get("value").is_none());
    }

    #[tokio::test]
    async fn malformed_typed_intent_fails_before_runtime_creation() {
        let mut server = McpServer::new();
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "oryn_perform",
                    "arguments": {
                        "contract_version": AGENT_INTENT_CONTRACT_VERSION + 1,
                        "intent": {"intent": "back"}
                    }
                }
            }))
            .await
            .unwrap();
        assert_eq!(response["error"]["code"], -32602);
        assert!(server.session.is_none());
    }

    #[tokio::test]
    async fn secret_value_field_is_rejected_before_runtime_creation() {
        let mut server = McpServer::new();
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "tools/call",
                "params": {
                    "name": "oryn_perform",
                    "arguments": {
                        "contract_version": AGENT_INTENT_CONTRACT_VERSION,
                        "intent": {
                            "intent": "type",
                            "target": {"kind": "role", "role": "textbox"},
                            "input": {
                                "kind": "secret",
                                "label": "password",
                                "value": "must-not-enter"
                            }
                        }
                    }
                }
            }))
            .await
            .unwrap();
        assert_eq!(response["error"]["code"], -32602);
        assert!(server.session.is_none());
    }

    #[tokio::test]
    async fn notifications_do_not_receive_json_rpc_responses() {
        let mut server = McpServer::new();
        assert!(
            server
                .handle(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
                .await
                .is_none()
        );
    }
}
