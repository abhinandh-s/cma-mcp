use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use worker::*;

#[derive(RustEmbed)]
#[folder = "skills/"]
struct SkillAssets;

#[derive(RustEmbed)]
#[folder = "references/"]
struct ReferenceAssets;

const GENERAL_SKILL_FILE: &str = "general.md";
const MCP_VERSION_2026: &str = "2026-07-28";
const SUPPORTED_VERSIONS: &[&str] = &[MCP_VERSION_2026, "2025-11-25", "2024-11-05"];
const SERVER_NAME: &str = "cmapaper-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const CACHE_TTL_MS: u64 = 3_600_000;

/// Valid ICMAI CMA papers: Foundation (1-4), Inter (5-12), Final (13-19, Electives 20A-20C).
pub const VALID_PAPERS: &[&str] = &[
    "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11", "p12", "p13", "p14",
    "p15", "p16", "p17", "p18", "p19", "p20a", "p20b", "p20c",
];

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
    // shortcut: line-based frontmatter parser for `key: value` — revisit when multi-line YAML scalars are needed
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

// ---------- ICMAI Paper & Reference Filename Parser ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocType {
    Pyq,
    Mqp,
    Ptp,
}

impl DocType {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "pyq" => Some(Self::Pyq),
            "mqp" => Some(Self::Mqp),
            "ptp" => Some(Self::Ptp),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Pyq => "pyq",
            Self::Mqp => "mqp",
            Self::Ptp => "ptp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaperVariant {
    Question,        // default (no suffix)
    SuggestedAnswer, // `sa` (PYQ) or `a` (PTP)
    Set1,            // `s1` (MQP)
    Set1Answer,      // `s1a` (MQP)
    Set2,            // `s2` (MQP)
    Set2Answer,      // `s2a` (MQP)
}

impl PaperVariant {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "" | "q" | "qp" | "question" => Some(Self::Question),
            "sa" | "a" | "ans" | "answer" => Some(Self::SuggestedAnswer),
            "s1" => Some(Self::Set1),
            "s1a" => Some(Self::Set1Answer),
            "s2" => Some(Self::Set2),
            "s2a" => Some(Self::Set2Answer),
            _ => None,
        }
    }

    const fn canonical_suffix(self, doc_type: DocType) -> Option<&'static str> {
        match (doc_type, self) {
            (_, Self::Question) => None,
            (DocType::Pyq, Self::SuggestedAnswer) => Some("sa"),
            (DocType::Ptp, Self::SuggestedAnswer) => Some("a"),
            (_, Self::SuggestedAnswer) => Some("sa"),
            (_, Self::Set1) => Some("s1"),
            (_, Self::Set1Answer) => Some("s1a"),
            (_, Self::Set2) => Some("s2"),
            (_, Self::Set2Answer) => Some("s2a"),
        }
    }
}

/// Canonical representation of an ICMAI exam reference key:
/// e.g. `p7-26d-mqp-s1-syl22.md` or shorthand `p5 26j pyq sa`
#[derive(Debug, Clone, PartialEq, Eq)]
struct RefQuery {
    paper: String,            // "p1" .. "p20c"
    term: Option<String>,     // e.g. "26j", "26d"
    doc_type: Option<DocType>,
    variant: Option<PaperVariant>,
    syllabus: Option<String>, // e.g. "syl22"
}

fn normalize_paper_code(raw: &str) -> Option<&'static str> {
    let cleaned = raw
        .trim()
        .to_ascii_lowercase();
    let stripped = cleaned
        .strip_prefix("paper-")
        .or_else(|| cleaned.strip_prefix("paper"))
        .or_else(|| cleaned.strip_prefix('p'))
        .unwrap_or(&cleaned);

    let candidate = format!("p{stripped}");
    VALID_PAPERS.iter().copied().find(|&p| p == candidate)
}

