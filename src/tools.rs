use crate::catalog::{
    resolve_subject_skill_file, CatalogCache, ReferenceAssets, GENERAL_SKILL_FILE,
};
use crate::parser::{is_term_token, normalize_paper_code, DocType, PaperVariant, RefQuery};
use crate::skills::build_system_context;
use serde_json::{json, Map, Value};

#[inline]
pub fn tool_text_result(text: impl Into<String>, is_error: bool) -> Value {
    let mut res = json!({
        "content": [{
            "type": "text",
            "text": text.into()
        }]
    });
    if is_error {
        res["isError"] = json!(true);
    }
    res
}

/// Handles `load_subject_skill`: returns General Skill + Specific Subject Skill
pub fn handle_load_subject_skill(args: Option<&Map<String, Value>>, cat: &CatalogCache) -> Value {
    let raw_arg = args
        .and_then(|a| {
            a.get("paper")
                .or_else(|| a.get("skill_file"))
                .or_else(|| a.get("query"))
        })
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();

    if raw_arg.is_empty() {
        return tool_text_result(
            format!(
                "Missing required argument: provide `paper` (`p1`..`p20c`) or `skill_file`. Available skills: {:?}",
                cat.subject_files
            ),
            true,
        );
    }

    if raw_arg.eq_ignore_ascii_case(GENERAL_SKILL_FILE) {
        return tool_text_result(build_system_context(None, cat), false);
    }

    let Some(resolved_file) = resolve_subject_skill_file(raw_arg, &cat.subject_files) else {
        return tool_text_result(
            format!(
                "Subject skill not found for '{raw_arg}'. Valid papers: p1..p20c. Available skill files: {:?}",
                cat.subject_files
            ),
            true,
        );
    };

    // Stacks General Skill + Subject Skill
    let full_context = build_system_context(Some(resolved_file), cat);
    tool_text_result(full_context, false)
}

