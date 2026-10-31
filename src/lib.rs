use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use worker::*;

#[derive(RustEmbed)]
#[folder = "data/"]
struct DataAssets;

// ---------- JSON-RPC Types ----------

#[derive(Deserialize, Debug)]
struct RpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

// ---------- Wasm Setup ----------

#[event(start)]
fn start() {
    // This safely prints Rust panics to the Cloudflare Worker logs 
    // instead of silently crashing the WebAssembly instance.
    console_error_panic_hook::set_once();
}

// ---------- HTTP Entry Point ----------

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/mcp", handle_mcp)
        .run(req, env)
        .await
}

// ---------- MCP Logic ----------

async fn handle_mcp(mut req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let session_id = req
        .headers()
        .get("mcp-session-id")?
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Parse JSON-RPC
    let rpc_req: RpcRequest = match req.json().await {
        Ok(r) => r,
        Err(e) => {
            console_error!("Failed to parse JSON-RPC: {}", e);
            return rpc_error(None, -32700, "Parse error");
        }
    };

    console_log!("Received MCP request: method={}", rpc_req.method);

    let result = match rpc_req.method.as_str() {
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "resources": {},
                "tools": {}
            },
            "serverInfo": {
                "name": "cmapaper-mcp",
                "version": "1.0"
            }
        }),

        "resources/list" => json!({
            "resources": [{
                "uri": "study-materials://guidelines/skill.md",
                "name": "Tutor Guidelines",
                "mimeType": "text/markdown",
                "description": "Custom instructions for tutoring and formatting answers"
            }]
        }),

        "resources/read" => {
            let uri = rpc_req
                .params
                .as_ref()
                .and_then(|p| p["uri"].as_str())
                .unwrap_or("");
                
            if uri == "study-materials://guidelines/skill.md" {
                let text = match DataAssets::get("skill.md") {
                    Some(file) => std::str::from_utf8(file.data.as_ref()).unwrap_or("Encoding error").to_string(),
                    None => "# Error\nskill.md not found in data folder.".to_string(),
                };

                json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "text/markdown",
                        "text": text
                    }]
                })
            } else {
                return rpc_error(rpc_req.id, -32602, "Resource not found");
            }
        }

        "tools/list" => json!({
            "tools": [{
                "name": "search_study_materials",
                "description": "Fetch PYQ and MQP study materials for a specific subject and year.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "subject": { "type": "string", "description": "e.g., physics, math" },
                        "year": { "type": "integer", "description": "e.g., 2026" },
                        "doc_type": { "type": "string", "enum": ["pyq", "mqp"], "description": "pyq or mqp" }
                    },
                    "required": ["subject", "year", "doc_type"]
                }
            }]
        }),

        "tools/call" => {
            let params = rpc_req.params.unwrap_or(json!({}));
            let name = params["name"].as_str().unwrap_or("");
            let args = params["arguments"].as_object();

            if name == "search_study_materials" && args.is_some() {
                let args = args.unwrap();
                let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("");
                let year = args.get("year").and_then(|v| v.as_i64()).unwrap_or(0);
                let doc_type = args.get("doc_type").and_then(|v| v.as_str()).unwrap_or("");

                let filename = format!("{}_{}_{}.md", subject, doc_type, year).to_lowercase();
                
                let document_text = match DataAssets::get(&filename) {
                    Some(file) => std::str::from_utf8(file.data.as_ref()).unwrap_or("Encoding error").to_string(),
                    None => format!("No document found for filename: {}", filename),
                };

                json!({
                    "content": [{
                        "type": "text",
                        "text": document_text
                    }]
                })
            } else {
                return rpc_error(rpc_req.id, -32601, "Tool not found or invalid arguments");
            }
        }

        _ => return rpc_error(rpc_req.id, -32601, "Method not found"),
    };

    let mut response = Response::from_json(&RpcResponse {
        jsonrpc: "2.0",
        id: rpc_req.id,
        result: Some(result),
        error: None,
    })?;

    response.headers_mut().set("mcp-session-id", &session_id)?;
    Ok(response)
}

// ---------- Helper for Errors ----------

fn rpc_error(id: Option<Value>, code: i32, message: &str) -> Result<Response> {
    Response::from_json(&RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(json!({
            "code": code,
            "message": message
        })),
    })
}
