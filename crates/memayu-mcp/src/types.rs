//! JSON-RPC 2.0 and MCP protocol types used for the stdio transport.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── JSON-RPC 2.0 ──

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    pub fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Option<serde_json::Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

// ── MCP Initialize ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: &'static str,
    pub server_info: ServerInfo,
    pub capabilities: ServerCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub tools: ToolsCapability,
}

#[derive(Debug, Serialize)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

// ── MCP Tools ──

#[derive(Debug, Serialize)]
pub struct ToolListResult {
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Deserialize)]
pub struct ToolCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ToolCallResult {
    pub content: Vec<ToolContent>,
}

#[derive(Debug, Serialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: &'static str,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The MCP 2024-11-05 `InitializeResult` must serialize with camelCase keys
    /// (`protocolVersion`, `serverInfo`) so strict clients (e.g. Hermes Agent's
    /// pydantic-validated client) accept the handshake.
    #[test]
    fn initialize_result_uses_camel_case_keys() {
        let result = InitializeResult {
            protocol_version: "2024-11-05",
            server_info: ServerInfo {
                name: "memayu-mcp",
                version: "0.1.0",
            },
            capabilities: ServerCapabilities {
                tools: ToolsCapability {
                    list_changed: false,
                },
            },
        };

        let value = serde_json::to_value(&result).unwrap();
        let obj = value.as_object().unwrap();

        assert!(
            obj.contains_key("protocolVersion"),
            "missing protocolVersion: {value}"
        );
        assert!(
            obj.contains_key("serverInfo"),
            "missing serverInfo: {value}"
        );
        assert!(
            obj.contains_key("capabilities"),
            "missing capabilities: {value}"
        );
        // snake_case variants must NOT leak into the wire format.
        assert!(
            !obj.contains_key("protocol_version"),
            "snake_case protocol_version present"
        );
        assert!(
            !obj.contains_key("server_info"),
            "snake_case server_info present"
        );

        let server_info = obj["serverInfo"].as_object().unwrap();
        assert_eq!(server_info["name"], "memayu-mcp");
        assert_eq!(server_info["version"], "0.1.0");

        // `tools.listChanged` (nested) is also camelCase per spec.
        let capabilities = obj["capabilities"].as_object().unwrap();
        let tools = capabilities["tools"].as_object().unwrap();
        assert!(
            tools.contains_key("listChanged"),
            "missing listChanged: {value}"
        );
    }
}
