use crate::catalog::{
    catalog, parse_skill, ReferenceAssets, SkillAssets, CACHE_TTL_MS, SERVER_NAME, SERVER_VERSION,
};
use crate::tools::{handle_get_exam_paper, handle_load_subject_skill};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_MCP_VERSION: &str = "2025-03-26";
pub const SUPPORTED_VERSIONS: &[&str] = &[
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum RpcPayload {
    Single(RpcRequest),
    Batch(Vec<RpcRequest>),
}

#[derive(Deserialize, Debug)]
pub struct RpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

#[derive(Debug, PartialEq)]
pub enum RpcOutcome {
    Notification,
    Reply(RpcResponse),
}

pub enum BatchOutcome {
    NotificationOnly,
    Single(RpcResponse),
    Batch(Vec<RpcResponse>),
}

#[inline]
pub fn rpc_err_response(id: Option<Value>, code: i32, message: &str) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(json!({
            "code": code,
            "message": message
        })),
    }
}

#[inline]
fn rpc_err(id: Option<Value>, code: i32, message: &str) -> RpcOutcome {
    RpcOutcome::Reply(rpc_err_response(id, code, message))
}

pub fn process_payload(payload: RpcPayload) -> BatchOutcome {
    match payload {
        RpcPayload::Single(req) => match process_rpc(req) {
            RpcOutcome::Notification => BatchOutcome::NotificationOnly,
            RpcOutcome::Reply(resp) => BatchOutcome::Single(resp),
        },
        RpcPayload::Batch(reqs) => {
            if reqs.is_empty() {
                return BatchOutcome::Single(rpc_err_response(None, -32600, "Invalid Request"));
            }
            let mut replies = Vec::with_capacity(reqs.len());
            for req in reqs {
                if let RpcOutcome::Reply(resp) = process_rpc(req) {
                    replies.push(resp);
                }
            }
            if replies.is_empty() {
                BatchOutcome::NotificationOnly
            } else {
                BatchOutcome::Batch(replies)
            }
        }
    }
}

pub fn process_rpc(rpc_req: RpcRequest) -> RpcOutcome {
    // Protocol notifications produce no response
    if rpc_req.id.is_none()
        || rpc_req.method == "initialized"
        || rpc_req.method.starts_with("notifications/")
    {
        return RpcOutcome::Notification;
    }

    let cat = catalog();

    let result = match rpc_req.method.as_str() {
        "server/discover" => json!({
            "resultType": "complete",
            "supportedVersions": SUPPORTED_VERSIONS,
            "capabilities": {
                "prompts": {},
                "resources": {},
                "tools": {}
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION
            },
            "instructions": cat.general_instructions,
            "ttlMs": CACHE_TTL_MS,
            "cacheScope": "public"
        }),

        "initialize" => {
            let negotiated_version = rpc_req
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .and_then(|req_ver| SUPPORTED_VERSIONS.iter().copied().find(|&v| v == req_ver))
                .unwrap_or(DEFAULT_MCP_VERSION);

            json!({
                "protocolVersion": negotiated_version,
                "capabilities": {
                    "prompts": {},
                    "resources": {},
                    "tools": {}
                },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": SERVER_VERSION
                },
                "instructions": cat.general_instructions
            })
        }

        "ping" => json!({}),

        "prompts/list" => cat.prompts_list_payload.clone(),

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
                        "text": cat.general_instructions
                    }
                }]
            })
        }

        "resources/list" => cat.resources_list_payload.clone(),

        "resources/templates/list" => json!({ "resourceTemplates": [] }),

        "resources/read" => {
            let uri = rpc_req
                .params
                .as_ref()
                .and_then(|p| p["uri"].as_str())
                .unwrap_or("");

            if let Some(skill_file) = uri.strip_prefix("cma://skills/") {
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
            } else if let Some(ref_file) = uri.strip_prefix("cma://references/") {
                let Some(asset) = ReferenceAssets::get(ref_file) else {
                    return rpc_err(rpc_req.id, -32602, "Reference resource not found");
                };
                let text = std::str::from_utf8(asset.data.as_ref()).unwrap_or("Encoding error");
                json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "text/markdown",
                        "text": text
                    }]
                })
            } else {
                return rpc_err(
                    rpc_req.id,
                    -32602,
                    "Resource URI must start with cma://skills/ or cma://references/",
                );
            }
        }

        "tools/list" => cat.tools_list_payload.clone(),

        "tools/call" => {
            let params = rpc_req.params.as_ref();
            let tool_name = params.and_then(|p| p["name"].as_str()).unwrap_or("");
            let args = params.and_then(|p| p["arguments"].as_object());

            match tool_name {
                "load_subject_skill" | "load_chapter_skill" => {
                    handle_load_subject_skill(args, cat)
                }
                "get_exam_paper" | "search_study_materials" | "get_reference_material" => {
                    handle_get_exam_paper(args, cat)
                }
                _ => return rpc_err(rpc_req.id, -32602, &format!("Unknown tool: {tool_name}")),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_client_protocol_version() {
        let req = RpcRequest {
            jsonrpc: "2.0".to_owned(),
            id: Some(json!(1)),
            method: "initialize".to_owned(),
            params: Some(json!({ "protocolVersion": "2024-11-05" })),
        };
        let RpcOutcome::Reply(resp) = process_rpc(req) else {
            panic!("expected reply");
        };
        assert_eq!(resp.result.unwrap()["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn handles_ping() {
        let req = RpcRequest {
            jsonrpc: "2.0".to_owned(),
            id: Some(json!(2)),
            method: "ping".to_owned(),
            params: None,
        };
        let RpcOutcome::Reply(resp) = process_rpc(req) else {
            panic!("expected reply");
        };
        assert_eq!(resp.result.unwrap(), json!({}));
    }

    #[test]
    fn ignores_notifications() {
        let req = RpcRequest {
            jsonrpc: "2.0".to_owned(),
            id: None,
            method: "notifications/initialized".to_owned(),
            params: None,
        };
        assert_eq!(process_rpc(req), RpcOutcome::Notification);

        let init_req = RpcRequest {
            jsonrpc: "2.0".to_owned(),
            id: Some(json!(3)),
            method: "initialized".to_owned(),
            params: None,
        };
        assert_eq!(process_rpc(init_req), RpcOutcome::Notification);
    }

    #[test]
    fn handles_unknown_tool() {
        let req = RpcRequest {
            jsonrpc: "2.0".to_owned(),
            id: Some(json!(4)),
            method: "tools/call".to_owned(),
            params: Some(json!({
                "name": "non_existent_tool",
                "arguments": {}
            })),
        };
        let RpcOutcome::Reply(resp) = process_rpc(req) else {
            panic!("expected reply");
        };
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap()["code"], -32602);
    }
}
