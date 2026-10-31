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
    #[allow(dead_code)]
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

// ---------- Asset Helpers ----------

fn strip_frontmatter(text: &str) -> &str {
    // shortcut: splits on first "\n---" delimiter, upgrade to a YAML parser if frontmatter contains "---" in strings
    let Some(rest) = text.strip_prefix("---") else {
        return text;
    };
    let Some((_, body)) = rest.split_once("\n---") else {
        return text;
    };
    body.trim_start()
}

fn load_skill_text() -> String {
    match DataAssets::get("skill.md") {
        Some(file) => match std::str::from_utf8(file.data.as_ref()) {
            Ok(raw) => strip_frontmatter(raw).to_owned(),
            Err(_) => "Encoding error in skill.md".to_owned(),
        },
        None => "# Error\nskill.md not found in data folder.".to_owned(),
    }
}

// ---------- MCP Logic ----------

async fn handle_mcp(mut req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let session_id = req
        .headers()
        .get("mcp-session-id")?
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let rpc_req: RpcRequest = match req.json().await {
        Ok(r) => r,
        Err(e) => {
            console_error!("Failed to parse JSON-RPC: {}", e);
            return rpc_error(None, -32700, "Parse error");
        }
    };

    console_log!("Received MCP request: method={}", rpc_req.method);

    // JSON-RPC notifications (e.g., notifications/initialized) have no `id` and expect 202 Accepted
    if rpc_req.id.is_none() || rpc_req.method.starts_with("notifications/") {
        let mut resp = Response::empty()?.with_status(202);
        resp.headers_mut().set("mcp-session-id", &session_id)?;
        return Ok(resp);
    }

    let result = match rpc_req.method.as_str() {
        "ping" => json!({}),

        // 1. Server `instructions`: Auto-injected into system context by supporting MCP clients
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "resources": {},
                "prompts": {},
                "tools": {}
            },
            "serverInfo": {
                "name": "cmapaper-mcp",
                "version": "1.0.0"
            },
            "instructions": load_skill_text()
        }),

        // 2. Prompts: 1-click / slash-command starter to load the skill for an entire chat
        "prompts/list" => json!({
            "prompts": [{
                "name": "indian-commerce-tutor",
                "title": "Activate Indian Commerce Tutor Skill",
                "description": "Loads the 8-step Indian Commerce & Exam Tutor methodology for the entire chat.",
                "arguments": [{
                    "name": "topic",
                    "description": "Optional concept, section, or exam problem to start with",
                    "required": false
                }]
            }]
        }),

        "prompts/get" => {
            let params = rpc_req.params.as_ref();
            let name = params.and_then(|p| p["name"].as_str()).unwrap_or("");

            if name != "indian-commerce-tutor" {
                return rpc_error(rpc_req.id, -32602, "Prompt not found");
            }

            let topic = params
                .and_then(|p| p["arguments"]["topic"].as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Confirm that the Indian Commerce Concept Tutor blueprint is active and ask what topic or problem I want to study.");

            let text = format!(
                "Follow these tutoring guidelines for the entire conversation:\n\n{}\n\n---\nUser request: {}",
                load_skill_text(),
                topic
            );

            json!({
                "description": "Indian Commerce Concept Tutor Skill",
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": text
                    }
                }]
            })
        }

        // 3. Resources: Manual attachment fallback
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
                json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "text/markdown",
                        "text": load_skill_text()
                    }]
                })
            } else {
                return rpc_error(rpc_req.id, -32602, "Resource not found");
            }
        }

        // 4. Tools: Model-driven skill loader + study material lookup
        "tools/list" => json!({
            "tools": [
                {
                    "name": "get_tutor_guidelines",
                    "description": "Returns the mandatory 8-step tutoring blueprint and formatting rules. Call this tool ONCE at the start of a chat before explaining concepts or solving PYQ/MQP problems.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "search_study_materials",
                    "description": "Fetch PYQ and MQP study materials for a specific subject and year. IMPORTANT: If you have not called `get_tutor_guidelines` in this chat yet, call it alongside or before this tool to format your explanation properly.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "subject": { "type": "string", "description": "e.g., physics, maths" },
                            "year": { "type": "integer", "description": "e.g., 2025" },
                            "doc_type": { "type": "string", "enum": ["pyq", "mqp"], "description": "pyq or mqp" }
                        },
                        "required": ["subject", "year", "doc_type"]
                    }
                }
            ]
        }),

        "tools/call" => {
            let params = rpc_req.params.as_ref();
            let name = params.and_then(|p| p["name"].as_str()).unwrap_or("");
            let args = params.and_then(|p| p["arguments"].as_object());

            match (name, args) {
                ("get_tutor_guidelines", _) => json!({
                    "content": [{
                        "type": "text",
                        "text": load_skill_text()
                    }]
                }),

                ("search_study_materials", Some(args)) => {
                    let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("");
                    let year = args.get("year").and_then(|v| v.as_i64()).unwrap_or(0);
                    let doc_type = args.get("doc_type").and_then(|v| v.as_str()).unwrap_or("");

                    let filename = format!("{}_{}_{}.md", subject, doc_type, year).to_lowercase();

                    let document_text = match DataAssets::get(&filename) {
                        Some(file) => std::str::from_utf8(file.data.as_ref())
                            .unwrap_or("Encoding error")
                            .to_owned(),
                        None => {
                            let available: Vec<_> = DataAssets::iter()
                                .filter(|f| f.as_ref() != "skill.md")
                                .collect();
                            format!(
                                "No document found for '{filename}'. Available files: {available:?}"
                            )
                        }
                    };

                    json!({
                        "content": [{
                            "type": "text",
                            "text": document_text
                        }]
                    })
                }

                _ => return rpc_error(rpc_req.id, -32601, "Tool not found or invalid arguments"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_yaml_frontmatter_cleanly() {
        let raw = "---\nname: test\ndescription: demo\n---\n\n# Heading\nBody";
        assert_eq!(strip_frontmatter(raw), "# Heading\nBody");
        assert_eq!(strip_frontmatter("# No Frontmatter"), "# No Frontmatter");
    }
}
