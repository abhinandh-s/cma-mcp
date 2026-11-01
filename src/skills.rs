pub const GENERAL_SKILL: &str = include_str!("../skills/general.md");

pub enum Subject {
    Paper6Laws,
    Paper7Dt,
    Paper8Costing,
    Paper10CostMgmt,
    Paper12CompanyAccounts,
    Paper17CorpLaws,
    Paper19IndirectTax,
    Paper20ASpmBvm,
    Paper20CRiskMgmt,
}

impl Subject {
    pub fn from_id(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "p6" | "paper6" | "law" => Some(Self::Paper6Laws),
            "p7" | "paper7" | "dt" | "direct_tax" => Some(Self::Paper7Dt),
            "p8" | "paper8" | "costing" | "marginal" => Some(Self::Paper8Costing),
            "p10" | "paper10" => Some(Self::Paper10CostMgmt),
            "p12" | "paper12" => Some(Self::Paper12CompanyAccounts),
            "p17" | "paper17" => Some(Self::Paper17CorpLaws),
            "p19" | "paper19" | "gst" | "itc" | "customs" => Some(Self::Paper19IndirectTax),
            "p20a" | "paper20a" => Some(Self::Paper20ASpmBvm),
            "p20c" | "paper20c" => Some(Self::Paper20CRiskMgmt),
            _ => None,
        }
    }

    pub fn skill_prompt(&self) -> &'static str {
        match self {
            Self::Paper8Costing => include_str!("../skills/costing_marginal_chapter.md"),
            Self::Paper19IndirectTax => include_str!("../skills/gst_itc_chapter.md"),
            // Default placeholder or specific instructions for others
            _ => "",
        }
    }
}
