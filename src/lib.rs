mod catalog;
mod parser;
mod rpc;
mod tools;

use crate::catalog::{SERVER_NAME, SERVER_VERSION};
use crate::rpc::{process_payload, rpc_err_response, BatchOutcome, RpcPayload};
use serde_json::json;
use worker::*;

fn with_cors(mut resp: Response) -> Result<Response> {
    let headers = resp.headers_mut();
    headers.set("Access-Control-Allow-Origin", "*")?;
    headers.set("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS")?;
    headers.set(
        "Access-Control-Allow-Headers",
        "Content-Type, Accept, Authorization, Mcp-Session-Id, Mcp-Protocol-Version, Last-Event-ID",
    )?;
    headers.set(
        "Access-Control-Expose-Headers",
        "Mcp-Session-Id, Mcp-Protocol-Version",
    )?;
    Ok(resp)
}

#[event(start)]
fn start() {
    console_error_panic_hook::set_once();
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/mcp", handle_mcp_post)
        .post_async("/mcp/", handle_mcp_post)
        .post_async("/", handle_mcp_post)
        .get("/mcp", handle_mcp_get)
        .get("/mcp/", handle_mcp_get)
        .get("/", handle_mcp_get)
        .delete("/mcp", handle_mcp_delete)
        .delete("/mcp/", handle_mcp_delete)
        .delete("/", handle_mcp_delete)
        .options("/mcp", handle_mcp_options)
        .options("/mcp/", handle_mcp_options)
        .options("/", handle_mcp_options)
        .run(req, env)
        .await
}

fn handle_mcp_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    with_cors(Response::empty()?.with_status(204))
}

fn handle_mcp_delete(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    with_cors(Response::empty()?.with_status(204))
}

fn handle_mcp_get(req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let accept = req.headers().get("Accept")?.unwrap_or_default();
    if accept.contains("text/event-stream") {
        // Stateless Streamable HTTP: 405 instructs MCP clients not to hold open a background GET SSE stream.
        return with_cors(Response::error("Method Not Allowed", 405)?);
    }
    let body = json!({
        "name": SERVER_NAME,
        "version": SERVER_VERSION,
        "status": "ok",
        "endpoint": "/mcp"
    });
    with_cors(Response::from_json(&body)?)
}

async fn handle_mcp_post(mut req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let payload: RpcPayload = match req.json().await {
        Ok(p) => p,
        Err(e) => {
            console_error!("Failed to parse JSON-RPC: {}", e);
            let err_body = rpc_err_response(None, -32700, "Parse error");
            return with_cors(Response::from_json(&err_body)?);
        }
    };

    match process_payload(payload) {
        BatchOutcome::NotificationOnly => with_cors(Response::empty()?.with_status(202)),
        BatchOutcome::Single(body) => with_cors(Response::from_json(&body)?),
        BatchOutcome::Batch(bodies) => with_cors(Response::from_json(&bodies)?),
    }
}
