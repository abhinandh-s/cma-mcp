use std::collections::HashSet;
use std::path::Path;

fn reference_files() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("references");
    let mut files = std::fs::read_dir(root)
        .expect("references directory must exist")
        .map(|entry| {
            entry.expect("reference directory entry must be readable")
                .path()
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn reference_filenames_are_unique() {
    let files = reference_files();
    let unique = files.iter().collect::<HashSet<_>>();
    assert_eq!(files.len(), unique.len(), "duplicate reference filenames found");
}

#[test]
fn reference_filenames_follow_canonical_pattern() {
    let files = reference_files();
    let malformed = files
        .iter()
        .filter(|name| {
            let stem = name.strip_suffix(".md").unwrap_or(name);
            let parts = stem.split('-').collect::<Vec<_>>();
            parts.len() < 4
                || !parts[0].starts_with('p')
                || parts[1].len() != 3
                || !parts[1][..2].chars().all(|c| c.is_ascii_digit())
                || !matches!(parts[1].as_bytes()[2].to_ascii_lowercase(), b'j' | b'd')
                || !matches!(parts[2], "mqp" | "pyq" | "ptp")
        })
        .collect::<Vec<_>>();
    assert!(
        malformed.is_empty(),
        "malformed reference filenames: {malformed:?}"
    );
}

#[test]
fn reference_documents_are_not_empty_stubs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("references");
    let stubs = reference_files()
        .into_iter()
        .filter_map(|name| {
            let path = root.join(&name);
            let content = std::fs::read_to_string(&path).ok()?;
            let normalized = content.trim();
            (normalized.len() < 500 || normalized.ends_with("### Answer:"))
                .then_some(name)
        })
        .collect::<Vec<_>>();
    assert!(
        stubs.is_empty(),
        "incomplete reference documents detected: {stubs:?}"
    );
}
