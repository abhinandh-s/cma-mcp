use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RefQuery {
    paper: String,            // "p1" .. "p20c"
    term: Option<String>,     // e.g. "26j", "26d"
    doc_type: Option<DocType>,
    variant: Option<PaperVariant>,
    syllabus: Option<String>, // e.g. "syl22"
}

fn normalize_paper_code(raw: &str) -> Option<&'static str> {
    let cleaned = raw.trim().to_ascii_lowercase();
    let stripped = cleaned
        .strip_prefix("paper-")
        .or_else(|| cleaned.strip_prefix("paper"))
        .or_else(|| cleaned.strip_prefix('p'))
        .unwrap_or(&cleaned);

    let candidate = format!("p{stripped}");
    VALID_PAPERS.iter().copied().find(|&p| p == candidate)
}

fn is_term_token(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 3
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && matches!(b[2].to_ascii_lowercase(), b'j' | b'd')
}

impl RefQuery {
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
                continue;
            }
            if doc_type.is_none() {
                if let Some(dt) = DocType::parse(&lower) {
                    doc_type = Some(dt);
                    continue;
                }
            }
            if syllabus.is_none() && lower.starts_with("syl") && lower.len() > 3 {
                syllabus = Some(lower);
                continue;
            }
            if variant.is_none() {
                if let Some(v) = PaperVariant::parse(&lower) {
                    variant = Some(v);
                    continue;
                }
            }
            return None;
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
        if self.term.is_some() && self.term != file_spec.term {
            return false;
        }
        if self.doc_type.is_some() && self.doc_type != file_spec.doc_type {
            return false;
        }
        if let Some(v) = self.variant {
            let file_v = file_spec.variant.unwrap_or(PaperVariant::Question);
            if v != file_v {
                return false;
            }
        }
        if self.syllabus.is_some() && self.syllabus != file_spec.syllabus {
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
            if let Some(sfx) = self.variant.and_then(|v| v.canonical_suffix(dt)) {
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

// ---------- Isolate-Cached Asset Catalog ----------

struct CatalogCache {
    subject_files: Vec<String>,
    reference_files: Vec<String>,
    subject_catalog: String,
    general_instructions: String,
}

static CATALOG: OnceLock<CatalogCache> = OnceLock::new();

fn catalog() -> &'static CatalogCache {
    CATALOG.get_or_init(|| {
        let subject_files = list_subject_skill_files();
        let reference_files = list_reference_files();
        let subject_catalog = format_subject_catalog(&subject_files);
        let general_instructions = build_general_instructions(&subject_catalog);
        CatalogCache {
            subject_files,
            reference_files,
            subject_catalog,
            general_instructions,
        }
    })
}

fn list_subject_skill_files() -> Vec<String> {
    let mut files: Vec<String> = SkillAssets::iter()
        .filter(|f| f.as_ref() != GENERAL_SKILL_FILE && f.ends_with(".md"))
        .map(|f| f.into_owned())
        .collect();
    files.sort();
    files
}

fn format_subject_catalog(subject_files: &[String]) -> String {
    subject_files
        .iter()
        .map(|filename| {
            if let Some(file) = SkillAssets::get(filename) {
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

fn build_general_instructions(subject_catalog: &str) -> String {
    let general_body = SkillAssets::get(GENERAL_SKILL_FILE)
        .and_then(|f| {
            std::str::from_utf8(f.data.as_ref())
                .ok()
                .map(|s| parse_skill(s).body.to_owned())
        })
        .unwrap_or_else(|| "# Error: skills/general.md missing".to_owned());

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
    if let Some(exact) = available.iter().find(|f| f.eq_ignore_ascii_case(trimmed)) {
        return Some(exact.clone());
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
            candidates.iter().any(|c| {
                lower == *c
                    || lower.starts_with(&format!("{code}-"))
                    || lower.starts_with(&format!("paper-{num}-"))
            })
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

// ---------- Pure MCP State Machine ----------

fn process_rpc(rpc_req: RpcRequest) -> RpcOutcome {
    if rpc_req.id.is_none() || rpc_req.method.starts_with("notifications/") {
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
            "cacheScope": "public",
            "_meta": server_meta()
        }),

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
            "instructions": cat.general_instructions,
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
                        "text": cat.general_instructions
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

            for file in &cat.subject_files {
                resources.push(json!({
                    "uri": format!("cma://skills/{file}"),
                    "name": file,
                    "mimeType": "text/markdown",
                    "description": "Subject-specific ICMAI CMA paper skill"
                }));
            }
            for file in &cat.reference_files {
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
                        "Load the subject-specific ICMAI CMA skill (statutory rules, formats, working note templates, and examiner trap zones) for Papers 1 to 20C. Use alongside the general CMA blueprint. Call ONCE per paper when a user asks about a topic from that paper.\nAvailable subject skill files:\n{}",
                        cat.subject_catalog
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
                                "enum": cat.subject_files,
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
                        Available files: {:?}",
                        cat.reference_files
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
                "load_subject_skill" | "load_chapter_skill" => {
                    handle_load_subject_skill(args, &cat.subject_files)
                }
                "get_exam_paper" | "search_study_materials" | "get_reference_material" => {
                    handle_get_exam_paper(args, &cat.reference_files)
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
        if let Some(asset) = ReferenceAssets::get(q_str) {
            let text = std::str::from_utf8(asset.data.as_ref()).unwrap_or("Encoding error");
            return tool_text_result(text, false);
        }

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

// ---------- Cloudflare Worker HTTP Entry Points ----------

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