fn is_term_token(t: &str) -> bool {
    // Matches 2-digit year + session char ('j' for June, 'd' for Dec), e.g. "26j", "26d", "25d"
    let b = t.as_bytes();
    b.len() == 3
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && matches!(b[2].to_ascii_lowercase(), b'j' | b'd')
}

impl RefQuery {
    /// Parses either a filename (`p7-26d-mqp-s1-syl22.md`) or shorthand (`p5 26j mqp s1a`).
    fn parse(input: &str) -> Option<Self> {
        let stem = input
            .trim()
            .strip_suffix(".md")
            .unwrap_or_else(|| input.trim());

        let mut tokens = stem
            .split(|c: char| c == '-' || c == '_' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty());

        let first = tokens.next()?;
        let paper = normalize_paper_code(first)?.to_owned();

        let mut term = None;
        let mut doc_type = None;
        let mut variant = None;
        let mut syllabus = None;

        for tok in tokens {
            let lower = tok.to_ascii_lowercase();
            if term.is_none() && is_term_token(&lower) {
                term = Some(lower);
            } else if doc_type.is_none() && let Some(dt) = DocType::parse(&lower) {
                doc_type = Some(dt);
            } else if lower.starts_with("syl") && lower.len() > 3 {
                syllabus = Some(lower);
            } else if variant.is_none() && let Some(v) = PaperVariant::parse(&lower) {
                variant = Some(v);
            } else {
                return None;
            }
        }

        Some(Self {
            paper,
            term,
            doc_type,
            variant,
            syllabus,
        })
    }

    fn matches_filename(&self, filename: &str) -> bool {
        let Some(file_spec) = Self::parse(filename) else {
            return false;
        };
        if self.paper != file_spec.paper {
            return false;
        }
        if let Some(ref t) = self.term && file_spec.term.as_ref() != Some(t) {
            return false;
        }
        if let Some(dt) = self.doc_type && file_spec.doc_type != Some(dt) {
            return false;
        }
        if let Some(v) = self.variant {
            let file_v = file_spec.variant.unwrap_or(PaperVariant::Question);
            if v != file_v {
                return false;
            }
        }
        if let Some(ref syl) = self.syllabus && file_spec.syllabus.as_ref() != Some(syl) {
            return false;
        }
        true
    }

    fn canonical_prefix(&self) -> String {
        let mut out = self.paper.clone();
        if let Some(ref t) = self.term {
            out.push('-');
            out.push_str(t);
        }
        if let Some(dt) = self.doc_type {
            out.push('-');
            out.push_str(dt.as_str());
            if let Some(v) = self.variant
                && let Some(sfx) = v.canonical_suffix(dt)
            {
                out.push('-');
                out.push_str(sfx);
            }
        }
        if let Some(ref syl) = self.syllabus {
            out.push('-');
            out.push_str(syl);
        }
        out
    }
}

// ---------- Asset Catalog Helpers ----------

fn list_subject_skill_files() -> Vec<String> {
    let mut files: Vec<String> = SkillAssets::iter()
        .filter(|f| f.as_ref() != GENERAL_SKILL_FILE && f.ends_with(".md"))
        .map(|f| f.into_owned())
        .collect();
    files.sort();
    files
}

