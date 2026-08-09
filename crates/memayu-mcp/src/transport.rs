//! MCP stdio transport — reads JSON-RPC requests from stdin, dispatches to tools,
//! and writes JSON-RPC responses to stdout.

use crate::tools;
use crate::types::*;
use crate::MemoryBackend;
use std::io::{BufRead, Write};
use std::sync::Arc;

pub async fn run(backend: Arc<dyn MemoryBackend>) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = stdin.lock();
    let mut writer = stdout.lock();

    let mut initialized = false;

    for line in reader.lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(_) => break,
        };

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::err(None, -32700, format!("Parse error: {e}"));
                let _ = writeln!(writer, "{}", serde_json::to_string(&resp).unwrap());
                let _ = writer.flush();
                continue;
            }
        };

        if !initialized && req.method != "initialize" {
            let resp = JsonRpcResponse::err(req.id, -32002, "Server not initialized".into());
            let _ = writeln!(writer, "{}", serde_json::to_string(&resp).unwrap());
            let _ = writer.flush();
            continue;
        }

        let response = handle_request(&req, backend.as_ref(), &mut initialized).await;
        // Notifications (id=None and no error) produce no wire response per JSON-RPC 2.0.
        if response.id.is_none() && response.error.is_none() && response.result.is_none() {
            continue;
        }
        let _ = writeln!(writer, "{}", serde_json::to_string(&response).unwrap());
        let _ = writer.flush();
    }
}

async fn handle_request(
    req: &JsonRpcRequest,
    backend: &dyn MemoryBackend,
    initialized: &mut bool,
) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize" => {
            *initialized = true;
            let result = InitializeResult {
                protocol_version: "2024-11-05",
                server_info: ServerInfo {
                    name: "memayu-mcp",
                    version: env!("CARGO_PKG_VERSION"),
                },
                capabilities: ServerCapabilities {
                    tools: ToolsCapability {
                        list_changed: false,
                    },
                },
            };
            JsonRpcResponse::ok(req.id.clone(), serde_json::to_value(result).unwrap())
        }
        "notifications/initialized" => JsonRpcResponse {
            jsonrpc: "2.0",
            id: None,
            result: None,
            error: None,
        },
        "tools/list" => {
            let list = ToolListResult {
                tools: tools::all_definitions(),
            };
            JsonRpcResponse::ok(req.id.clone(), serde_json::to_value(list).unwrap())
        }
        "tools/call" => match &req.params {
            Some(params) => {
                let call: ToolCallParams = match serde_json::from_value(params.clone()) {
                    Ok(p) => p,
                    Err(e) => {
                        return JsonRpcResponse::err(
                            req.id.clone(),
                            -32602,
                            format!("Invalid params: {e}"),
                        );
                    }
                };
                match tools::dispatch(&call.name, &call.arguments, backend).await {
                    Ok(value) => JsonRpcResponse::ok(req.id.clone(), value),
                    Err(e) => JsonRpcResponse::err(req.id.clone(), -32000, e.to_string()),
                }
            }
            None => JsonRpcResponse::err(req.id.clone(), -32602, "Missing params".into()),
        },
        _ => JsonRpcResponse::err(
            req.id.clone(),
            -32601,
            format!("Method not found: {}", req.method),
        ),
    }
}
