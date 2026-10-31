use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use worker::*;

#[derive(RustEmbed)]
#[folder = "skills/"]
struct SkillAssets;

#[derive(RustEmbed)]
#[folder = "references/"]
struct ReferenceAssets;

const GENERAL_SKILL_FILE: &str = "general.md";

// ---------- JSON-RPC Types ----------

#[derive(Deserialize, Debug)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize, Debug, PartialEq)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

#[derive(Debug, PartialEq)]
enum RpcOutcome {
    Notification,
    Reply(RpcResponse),
}

// ---------- Zero-Allocation Skill Parser ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SkillDoc<'a> {
    name: &'a str,
    description: &'a str,
    body: &'a str,
}

fn parse_skill(raw: &str) -> SkillDoc<'_> {
    // shortcut: line-based frontmatter parser for `key: value`
    let trimmed = raw.trim_start_matches('\u{feff}');
    let Some(after_open) = trimmed.strip_prefix("---") else {
        return SkillDoc {
            name: "",
            description: "",
            body: trimmed.trim(),
        };
    };
    let Some((fm, body)) = after_open.split_once("\n---") else {
        return SkillDoc {
            name: "",
            description: "",
            body: trimmed.trim(),
        };
    };

    let mut name = "";
    let mut description = "";
    for line in fm.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            match k.trim() {
                "name" => name = v,
                "description" => description = v,
                _ => {}
            }
        }
    }

    SkillDoc {
        name,
        description,
        body: body.trim(),
    }
}

// ---------- Asset Catalog Helpers ----------

fn load_general_instructions() -> String {
    let general_body = SkillAssets::get(GENERAL_SKILL_FILE)
        .and_then(|f| std::str::from_utf8(f.data.as_ref()).ok().map(|s| parse_skill(s).body.to_owned()))
        .unwrap_or_else(|| "# Error: skills/general.md missing".to_owned());

    let chapter_catalog = format_chapter_catalog();

    format!(
        "{general_body}\n\n---\n## Chapter Skill Routing\nThe general CMA blueprint above is active for this entire chat. When a question touches a specific syllabus chapter below, call `load_chapter_skill` ONCE for that chapter before answering (do not re-call if already loaded in this chat):\n{chapter_catalog}"
    )
}

fn list_chapter_files() -> Vec<String> {
    let mut files: Vec<String> = SkillAssets::iter()
        .filter(|f| f.as_ref() != GENERAL_SKILL_FILE && f.ends_with(".md"))
        .map(|f| f.into_owned())
        .collect();
    files.sort();
    files
}

