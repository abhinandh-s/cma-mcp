use crate::parser::{normalize_paper_code, strip_ascii_prefix, RefQuery, VALID_PAPERS};
use rust_embed::RustEmbed;
use serde_json::{json, Value};
use std::sync::OnceLock;

#[derive(RustEmbed)]
#[folder = "skills/"]
pub struct SkillAssets;

#[derive(RustEmbed)]
#[folder = "references/"]
pub struct ReferenceAssets;

pub const GENERAL_SKILL_FILE: &str = "general.md";
pub const SERVER_NAME: &str = "cmapaper-mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CACHE_TTL_MS: u64 = 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillDoc<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub body: &'a str,
}

pub fn parse_skill(raw: &str) -> SkillDoc<'_> {
    // shortcut: line-based frontmatter parser for key: value — revisit when multi-line YAML scalars are needed
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

pub struct IndexedRef {
    pub filename: String,
    pub spec: Option<RefQuery>,
}

pub struct CatalogCache {
    pub subject_files: Vec<String>,
    pub reference_files: Vec<String>,
    pub indexed_refs: Vec<IndexedRef>,
    pub general_instructions: String,
    pub tools_list_payload: Value,
    pub resources_list_payload: Value,
    pub prompts_list_payload: Value,
}

static CATALOG: OnceLock<CatalogCache> = OnceLock::new();

pub fn catalog() -> &'static CatalogCache {
    CATALOG.get_or_init(|| {
        let subject_files = list_subject_skill_files();
        let reference_files = list_reference_files();
        let indexed_refs = reference_files
            .iter()
            .map(|f| IndexedRef {
                filename: f.clone(),
                spec: RefQuery::parse(f),
            })
            .collect();

        let subject_catalog = format_subject_catalog(&subject_files);
        let general_instructions = build_general_instructions(&subject_catalog);
        let tools_list_payload =
            build_tools_list_payload(&subject_files, &reference_files, &subject_catalog);
        let resources_list_payload = build_resources_list_payload(&subject_files, &reference_files);
        let prompts_list_payload = build_prompts_list_payload();

        CatalogCache {
            subject_files,
            reference_files,
            indexed_refs,
            general_instructions,
            tools_list_payload,
            resources_list_payload,
            prompts_list_payload,
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

fn list_reference_files() -> Vec<String> {
    let mut files: Vec<String> = ReferenceAssets::iter()
        .filter(|f| !f.starts_with('.') && f.ends_with(".md"))
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
                    return format!("- {filename} ({}) — {}", doc.name, doc.description);
                }
            }
            format!("- {filename}")
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

fn build_prompts_list_payload() -> Value {
    json!({
        "prompts": [{
            "name": "start-cma-session",
            "title": "Start CMA Study Session (General Skill)",
            "description": "Loads the general CMA blueprint and Paper 1–20C routing instructions."
        }]
    })
}

fn build_resources_list_payload(subject_files: &[String], reference_files: &[String]) -> Value {
    let mut resources = Vec::with_capacity(1 + subject_files.len() + reference_files.len());
    resources.push(json!({
        "uri": "cma://skills/general.md",
        "name": "General CMA Tutor Skill",
        "mimeType": "text/markdown",
        "description": "Core CMA tutoring blueprint active across all papers"
    }));

    for file in subject_files {
        resources.push(json!({
            "uri": format!("cma://skills/{file}"),
            "name": file,
            "mimeType": "text/markdown",
            "description": "Subject-specific ICMAI CMA paper skill"
        }));
    }
    for file in reference_files {
        resources.push(json!({
            "uri": format!("cma://references/{file}"),
            "name": file,
            "mimeType": "text/markdown",
            "description": "ICMAI CMA PYQ / MQP / PTP reference document"
        }));
    }

    json!({ "resources": resources })
}

fn build_tools_list_payload(
    subject_files: &[String],
    reference_files: &[String],
    subject_catalog: &str,
) -> Value {
    let mut skill_file_prop = json!({
        "type": "string",
        "description": "Exact subject skill filename from `skills/` (optional if `paper` is provided)."
    });
    if !subject_files.is_empty() {
        skill_file_prop["enum"] = json!(subject_files);
    }

    json!({
        "tools": [
            {
                "name": "load_subject_skill",
                "title": "Load ICMAI CMA Subject Skill (Paper 1 to 20C)",
                "description": format!(
                    "Load the subject-specific ICMAI CMA skill (statutory rules, formats, working note templates, and examiner trap zones) for Papers 1 to 20C. Use alongside the general CMA blueprint. Call ONCE per paper when a user asks about a topic from that paper.\nAvailable subject skill files:\n{subject_catalog}"
                ),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "paper": {
                            "type": "string",
                            "description": "Paper code (`p1` to `p20c`, e.g. `p5`, `p7`, `p19`, `p20c`) or exact skill filename from `skills/`."
                        },
                        "skill_file": skill_file_prop
                    }
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
                    }
                },
                "annotations": {
                    "title": "Fetch ICMAI PYQ / MQP / PTP",
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false
                }
            }
        ]
    })
}

pub fn resolve_subject_skill_file<'a>(input: &str, available: &'a [String]) -> Option<&'a str> {
    let trimmed = input.trim();
    if let Some(exact) = available.iter().find(|f| f.eq_ignore_ascii_case(trimmed)) {
        return Some(exact.as_str());
    }
    let code = normalize_paper_code(trimmed)?;
    let num = &code[1..];

    available
        .iter()
        .find(|f| {
            let name = f.as_str();
            strip_ascii_prefix(name, code).is_some_and(|r| r.eq_ignore_ascii_case(".md") || r.starts_with('-'))
                || strip_ascii_prefix(name, "paper-")
                    .and_then(|r| strip_ascii_prefix(r, num))
                    .is_some_and(|r| r.eq_ignore_ascii_case(".md") || r.starts_with('-'))
                || strip_ascii_prefix(name, "p-")
                    .and_then(|r| strip_ascii_prefix(r, num))
                    .is_some_and(|r| r.eq_ignore_ascii_case(".md"))
        })
        .map(String::as_str)
}
