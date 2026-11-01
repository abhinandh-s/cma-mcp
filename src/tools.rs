use crate::catalog::{
    parse_skill, resolve_subject_skill_file, CatalogCache, ReferenceAssets, SkillAssets,
    GENERAL_SKILL_FILE,
};
use crate::parser::{is_term_token, normalize_paper_code, DocType, PaperVariant, RefQuery};
use serde_json::{json, Map, Value};

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

pub fn handle_load_subject_skill(args: Option<&Map<String, Value>>, cat: &CatalogCache) -> Value {
    let raw_arg = args
        .and_then(|a| {
            a.get("skill_file")
                .or_else(|| a.get("paper"))
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
        return tool_text_result(
            "general.md is already loaded via session instructions; select a subject skill from Paper 1 to Paper 20C.",
            true,
        );
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

    let Some(asset) = SkillAssets::get(resolved_file) else {
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
            return tool_text_result(text, false);
        }

        let Some(parsed_query) = RefQuery::parse(q_str) else {
            return tool_text_result(
                format!(
                    "Invalid ICMAI reference query '{q_str}'. Expected format like `p7-26d-mqp-s1-syl22.md`, `p5 26j pyq`, `p5 26j pyq sa`, `p5 26j mqp s1a`, or `p5 26j ptp a`. Available files: {:?}",
                    cat.reference_files
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
                cat.reference_files
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
                format!(
                    "Invalid term '{t}'. Expected 2-digit year + `j` (June) or `d` (Dec), e.g. `26j`, `26d`."
                ),
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
            tool_text_result(content, false)
        }
        [] => {
            let prefix = query.canonical_prefix();
            tool_text_result(
                format!(
                    "No reference file found matching `{prefix}` (expected `{prefix}-syl22.md` or `{prefix}.md`). Available files: {:?}",
                    cat.reference_files
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

use crate::skills::{build_system_context, Subject};

pub fn handle_get_paper_and_skill(paper_id: &str) -> Result<String, String> {
    let subject_opt = Subject::from_id(paper_id);
    
    // 1. Assemble base instructions + subject skill
    let system_instructions = build_system_context(Some(paper_id));
    
    // 2. Fetch the requested question paper / suggested answers
    let paper_content = match paper_id {
        "p19" => include_str!("../references/p19-26j-mqp-s1-syl22.md"),
        "p8"  => include_str!("../references/p8-26d-mqp-s1-syl22.md"),
        "p10" => include_str!("../references/p10-26d-mqp-s1-syl22.md"),
        "p12" => include_str!("../references/p12-26d-mqp-s1-syl22.md"),
        "p17" => include_str!("../references/p17-26d-mqp-s1-syl22.md"),
        "p20a" => include_str!("../references/p20A-26d-mqp-s1-syl22.md"),
        "p20c" => include_str!("../references/p20C-26d-mqp-s1-syl22.md"),
        _ => return Err(format!("Unknown paper ID: {}", paper_id)),
    };

    // 3. Format unified output payload
    Ok(format!(
        "<tutoring_guidelines>\n{}\n</tutoring_guidelines>\n\n<source_material>\n{}\n</source_material>",
        system_instructions,
        paper_content
    ))
}