fn format_chapter_catalog() -> String {
    list_chapter_files()
        .into_iter()
        .map(|filename| {
            if let Some(file) = SkillAssets::get(&filename) {
                if let Ok(raw) = std::str::from_utf8(file.data.as_ref()) {
                    let doc = parse_skill(raw);
                    return format!("- `{filename}` ({}) — {}", doc.name, doc.description);
                }
            }
            format!("- `{filename}`")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn list_reference_files() -> Vec<String> {
    let mut files: Vec<String> = ReferenceAssets::iter()
        .filter(|f| !f.starts_with('.'))
        .map(|f| f.into_owned())
        .collect();
    files.sort();
    files
}

// ---------- Pure MCP State Machine (Host-Testable) ----------

fn process_rpc(rpc_req: RpcRequest) -> RpcOutcome {
    if rpc_req.id.is_none() || rpc_req.method.starts_with("notifications/") {
        return RpcOutcome::Notification;
    }

    let chapter_files = list_chapter_files();
    let reference_files = list_reference_files();
    let chapter_catalog = format_chapter_catalog();

    let result = match rpc_req.method.as_str() {
        "ping" => json!({}),

        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "prompts": {},
                "resources": {},
                "tools": {}
            },
            "serverInfo": {
                "name": "cmapaper-mcp",
                "version": "2.0.1"
            },
            "instructions": load_general_instructions()
        }),

        "prompts/list" => json!({
            "prompts": [{
                "name": "start-cma-session",
                "title": "Start CMA Study Session (General Skill)",
                "description": "Loads the general CMA blueprint once at the start of a chat."
            }]
        }),

        "prompts/get" => {
            let name = rpc_req
                .params
                .as_ref()
                .and_then(|p| p["name"].as_str())
                .unwrap_or("");

            if name != "start-cma-session" {
                return rpc_err(rpc_req.id, -32602, "Prompt not found");
            }

            json!({
                "description": "General CMA Tutor Blueprint",
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": load_general_instructions()
                    }
                }]
            })
        }

        "resources/list" => {
            let mut resources = vec![json!({
                "uri": "cma://skills/general.md",
                "name": "General CMA Tutor Skill",
                "mimeType": "text/markdown",
                "description": "Core 8-step CMA tutoring blueprint"
            })];

            for file in &chapter_files {
                resources.push(json!({
                    "uri": format!("cma://skills/{file}"),
                    "name": file,
                    "mimeType": "text/markdown",
                    "description": "Chapter-specific CMA skill"
                }));
            }
            json!({ "resources": resources })
        }

        "resources/read" => {
            let uri = rpc_req
                .params
                .as_ref()
                .and_then(|p| p["uri"].as_str())
                .unwrap_or("");

            let Some(skill_file) = uri.strip_prefix("cma://skills/") else {
                return rpc_err(rpc_req.id, -32602, "Resource URI must start with cma://skills/");
            };

            let Some(asset) = SkillAssets::get(skill_file) else {
                return rpc_err(rpc_req.id, -32602, "Skill resource not found");
            };

            let text = std::str::from_utf8(asset.data.as_ref())
                .map(|raw| parse_skill(raw).body)
                .unwrap_or("Encoding error");

            json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "text/markdown",
                    "text": text
                }]
            })
        }

        "tools/list" => json!({
            "tools": [
                {
                    "name": "load_chapter_skill",
                    "description": format!(
                        "Load chapter-specific ICMAI CMA statutory rules, formulas, and examiner trap zones on demand. Call this ONCE when the conversation enters a specific chapter (do NOT re-call if already loaded in this chat). Available chapter skills:\n{chapter_catalog}"
                    ),
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "skill_file": {
                                "type": "string",
                                "enum": chapter_files,
                                "description": "Exact chapter skill filename from skills/"
                            }
                        },
                        "required": ["skill_file"]
                    }
                },
                {
                    "name": "get_reference_material",
                    "description": format!(
                        "Fetch PYQ, MQP, or statutory reference documents from references/. Returns ONLY the reference text without repeating skill instructions. Available files: {reference_files:?}"
                    ),
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "reference_file": {
                                "type": "string",
                                "enum": reference_files,
                                "description": "Exact filename from references/"
                            }
                        },
                        "required": ["reference_file"]
                    }
                }
            ]
        }),

        "tools/call" => {
            let params = rpc_req.params.as_ref();
            let tool_name = params.and_then(|p| p["name"].as_str()).unwrap_or("");
            let args = params.and_then(|p| p["arguments"].as_object());

            match tool_name {
                "load_chapter_skill" => {
                    let Some(skill_file) = args.and_then(|a| a.get("skill_file")).and_then(|v| v.as_str()) else {
                        return rpc_err(rpc_req.id, -32602, "Missing `skill_file` argument");
                    };

                    if skill_file == GENERAL_SKILL_FILE {
                        return rpc_err(
                            rpc_req.id,
                            -32602,
                            "general.md is already loaded at session start; pick a chapter skill.",
                        );
                    }

                    let Some(asset) = SkillAssets::get(skill_file) else {
                        return rpc_err(rpc_req.id, -32602, "Chapter skill file not found");
                    };

                    let body = std::str::from_utf8(asset.data.as_ref())
                        .map(|raw| parse_skill(raw).body)
                        .unwrap_or("Encoding error");

                    json!({
                        "content": [{
                            "type": "text",
                            "text": body
                        }]
                    })
                }

                "get_reference_material" => {
                    let Some(ref_file) = args.and_then(|a| a.get("reference_file")).and_then(|v| v.as_str()) else {
                        return rpc_err(rpc_req.id, -32602, "Missing `reference_file` argument");
                    };

                    let Some(asset) = ReferenceAssets::get(ref_file) else {
                        return rpc_err(rpc_req.id, -32602, "Reference file not found");
                    };

                    let content = std::str::from_utf8(asset.data.as_ref()).unwrap_or("Encoding error");

                    json!({
                        "content": [{
                            "type": "text",
                            "text": content
                        }]
                    })
                }

                _ => return rpc_err(rpc_req.id, -32601, "Tool not found"),
            }
        }

        _ => return rpc_err(rpc_req.id, -32601, "Method not found"),
    };

    RpcOutcome::Reply(RpcResponse {
        jsonrpc: "2.0",
        id: rpc_req.id,
        result: Some(result),
        error: None,
    })
}

