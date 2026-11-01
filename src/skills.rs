use crate::catalog::{parse_skill, resolve_subject_skill_file, CatalogCache, SkillAssets, GENERAL_SKILL_FILE};

/// Extracts the raw body of `general.md`
pub fn get_general_skill() -> &'static str {
    static GENERAL_BODY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    GENERAL_BODY.get_or_init(|| {
        SkillAssets::get(GENERAL_SKILL_FILE)
            .and_then(|f| std::str::from_utf8(f.data.as_ref()).ok().map(|s| parse_skill(s).body.to_owned()))
            .unwrap_or_else(|| "# General CMA Study Blueprint".to_owned())
    })
}

/// Builds the stacked system context: General Skill + (Optional) Subject Skill
pub fn build_system_context(subject_hint: Option<&str>, cat: &CatalogCache) -> String {
    let general = get_general_skill();
    let mut out = String::with_capacity(general.len() + 2048);

    // 1. General blueprint is always active
    out.push_str("# GENERAL CMA TUTOR BLUEPRINT\n\n");
    out.push_str(general);

    // 2. Dynamically attach subject skill if a subject/paper is identified
    if let Some(hint) = subject_hint.filter(|h| !h.trim().is_empty()) {
        if let Some(file) = resolve_subject_skill_file(hint, &cat.subject_files) {
            if let Some(asset) = SkillAssets::get(file) {
                if let Ok(raw) = std::str::from_utf8(asset.data.as_ref()) {
                    let doc = parse_skill(raw);
                    out.push_str("\n\n---\n\n# SPECIALIZED SUBJECT SKILL: ");
                    out.push_str(if doc.name.is_empty() { file } else { doc.name });
                    out.push_str("\n\n");
                    out.push_str(doc.body);
                }
            }
        }
    }

    out
}