/// Handles `get_exam_paper`: fetches requested paper and embeds the general/subject context
pub fn handle_get_exam_paper(args: Option<&Map<String, Value>>, cat: &CatalogCache) -> Value {
    if let Some(q_str) = args
        .and_then(|a| {
            a.get("query")
                .or_else(|| a.get("filename"))
                .or_else(|| a.get("reference_file"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(asset) = ReferenceAssets::get(q_str) {
            let text = std::str::from_utf8(asset.data.as_ref()).unwrap_or("Encoding error");
            return bundle_paper_response(q_str, text, cat);
        }

        let Some(parsed_query) = RefQuery::parse(q_str) else {
            return tool_text_result(
                format!(
                    "Invalid ICMAI reference query '{q_str}'. Expected format like `p7-26d-mqp-s1-syl22.md`, `p5 26j pyq`, or `p5 26j mqp s1a`. Use `list_exam_papers` to discover available files.",
                ),
                true,
            );
        };

        return match_and_read_reference(&parsed_query, cat);
    }

    let Some(raw_paper) = args.and_then(|a| a.get("paper")).and_then(Value::as_str) else {
        return tool_text_result(
            format!(
                "Provide either `query` (e.g. `p5 26j pyq sa`) or `paper` (`p1`..`p20c`). Available files: {:?}",
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

    let term = match args.and_then(|a| a.get("term")).and_then(Value::as_str) {
        Some(t) if is_term_token(t.trim()) => Some(t.trim().to_ascii_lowercase()),
        Some(t) => {
            return tool_text_result(
                format!("Invalid term '{t}'. Expected 2-digit year + `j` or `d`, e.g. `26j`, `26d`."),
                true,
            )
        }
        None => None,
    };

    let doc_type = match args.and_then(|a| a.get("doc_type")).and_then(Value::as_str) {
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

    let variant = match args.and_then(|a| a.get("variant")).and_then(Value::as_str) {
        Some(v) => match PaperVariant::parse(v.trim()) {
            Some(parsed) => Some(parsed),
            None => {
                return tool_text_result(
                    format!("Invalid variant '{v}'. Expected `q`, `sa`, `s1`, `s1a`, `s2`, `s2a`, or `a`."),
                    true,
                )
            }
        },
        None => None,
    };

    let syllabus = args
        .and_then(|a| a.get("syllabus"))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    let query = RefQuery {
        paper,
        term,
        doc_type,
        variant,
        syllabus,
    };

    match_and_read_reference(&query, cat)
}


/// Lists reference filenames with optional filters and offset-based pagination.
pub fn handle_list_exam_papers(args: Option<&Map<String, Value>>, cat: &CatalogCache) -> Value {
    let get_str = |key: &str| args.and_then(|a| a.get(key)).and_then(Value::as_str)
        .map(str::trim).filter(|s| !s.is_empty());
    let paper = match get_str("paper") {
        Some(raw) => match normalize_paper_code(raw) {
            Some(code) => Some(code),
            None => return tool_text_result(format!("Invalid paper '{raw}'. Expected p1..p20c."), true),
        },
        None => None,
    };
    let term = match get_str("term") {
        Some(raw) if is_term_token(raw) => Some(raw.to_ascii_lowercase()),
        Some(raw) => return tool_text_result(format!("Invalid term '{raw}'. Expected 26j or 26d."), true),
        None => None,
    };
    let doc_type = match get_str("doc_type") {
        Some(raw) => match DocType::parse(raw) {
            Some(v) => Some(v),
            None => return tool_text_result("Invalid doc_type. Expected pyq, mqp, or ptp.", true),
        },
        None => None,
    };
    let variant = match get_str("variant") {
        Some(raw) => match PaperVariant::parse(raw) {
            Some(v) => Some(v),
            None => return tool_text_result("Invalid variant. Expected q, sa, s1, s1a, s2, s2a, or a.", true),
        },
        None => None,
    };
    let syllabus = get_str("syllabus").map(str::to_ascii_lowercase);
    let limit = args.and_then(|a| a.get("limit")).and_then(Value::as_u64).unwrap_or(50).clamp(1, 100) as usize;
    let offset = args.and_then(|a| a.get("offset")).and_then(Value::as_u64).unwrap_or(0) as usize;
    let filtered: Vec<&str> = cat.indexed_refs.iter().filter(|item| {
        let Some(spec) = item.spec.as_ref() else { return false; };
        if paper.is_some_and(|v| v != spec.paper) { return false; }
        if term.as_ref().is_some_and(|v| spec.term.as_ref() != Some(v)) { return false; }
        if doc_type.is_some_and(|v| spec.doc_type != Some(v)) { return false; }
        if variant.is_some_and(|v| spec.variant.unwrap_or(PaperVariant::Question) != v) { return false; }
        if syllabus.as_ref().is_some_and(|v| spec.syllabus.as_ref() != Some(v)) { return false; }
        true
    }).map(|item| item.filename.as_str()).collect();
    let total = filtered.len();
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);
    tool_text_result(json!({
        "total": total, "offset": start, "limit": limit,
        "next_offset": if end < total { Some(end) } else { None },
        "files": &filtered[start..end]
    }).to_string(), false)
}

fn match_and_read_reference(query: &RefQuery, cat: &CatalogCache) -> Value {
    let matches: Vec<&str> = cat
        .indexed_refs
        .iter()
        .filter(|item| item.spec.as_ref().is_some_and(|spec| query.matches_spec(spec)))
        .map(|item| item.filename.as_str())
        .collect();

    match matches.as_slice() {
        [single] => {
            let Some(asset) = ReferenceAssets::get(single) else {
                return tool_text_result(format!("Asset '{single}' unreadable."), true);
            };
            let content = std::str::from_utf8(asset.data.as_ref()).unwrap_or("Encoding error");
            bundle_paper_response(single, content, cat)
        }
        [] => {
            let prefix = query.canonical_prefix();
            tool_text_result(
                format!(
                    "No reference file found matching `{prefix}`. Available files: {:?}",
                ),
                true,
            )
        }
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

/// Helper that wraps the exam paper with active study guidelines
fn bundle_paper_response(filename: &str, content: &str, cat: &CatalogCache) -> Value {
    // Look up the parsed paper code (e.g., "p19", "p8") from indexed references
    let paper_hint = cat
        .indexed_refs
        .iter()
        .find(|item| item.filename == filename)
        .and_then(|item| item.spec.as_ref())
        .map(|spec| spec.paper)
        .unwrap_or(filename);

    let guidelines = build_system_context(Some(paper_hint), cat);
    let payload = format!(
        "<tutoring_guidelines>\n{guidelines}\n</tutoring_guidelines>\n\n<source_material filename=\"{filename}\">\n{content}\n</source_material>"
    );
    tool_text_result(payload, false)
}