fn rpc_err(id: Option<Value>, code: i32, message: &str) -> RpcOutcome {
    RpcOutcome::Reply(RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(json!({
            "code": code,
            "message": message
        })),
    })
}

// ---------- Cloudflare Worker HTTP Entry Points ----------

#[event(start)]
fn start() {
    console_error_panic_hook::set_once();
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/mcp", handle_mcp)
        .run(req, env)
        .await
}

async fn handle_mcp(mut req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    // shortcut: fallback to random UUID only if header is missing, utilizing getrandom js feature
    let session_id = req
        .headers()
        .get("mcp-session-id")?
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let rpc_req: RpcRequest = match req.json().await {
        Ok(r) => r,
        Err(e) => {
            console_error!("Failed to parse JSON-RPC: {}", e);
            let RpcOutcome::Reply(err_body) = rpc_err(None, -32700, "Parse error") else {
                unreachable!()
            };
            return Response::from_json(&err_body);
        }
    };

    match process_rpc(rpc_req) {
        RpcOutcome::Notification => {
            let mut resp = Response::empty()?.with_status(202);
            resp.headers_mut().set("mcp-session-id", &session_id)?;
            Ok(resp)
        }
        RpcOutcome::Reply(body) => {
            let mut resp = Response::from_json(&body)?;
            resp.headers_mut().set("mcp-session-id", &session_id)?;
            Ok(resp)
        }
    }
}

// ---------- Invariant & Protocol Tests (`cargo test`) ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invariant_1_general_skill_exists_and_has_body() {
        let file = SkillAssets::get(GENERAL_SKILL_FILE)
            .expect("INVARIANT BROKEN: skills/general.md must exist");
        let raw = std::str::from_utf8(file.data.as_ref())
            .expect("INVARIANT BROKEN: skills/general.md must be valid UTF-8");
        let parsed = parse_skill(raw);
        assert!(
            !parsed.body.is_empty(),
            "INVARIANT BROKEN: skills/general.md body cannot be empty"
        );
    }

    #[test]
    fn invariant_2_chapter_skills_have_valid_frontmatter_and_exclude_general() {
        let chapters = list_chapter_files();
        assert!(
            !chapters.contains(&GENERAL_SKILL_FILE.to_owned()),
            "INVARIANT BROKEN: general.md must never appear in chapter skill enum"
        );
        assert!(
            !chapters.is_empty(),
            "INVARIANT BROKEN: at least one chapter skill must exist in skills/"
        );

        for filename in chapters {
            let file = SkillAssets::get(&filename).unwrap();
            let raw = std::str::from_utf8(file.data.as_ref())
                .unwrap_or_else(|_| panic!("INVARIANT BROKEN: {filename} is not valid UTF-8"));
            let doc = parse_skill(raw);
            assert!(
                !doc.name.is_empty(),
                "INVARIANT BROKEN: {filename} is missing `name:` in YAML frontmatter"
            );
            assert!(
                !doc.description.is_empty(),
                "INVARIANT BROKEN: {filename} is missing `description:` in YAML frontmatter"
            );
            assert!(
                !doc.body.is_empty(),
                "INVARIANT BROKEN: {filename} has an empty markdown body"
            );
        }
    }

    #[test]
    fn invariant_3_general_skill_never_injected_into_tool_calls() {
        let general_raw = SkillAssets::get(GENERAL_SKILL_FILE).unwrap();
        let general_body = parse_skill(std::str::from_utf8(general_raw.data.as_ref()).unwrap()).body;

        let RpcOutcome::Reply(init_resp) = process_rpc(RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "initialize".into(),
            params: None,
        }) else {
            panic!("Expected Reply for initialize");
        };
        let instructions = init_resp.result.unwrap()["instructions"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(instructions.contains(general_body));

        let mut chapters = list_chapter_files();
        if let Some(first_chapter) = chapters.pop() {
            let RpcOutcome::Reply(chapter_resp) = process_rpc(RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(2)),
                method: "tools/call".into(),
                params: Some(json!({
                    "name": "load_chapter_skill",
                    "arguments": { "skill_file": first_chapter }
                })),
            }) else {
                panic!("Expected Reply for load_chapter_skill");
            };
            let chapter_text = chapter_resp.result.unwrap()["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned();
            assert!(
                !chapter_text.contains(general_body),
                "INVARIANT BROKEN: load_chapter_skill must not re-inject general.md"
            );
        }
    }
}
