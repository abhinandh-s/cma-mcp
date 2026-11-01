/// Valid ICMAI CMA papers: Foundation (1-4), Inter (5-12), Final (13-19, Electives 20A-20C).
pub const VALID_PAPERS: &[&str] = &[
    "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11", "p12", "p13", "p14",
    "p15", "p16", "p17", "p18", "p19", "p20a", "p20b", "p20c",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocType {
    Pyq,
    Mqp,
    Ptp,
}

impl DocType {
    pub fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("pyq") {
            Some(Self::Pyq)
        } else if s.eq_ignore_ascii_case("mqp") {
            Some(Self::Mqp)
        } else if s.eq_ignore_ascii_case("ptp") {
            Some(Self::Ptp)
        } else {
            None
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pyq => "pyq",
            Self::Mqp => "mqp",
            Self::Ptp => "ptp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperVariant {
    Question,        // default (no suffix)
    SuggestedAnswer, // sa (PYQ) or a (PTP)
    Set1,            // s1 (MQP)
    Set1Answer,      // s1a (MQP)
    Set2,            // s2 (MQP)
    Set2Answer,      // s2a (MQP)
}

impl PaperVariant {
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty()
            || s.eq_ignore_ascii_case("q")
            || s.eq_ignore_ascii_case("qp")
            || s.eq_ignore_ascii_case("question")
        {
            Some(Self::Question)
        } else if s.eq_ignore_ascii_case("sa")
            || s.eq_ignore_ascii_case("a")
            || s.eq_ignore_ascii_case("ans")
            || s.eq_ignore_ascii_case("answer")
        {
            Some(Self::SuggestedAnswer)
        } else if s.eq_ignore_ascii_case("s1") {
            Some(Self::Set1)
        } else if s.eq_ignore_ascii_case("s1a") {
            Some(Self::Set1Answer)
        } else if s.eq_ignore_ascii_case("s2") {
            Some(Self::Set2)
        } else if s.eq_ignore_ascii_case("s2a") {
            Some(Self::Set2Answer)
        } else {
            None
        }
    }

    pub const fn canonical_suffix(self, doc_type: DocType) -> Option<&'static str> {
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
pub struct RefQuery {
    pub paper: &'static str,          // "p1" .. "p20c"
    pub term: Option<String>,         // e.g. "26j", "26d"
    pub doc_type: Option<DocType>,
    pub variant: Option<PaperVariant>,
    pub syllabus: Option<String>,     // e.g. "syl22"
}

pub fn strip_ascii_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn strip_ascii_suffix<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let split_pos = s.len().checked_sub(suffix.len())?;
    if s.is_char_boundary(split_pos) && s[split_pos..].eq_ignore_ascii_case(suffix) {
        Some(&s[..split_pos])
    } else {
        None
    }
}

pub fn normalize_paper_code(raw: &str) -> Option<&'static str> {
    let cleaned = raw.trim();
    let stripped = strip_ascii_prefix(cleaned, "paper-")
        .or_else(|| strip_ascii_prefix(cleaned, "paper"))
        .or_else(|| strip_ascii_prefix(cleaned, "p"))
        .unwrap_or(cleaned);

    VALID_PAPERS
        .iter()
        .copied()
        .find(|p| p[1..].eq_ignore_ascii_case(stripped))
}

pub fn is_term_token(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 3
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && matches!(b[2].to_ascii_lowercase(), b'j' | b'd')
}

impl RefQuery {
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        let stem = strip_ascii_suffix(trimmed, ".md").unwrap_or(trimmed);

        let mut tokens = stem
            .split(|c: char| c == '-' || c == '_' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty());

        let first = tokens.next()?;
        let paper = normalize_paper_code(first)?;

        let mut term = None;
        let mut doc_type = None;
        let mut variant = None;
        let mut syllabus = None;

        for tok in tokens {
            if term.is_none() && is_term_token(tok) {
                term = Some(tok.to_ascii_lowercase());
                continue;
            }
            if doc_type.is_none() {
                if let Some(dt) = DocType::parse(tok) {
                    doc_type = Some(dt);
                    continue;
                }
            }
            if syllabus.is_none()
                && strip_ascii_prefix(tok, "syl").is_some_and(|rest| !rest.is_empty())
            {
                syllabus = Some(tok.to_ascii_lowercase());
                continue;
            }
            if variant.is_none() {
                if let Some(v) = PaperVariant::parse(tok) {
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

    pub fn matches_spec(&self, file_spec: &Self) -> bool {
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

    pub fn canonical_prefix(&self) -> String {
        let mut out = String::with_capacity(24);
        out.push_str(self.paper);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shorthand_and_filenames() {
        let q1 = RefQuery::parse("p7-26d-mqp-s1-syl22.md").expect("valid filename");
        assert_eq!(q1.paper, "p7");
        assert_eq!(q1.term.as_deref(), Some("26d"));
        assert_eq!(q1.doc_type, Some(DocType::Mqp));
        assert_eq!(q1.variant, Some(PaperVariant::Set1));
        assert_eq!(q1.syllabus.as_deref(), Some("syl22"));

        let q2 = RefQuery::parse("Paper-20C 26J pyq sa").expect("valid shorthand");
        assert_eq!(q2.paper, "p20c");
        assert_eq!(q2.canonical_prefix(), "p20c-26j-pyq-sa");
    }
}