fn format_subject_catalog() -> String {
    list_subject_skill_files()
        .into_iter()
        .map(|filename| {
            if let Some(file) = SkillAssets::get(&filename)
                && let Ok(raw) = std::str::from_utf8(file.data.as_ref())
            {
                let doc = parse_skill(raw);
                return format!("- `{filename}` ({}) — {}", doc.name, doc.description);
            }
            format!("- `{filename}`")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn load_general_instructions() -> String {
    let general_body = SkillAssets::get(GENERAL_SKILL_FILE)
        .and_then(|f| {
            std::str::from_utf8(f.data.as_ref())
                .ok()
                .map(|s| parse_skill(s).body.to_owned())
        })
        .unwrap_or_else(|| "# Error: skills/general.md missing".to_owned());

    let subject_catalog = format_subject_catalog();

    format!(
        "{general_body}\n\n---\n## ICMAI Paper Skill Routing (Paper 1 to Paper 20C)\n\
        The general CMA blueprint above is active for this entire chat. When a user asks about any topic from ICMAI Papers 1 to 20C, call `load_subject_skill` ONCE for that subject/paper before answering (do not re-call if already loaded in this chat):\n{subject_catalog}"
    )
}

fn list_reference_files() -> Vec<String> {
    let mut files: Vec<String> = ReferenceAssets::iter()
        .filter(|f| !f.starts_with('.') && f.ends_with(".md"))
        .map(|f| f.into_owned())
        .collect();
    files.sort();
    files
}

fn resolve_subject_skill_file(input: &str, available: &[String]) -> Option<String> {
    let trimmed = input.trim();
    if available.iter().any(|f| f.eq_ignore_ascii_case(trimmed)) {
        return available
            .iter()
            .find(|f| f.eq_ignore_ascii_case(trimmed))
            .cloned();
    }
    let code = normalize_paper_code(trimmed)?;
    let num = code.strip_prefix('p').unwrap_or(code);
    let candidates = [
        format!("{code}.md"),
        format!("paper-{num}.md"),
        format!("p-{num}.md"),
    ];
    available
        .iter()
        .find(|f| {
            let lower = f.to_ascii_lowercase();
            candidates.iter().any(|c| lower == *c || lower.starts_with(&format!("{code}-")) || lower.starts_with(&format!("paper-{num}-")))
        })
        .cloned()
}

// ---------- MCP 2026-07-28 Result & Error Builders ----------

fn server_meta() -> Value {
    json!({
        "io.modelcontextprotocol/serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION
        }
    })
}

/// Standard MCP 2026-07-28 tool result (`isError: false` for success, `isError: true` for model-recoverable execution/validation errors).
fn tool_text_result(text: impl Into<String>, is_error: bool) -> Value {
    let mut res = json!({
        "resultType": "complete",
        "content": [{
            "type": "text",
            "text": text.into()
        }],
        "_meta": server_meta()
    });
    if is_error {
        res["isError"] = json!(true);
    }
    res
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

// ---------- Pure MCP State Machine (Dual-Era: 2026-07-28 + 2024-11-05/2025-11-25) ----------

fn process_rpc(rpc_req: RpcRequest) -> RpcOutcome {
    if rpc_req.id.is_none() || rpc_req.method.starts_with("notifications/") {
        return RpcOutcome::Notification;
    }

    let subject_files = list_subject_skill_files();
    let reference_files = list_reference_files();
    let subject_catalog = format_subject_catalog();

    let result = match rpc_req.method.as_str() {
        // MCP 2026-07-28 stateless discovery endpoint (MUST implement)
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
            "instructions": load_general_instructions(),
            "ttlMs": CACHE_TTL_MS,
            "cacheScope": "public",
            "_meta": server_meta()
        }),

        // Legacy handshake kept for dual-era client compatibility
        "initialize" => json!({
            "resultType": "complete",
            "protocolVersion": MCP_VERSION_2026,
            "capabilities": {
                "prompts": {},
                "resources": {},
                "tools": {}
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION
            },
            "instructions": load_general_instructions(),
            "_meta": server_meta()
        }),

        "ping" => json!({ "resultType": "complete" }),

        "prompts/list" => json!({
            "resultType": "complete",
            "ttlMs": CACHE_TTL_MS,
            "cacheScope": "public",
            "prompts": [{
                "name": "start-cma-session",
                "title": "Start CMA Study Session (General Skill)",
                "description": "Loads the general CMA blueprint and Paper 1–20C routing instructions."
            }],
            "_meta": server_meta()
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
                "resultType": "complete",
                "description": "General CMA Tutor Blueprint",
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": load_general_instructions()
                    }
                }],
                "_meta": server_meta()
            })
        }

        "resources/list" => {
            let mut resources = vec![json!({
                "uri": "cma://skills/general.md",
                "name": "General CMA Tutor Skill",
                "mimeType": "text/markdown",
                "description": "Core CMA tutoring blueprint active across all papers"
            })];

            for file in &subject_files {
                resources.push(json!({
                    "uri": format!("cma://skills/{file}"),
                    "name": file,
                    "mimeType": "text/markdown",
                    "description": "Subject-specific ICMAI CMA paper skill"
                }));
            }
            for file in &reference_files {
                resources.push(json!({
                    "uri": format!("cma://references/{file}"),
                    "name": file,
                    "mimeType": "text/markdown",
                    "description": "ICMAI CMA PYQ / MQP / PTP reference document"
                }));
            }

            json!({
                "resultType": "complete",
                "ttlMs": CACHE_TTL_MS,
                "cacheScope": "public",
                "resources": resources,
                "_meta": server_meta()
            })
        }

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
                    "resultType": "complete",
                    "ttlMs": CACHE_TTL_MS,
                    "cacheScope": "public",
                    "contents": [{
                        "uri": uri,
                        "mimeType": "text/markdown",
                        "text": text
                    }],
                    "_meta": server_meta()
                })
            } else if let Some(ref_file) = uri.strip_prefix("cma://references/") {
                let Some(asset) = ReferenceAssets::get(ref_file) else {
                    return rpc_err(rpc_req.id, -32602, "Reference resource not found");
                };
                let text = std::str::from_utf8(asset.data.as_ref()).unwrap_or("Encoding error");
                json!({
                    "resultType": "complete",
                    "ttlMs": CACHE_TTL_MS,
                    "cacheScope": "public",
                    "contents": [{
                        "uri": uri,
                        "mimeType": "text/markdown",
                        "text": text
                    }],
                    "_meta": server_meta()
                })
            } else {
                return rpc_err(
                    rpc_req.id,
                    -32602,
                    "Resource URI must start with cma://skills/ or cma://references/",
                );
            }
        }

        "tools/list" => json!({
            "resultType": "complete",
            "ttlMs": CACHE_TTL_MS,
            "cacheScope": "public",
            "tools": [
                {
                    "name": "load_subject_skill",
                    "title": "Load ICMAI CMA Subject Skill (Paper 1 to 20C)",
                    "description": format!(
                        "Load the subject-specific ICMAI CMA skill (statutory rules, formats, working note templates, and examiner trap zones) for Papers 1 to 20C. Use alongside the general CMA blueprint. Call ONCE per paper when a user asks about a topic from that paper.\nAvailable subject skill files:\n{subject_catalog}"
                    ),
                    "inputSchema": {
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "type": "object",
                        "properties": {
                            "paper": {
                                "type": "string",
                                "description": "Paper code (`p1` to `p20c`, e.g. `p5`, `p7`, `p19`, `p20c`) or exact skill filename from `skills/`."
                            },
                            "skill_file": {
                                "type": "string",
                                "enum": subject_files,
                                "description": "Exact subject skill filename from `skills/` (optional if `paper` is provided)."
                            }
                        },
                        "additionalProperties": false
                    },
                    "annotations": {
                        "title": "Load ICMAI Subject Skill",
                        "readOnlyHint": true,
                        "destructiveHint": false,
                        "idempotentHint": true,
                        "openWorldHint": false
                    }
                },
                {
                    "name": "get_exam_paper",
                    "title": "Fetch ICMAI PYQ, MQP, or PTP Exam Material",
                    "description": format!(
                        "Fetch ICMAI Previous Year Question Papers (PYQ), Model Question Papers (MQP), Practice Test Papers (PTP), and Suggested Answers/Answer Keys for Papers 1 to 20C.\n\
                        Naming convention:\n\
                        - Full filename: `p7-26d-mqp-s1-syl22.md` (Paper 7, 2026 Dec, MQP Set 1, Syllabus 2022)\n\
                        - PYQ shorthand: `p5 26j pyq` (Question Paper), `p5 26j pyq sa` (Suggested Answer)\n\
                        - MQP shorthand: `p5 26j mqp s1` (Set 1), `p5 26j mqp s1a` (Set 1 Answer), `p5 26j mqp s2` (Set 2), `p5 26j mqp s2a` (Set 2 Answer)\n\
                        - PTP shorthand: `p5 26j ptp` (Question Paper), `p5 26j ptp a` (Answer Key)\n\
                        Available files: {reference_files:?}"
                    ),
                    "inputSchema": {
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Shorthand or exact filename, e.g. `p7-26d-mqp-s1-syl22.md`, `p5 26j pyq sa`, `p5 26j mqp s1a`, or `p5 26j ptp a`."
                            },
                            "paper": {
                                "type": "string",
                                "enum": VALID_PAPERS,
                                "description": "ICMAI Paper number from `p1` to `p20c` (e.g. `p5`, `p7`, `p20c`)."
                            },
                            "term": {
                                "type": "string",
                                "description": "Exam term as 2-digit year + `j` (June) or `d` (Dec), e.g. `26j`, `26d`, `25d`."
                            },
                            "doc_type": {
                                "type": "string",
                                "enum": ["pyq", "mqp", "ptp"],
                                "description": "`pyq` (Previous Year Question Paper), `mqp` (Model Question Paper), or `ptp` (Practice Test Paper)."
                            },
                            "variant": {
                                "type": "string",
                                "enum": ["q", "sa", "s1", "s1a", "s2", "s2a", "a"],
                                "description": "`q` (question paper), `sa` (PYQ suggested answer), `s1`/`s2` (MQP set 1/2), `s1a`/`s2a` (MQP set 1/2 answer), `a` (PTP answer key)."
                            },
                            "syllabus": {
                                "type": "string",
                                "description": "Syllabus code, e.g. `syl22` (defaults to matching any available syllabus if omitted)."
                            }
                        },
                        "additionalProperties": false
                    },
                    "annotations": {
                        "title": "Fetch ICMAI PYQ / MQP / PTP",
                        "readOnlyHint": true,
                        "destructiveHint": false,
                        "idempotentHint": true,
                        "openWorldHint": false
                    }
                }
            ],
            "_meta": server_meta()
        }),

        "tools/call" => {
            let params = rpc_req.params.as_ref();
            let tool_name = params.and_then(|p| p["name"].as_str()).unwrap_or("");
            let args = params.and_then(|p| p["arguments"].as_object());

            match tool_name {
                // Accept both `load_subject_skill` and legacy `load_chapter_skill`
                "load_subject_skill" | "load_chapter_skill" => {
                    handle_load_subject_skill(args, &subject_files)
                }
                // Accept `get_exam_paper` plus legacy tool names for smooth migration
                "get_exam_paper" | "search_study_materials" | "get_reference_material" => {
                    handle_get_exam_paper(args, &reference_files)
                }
                // Per MCP 2026-07-28 `/server/tools`: unknown tool is a protocol error (-32602)
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

fn handle_load_subject_skill(
    args: Option<&Map<String, Value>>,
    subject_files: &[String],
) -> Value {
    let raw_arg = args
        .and_then(|a| a.get("skill_file").or_else(|| a.get("paper")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if raw_arg.is_empty() {
        return tool_text_result(
            format!(
                "Missing required argument: provide `paper` (`p1`..`p20c`) or `skill_file`. Available skills: {subject_files:?}"
            ),
            true,
        );
    }

    if raw_arg.eq_ignore_ascii_case(GENERAL_SKILL_FILE) {
        return tool_text_result(
            "general.md is already loaded via session instructions; select a subject skill from Paper 1 to Paper 20C.",
            true,
        );
    }

    let Some(resolved_file) = resolve_subject_skill_file(raw_arg, subject_files) else {
        return tool_text_result(
            format!(
                "Subject skill not found for '{raw_arg}'. Valid papers: p1..p20c. Available skill files: {subject_files:?}"
            ),
            true,
        );
    };

    let Some(asset) = SkillAssets::get(&resolved_file) else {
        return tool_text_result(
            format!("Subject skill asset '{resolved_file}' could not be read."),
            true,
        );
    };

    let body = std::str::from_utf8(asset.data.as_ref())
        .map(|raw| parse_skill(raw).body)
        .unwrap_or("Encoding error");

    tool_text_result(body, false)
}

fn handle_get_exam_paper(
    args: Option<&Map<String, Value>>,
    reference_files: &[String],
) -> Value {
    // 1. Check direct filename or shorthand `query` string
    if let Some(q_str) = args
        .and_then(|a| {
            a.get("query")
                .or_else(|| a.get("filename"))
                .or_else(|| a.get("reference_file"))
        })
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Exact match in embedded assets
        if let Some(asset) = ReferenceAssets::get(q_str) {
            let text = std::str::from_utf8(asset.data.as_ref()).unwrap_or("Encoding error");
            return tool_text_result(text, false);
        }

        // Parse shorthand / filename pattern (e.g., "p5 26j pyq sa" or "p7-26d-mqp-s1-syl22.md")
        let Some(parsed_query) = RefQuery::parse(q_str) else {
            return tool_text_result(
                format!(
                    "Invalid ICMAI reference query '{q_str}'. Expected format like `p7-26d-mqp-s1-syl22.md`, `p5 26j pyq`, `p5 26j pyq sa`, `p5 26j mqp s1a`, or `p5 26j ptp a`. Available files: {reference_files:?}"
                ),
                true,
            );
        };

        return match_and_read_reference(&parsed_query, reference_files);
    }

    // 2. Otherwise build RefQuery from structured parameters (`paper`, `term`, `doc_type`, `variant`, `syllabus`)
    let Some(raw_paper) = args.and_then(|a| a.get("paper")).and_then(|v| v.as_str()) else {
        return tool_text_result(
            format!(
                "Provide either `query` (e.g. `p5 26j pyq sa`) or `paper` (`p1`..`p20c`). Available files: {reference_files:?}"
            ),
            true,
        );
    };

    let Some(paper) = normalize_paper_code(raw_paper) else {
        return tool_text_result(
            format!("Invalid paper '{raw_paper}'. Must be between `p1` and `p20c`."),
            true,
        );
    };

    let term = match args.and_then(|a| a.get("term")).and_then(|v| v.as_str()) {
        Some(t) if is_term_token(t.trim()) => Some(t.trim().to_ascii_lowercase()),
        Some(t) => {
            return tool_text_result(
                format!(
                    "Invalid term '{t}'. Expected 2-digit year + `j` (June) or `d` (Dec), e.g. `26j`, `26d`."
                ),
                true,
            )
        }
        None => None,
    };

    let doc_type = match args.and_then(|a| a.get("doc_type")).and_then(|v| v.as_str()) {
        Some(dt) => match DocType::parse(dt.trim()) {
            Some(parsed) => Some(parsed),
            None => {
                return tool_text_result(
                    format!("Invalid doc_type '{dt}'. Expected `pyq`, `mqp`, or `ptp`."),
                    true,
                )
            }
        },
        None => None,
    };

    let variant = match args.and_then(|a| a.get("variant")).and_then(|v| v.as_str()) {
        Some(v) => match PaperVariant::parse(v.trim()) {
            Some(parsed) => Some(parsed),
            None => {
                return tool_text_result(
                    format!(
                        "Invalid variant '{v}'. Expected `q`, `sa`, `s1`, `s1a`, `s2`, `s2a`, or `a`."
                    ),
                    true,
                )
            }
        },
        None => None,
    };

    let syllabus = args
        .and_then(|a| a.get("syllabus"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    let query = RefQuery {
        paper: paper.to_owned(),
        term,
        doc_type,
        variant,
        syllabus,
    };

    match_and_read_reference(&query, reference_files)
}

fn match_and_read_reference(query: &RefQuery, reference_files: &[String]) -> Value {
    let matches: Vec<&String> = reference_files
        .iter()
        .filter(|f| query.matches_filename(f))
        .collect();

    match matches.as_slice() {
        [single] => {
            let Some(asset) = ReferenceAssets::get(single.as_str()) else {
                return tool_text_result(format!("Asset '{single}' unreadable."), true);
            };
            let content = std::str::from_utf8(asset.data.as_ref()).unwrap_or("Encoding error");
            tool_text_result(content, false)
        }
        [] => tool_text_result(
            format!(
                "No reference file found matching `{}` (expected `{}-syl22.md` or `{}.md`). Available files: {reference_files:?}",
                query.canonical_prefix(),
                query.canonical_prefix(),
                query.canonical_prefix()
            ),
            true,
        ),
        many => {
            let list = many
                .iter()
                .map(|s| format!("- `{s}`"))
                .collect::<Vec<_>>()
                .join("\n");
            tool_text_result(
                format!(
                    "Multiple files match `{}`. Call `get_exam_paper` with the exact `query` filename:\n{list}",
                    query.canonical_prefix()
                ),
                false,
            )
        }
    }
}

// ---------- Cloudflare Worker HTTP Entry Points (Stateless Streamable HTTP) ----------

#[event(start)]
fn start() {
    console_error_panic_hook::set_once();
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/mcp", handle_mcp_post)
        .get("/mcp", |_, _| Response::error("Method Not Allowed", 405))
        .delete("/mcp", |_, _| Response::error("Method Not Allowed", 405))
        .run(req, env)
        .await
}

async fn handle_mcp_post(mut req: Request, _ctx: RouteContext<()>) -> Result<Response> {
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
        RpcOutcome::Notification => Ok(Response::empty()?.with_status(202)),
        RpcOutcome::Reply(body) => Response::from_json(&body),
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
    fn invariant_2_subject_skills_have_valid_frontmatter_and_exclude_general() {
        let subjects = list_subject_skill_files();
        assert!(
            !subjects.contains(&GENERAL_SKILL_FILE.to_owned()),
            "INVARIANT BROKEN: general.md must never appear in subject skill list"
        );
        assert!(
            !subjects.is_empty(),
            "INVARIANT BROKEN: at least one subject skill must exist in skills/"
        );

        for filename in subjects {
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
        let general_body =
            parse_skill(std::str::from_utf8(general_raw.data.as_ref()).unwrap()).body;

        let RpcOutcome::Reply(disc_resp) = process_rpc(RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "server/discover".into(),
            params: None,
        }) else {
            panic!("Expected Reply for server/discover");
        };
        let result = disc_resp.result.unwrap();
        assert_eq!(result["resultType"], "complete");
        assert!(
            result["instructions"]
                .as_str()
                .unwrap()
                .contains(general_body)
        );

        let mut subjects = list_subject_skill_files();
        if let Some(first_subject) = subjects.pop() {
            let RpcOutcome::Reply(subj_resp) = process_rpc(RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(2)),
                method: "tools/call".into(),
                params: Some(json!({
                    "name": "load_subject_skill",
                    "arguments": { "skill_file": first_subject }
                })),
            }) else {
                panic!("Expected Reply for load_subject_skill");
            };
            let res = subj_resp.result.unwrap();
            assert_eq!(res["resultType"], "complete");
            assert!(res.get("isError").is_none());
            let subj_text = res["content"][0]["text"].as_str().unwrap();
            assert!(
                !subj_text.contains(general_body),
                "INVARIANT BROKEN: load_subject_skill must not re-inject general.md"
            );
        }
    }

    #[test]
    fn icmai_filename_and_shorthand_conventions_parse_and_match() {
        // Full hyphenated filename with syllabus
        let full = RefQuery::parse("p7-26d-mqp-s1-syl22.md").unwrap();
        assert_eq!(full.paper, "p7");
        assert_eq!(full.term.as_deref(), Some("26d"));
        assert_eq!(full.doc_type, Some(DocType::Mqp));
        assert_eq!(full.variant, Some(PaperVariant::Set1));
        assert_eq!(full.syllabus.as_deref(), Some("syl22"));

        // 1. PYQ Question Paper & Suggested Answer
        let pyq_q = RefQuery::parse("p5 26j pyq").unwrap();
        assert!(pyq_q.matches_filename("p5-26j-pyq-syl22.md"));
        assert!(!pyq_q.matches_filename("p5-26j-pyq-sa-syl22.md"));

        let pyq_sa = RefQuery::parse("p5 26j pyq sa").unwrap();
        assert!(pyq_sa.matches_filename("p5-26j-pyq-sa-syl22.md"));
        assert!(!pyq_sa.matches_filename("p5-26j-pyq-syl22.md"));

        // 2. MQP Set 1 / Set 1a / Set 2 / Set 2a
        let mqp_s1 = RefQuery::parse("p5 26j mqp s1").unwrap();
        assert!(mqp_s1.matches_filename("p5-26j-mqp-s1-syl22.md"));
        assert!(!mqp_s1.matches_filename("p5-26j-mqp-s1a-syl22.md"));

        let mqp_s1a = RefQuery::parse("p5 26j mqp s1a").unwrap();
        assert!(mqp_s1a.matches_filename("p5-26j-mqp-s1a-syl22.md"));

        let mqp_s2 = RefQuery::parse("p5 26j mqp s2").unwrap();
        assert!(mqp_s2.matches_filename("p5-26j-mqp-s2-syl22.md"));

        let mqp_s2a = RefQuery::parse("p5 26j mqp s2a").unwrap();
        assert!(mqp_s2a.matches_filename("p5-26j-mqp-s2a-syl22.md"));

        // 3. PTP Question Paper & Answer Key (`a`)
        let ptp_q = RefQuery::parse("p5 26j ptp").unwrap();
        assert!(ptp_q.matches_filename("p5-26j-ptp-syl22.md"));
        assert!(!ptp_q.matches_filename("p5-26j-ptp-a-syl22.md"));

        let ptp_a = RefQuery::parse("p5 26j ptp a").unwrap();
        assert!(ptp_a.matches_filename("p5-26j-ptp-a-syl22.md"));

        // Elective Paper 20C
        let p20c = RefQuery::parse("p20c 26d pyq sa").unwrap();
        assert_eq!(p20c.paper, "p20c");
        assert!(p20c.matches_filename("p20c-26d-pyq-sa-syl22.md"));
    }

    #[test]
    fn mcp_2026_07_28_error_semantics_distinguish_protocol_vs_tool_errors() {
        // Unknown tool -> JSON-RPC -32602 Protocol Error
        let RpcOutcome::Reply(unknown_tool) = process_rpc(RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(10)),
            method: "tools/call".into(),
            params: Some(json!({ "name": "nonexistent_tool", "arguments": {} })),
        }) else {
            panic!("Expected Reply");
        };
        assert_eq!(unknown_tool.error.unwrap()["code"], -32602);

        // Bad tool argument -> Tool Execution Error (`result.isError = true`, NOT JSON-RPC error)
        let RpcOutcome::Reply(bad_arg) = process_rpc(RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(11)),
            method: "tools/call".into(),
            params: Some(json!({
                "name": "load_subject_skill",
                "arguments": { "skill_file": "general.md" }
            })),
        }) else {
            panic!("Expected Reply");
        };
        assert!(bad_arg.error.is_none());
        let res = bad_arg.result.unwrap();
        assert_eq!(res["resultType"], "complete");
        assert_eq!(res["isError"], true);
    }
}
