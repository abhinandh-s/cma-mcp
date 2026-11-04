# ICMAI Official Publication & CMA Connector Availability Registry

**Syllabus:** 2022  
**Snapshot:** June 2026 publication set  
**Authority:** User-supplied consolidated inventory of the ICMAI official website, cross-referenced with the CMA connector's indexed Markdown catalogue. This is a dated snapshot, not a live ICMAI website monitor.

## Status legend

- `[ ]` — Not published by ICMAI in this snapshot.
- `[~]` — Not fully published / publication rollout in progress.
- `[x]` — Published by ICMAI and available in the CMA connector.

Q = question paper; A = suggested answer / answer key. A count is a **file count**: Q and A are counted separately. A dash means no publication is recorded for that document type in the supplied snapshot, not permanent non-publication.

## June 2026 — latest complete publication snapshot

Paper codes P1–P20C comprise 22 papers.

| Document | Papers | Q files | A files | Total |
|---|---|---:|---:|---:|
| PYQ | P1–P20C | 22 | 0 | 22 |
| MQP Set 1 | P1–P20C | 22 | 22 | 44 |
| MQP Set 2 | P1–P12 | 12 | 12 | 24 |
| PTP | P1–P20C | 22 | 22 | 44 |
| **Total** | | **78** | **56** | **134** |

**June 2026 per-paper matrix**

| Papers | PYQ | MQP S1 | MQP S2 | PTP |
|---|---|---|---|---|
| P1–P12 | Q | Q+A | Q+A | Q+A |
| P13–P20C | Q | Q+A | — | Q+A |

June 2026 PYQ suggested answers are not included in this snapshot. MQP Set 2 is recorded only for P1–P12.

## Earlier publication snapshots

Ranges are compact and human-editable. Each row defines the exact paper range and whether Q, A or both are available.

| Term | Document | Paper range | Q files | A files | Total |
|---|---|---|---:|---:|---:|
| Dec 2025 | PYQ | P5–P20C | 18 | 18 | 36 |
| Dec 2025 | MQP Set 1 | P1–P20C | 22 | 22 | 44 |
| Dec 2025 | MQP Set 2 | P1–P20C | 22 | 22 | 44 |
| Dec 2025 | PTP | — | 0 | 0 | 0 |
| Jun 2025 | PYQ | P5–P20C | 18 | 18 | 36 |
| Jun 2025 | MQP Set 1 | P1–P20C | 22 | 22 | 44 |
| Jun 2025 | MQP Set 2 | P1–P20C | 22 | 22 | 44 |
| Jun 2025 | PTP | — | 0 | 0 | 0 |
| Dec 2024 | PYQ | P5–P20C | 18 | 18 | 36 |
| Dec 2024 | MQP Set 1 | P1–P20C | 22 | 22 | 44 |
| Dec 2024 | MQP Set 2 | P1–P20C | 22 | 22 | 44 |
| Dec 2024 | PTP | — | 0 | 0 | 0 |
| Jun 2024 | PYQ | P5–P20C | 18 | 18 | 36 |
| Jun 2024 | MQP Set 1 | P1–P20C | 22 | 22 | 44 |
| Jun 2024 | MQP Set 2 | — | 0 | 0 | 0 |
| Jun 2024 | PTP | — | 0 | 0 | 0 |
| Dec 2023 | PYQ | P5–P20C | 18 | 18 | 36 |
| Dec 2023 | MQP Set 1 | P5–P20C | 18 | 18 | 36 |
| Dec 2023 | MQP Set 2 | P5–P12 | 8 | 8 | 16 |
| Dec 2023 | PTP | — | 0 | 0 | 0 |

## Aggregate counts

| Term | PYQ | MQP S1 | MQP S2 | PTP | Total |
|---|---:|---:|---:|---:|---:|
| Jun 2026 | 22 | 44 | 24 | 44 | 134 |
| Dec 2025 | 36 | 44 | 44 | 0 | 124 |
| Jun 2025 | 36 | 44 | 44 | 0 | 124 |
| Dec 2024 | 36 | 44 | 44 | 0 | 124 |
| Jun 2024 | 36 | 44 | 0 | 0 | 80 |
| Dec 2023 | 36 | 36 | 16 | 0 | 88 |
| **Total** | **202** | **256** | **172** | **44** | **674** |

These counts cover only the supplied Syllabus 2022 publication inventory. One paper with both Q and A contributes two files.

## December 2026 rollout

**Status: [~] Not fully published / in progress.**

December 2026 is intentionally excluded from the totals. Add individual paper/document rows as ICMAI releases them. Do not treat a not-yet-released document as a connector extraction failure.

## Audit notes and reconciliation

1. This registry records the user-supplied consolidated ICMAI publication list and stated bot availability. It is not a live website check.
2. The uploaded `26j_mqp.tar.xz` archive contained 67 MQP PDFs, while this publication inventory records 68 (44 Set 1 + 24 Set 2). The archive appears to omit one PDF relative to the supplied publication inventory. The connector index includes `p12-26j-mqp-s2a-syl22.md`; record this as a source-package discrepancy pending recheck of the original ICMAI listing/PDF.
3. If the ICMAI listing, source PDF package and connector index disagree, track each layer separately: official publication, source PDF presence, connector Markdown presence.
4. Use `list_exam_papers` to verify exact indexed filenames; use this registry to interpret expected official publication coverage.

## How to update

- Add a term to the detail and aggregate tables.
- Use paper ranges when every paper in the range has the same status.
- Split ranges for exceptions (as in June 2026 MQP Set 2).
- Count Q and A separately; total = Q files + A files.
- Keep December 2026 as `[~]` until the rollout is complete.
- Update the snapshot date and audit notes when the source inventory changes.
