# Reference Corpus Audit Findings

This report records repository-level anomalies verified against the uploaded 674-paper audit and the current `master` tree.

## Verified repository inventory

- The current tree contains **678** Markdown files under `references/`.
- The 674-paper audit accounts for 26J (134), 25D (124), 25J (124), 24D (124), 24J (80), and 23D (88), totaling 674.
- The repository also contains **four 26D MQP files**. These explain the exact difference between the audit's 674-paper scope and the 678 files currently embedded by the server.
- There are no duplicate reference filenames in the tree.
- The four June 2024 Foundation MQP Set 1 answer files are only 296–307 bytes and contain metadata plus an empty `### Answer:` heading. These are incomplete reference documents, not valid complete answer keys:
  - `references/p1-24j-mqp-s1a-syl22.md`
  - `references/p2-24j-mqp-s1a-syl22.md`
  - `references/p3-24j-mqp-s1a-syl22.md`
  - `references/p4-24j-mqp-s1a-syl22.md`

## Verified content defects

- `references/p1-26j-mqp-s1-syl22.md` contains the MCQ artifact pattern described in audit defect D03: option markers are rendered as `a)`–`d)` but the source text retains a dangling opening parenthesis before the option list. The content also uses inconsistent `## 1` question numbering and includes repeated subject/running-header text inside question content.
- `references/p10-26j-mqp-s1-syl22.md` contains inline Directorate/running-header text in Section A and duplicated heading styles in question content, consistent with D02. It is not safe to remove text solely by global string replacement because subject-name occurrences may be legitimate exam content.
- `references/p11-26j-mqp-s1-syl22.md` has a severely corrupted 9-column table near Questions 6–8, plus duplicated raw and table representations of the same material. The repeated content is visibly inconsistent and cannot be reliably repaired without the source PDF.

## Implementation-layer finding

The Rust project embeds the pre-converted Markdown files with `rust-embed`. No PDF extraction/conversion pipeline is present in this repository. Therefore, extraction heuristics described in D03, D10–D13 must be fixed in the upstream converter, not in this MCP serving layer.

## Audit-report arithmetic / scope caveats

The audit states 674 files and 613 FAIL + 61 PASS, which reconciles. However, its Tier 1 subcounts (92 + 165 + 110) total 367, not the stated 408; Tier 3 subcounts (22 + 175 + 34) total 231; Tier 4 subcounts (283 + 92) total 375; Tier 5 subcounts (337 + 94) total 431. The Tier 1 summary therefore needs clarification of 41 additional affected files or correction of the total.
The report's per-term counts sum to 674 exactly. The repository's additional 4 files are the 26D MQPs and should be explicitly included in any future audit scope.

## Recommended remediation order

1. Restore the four empty answer keys from their source answer documents or remove them from the advertised corpus until complete.
2. Correct the converter, with regression fixtures for option parsing, running headers, wide tables, glyph maps, and table segmentation.
3. Reconvert affected references from original PDFs. Avoid attempting to reconstruct clipped questions, missing fractions, or lost table cells from Markdown alone.
4. Add a corpus integrity test that checks expected inventory by term, non-empty reference minimum size, parseable naming, no duplicate names, and a small set of known structural invariants.
5. Rerun the complete PDF-to-Markdown audit and publish a machine-readable manifest so future totals can be independently reproduced.
