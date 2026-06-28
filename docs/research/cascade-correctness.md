# SPARQL→SQL rewriting cascade correctness

**Date:** 2026-06-27 · **Decision:** ADR-0007 (design), ADR-0012 (tests). **Evidence:** [A]/[B]/[C].

## Root cause
SPARQL solution mappings are **partial functions under bag + 3-valued semantics**; SQL tuples are **total functions with NULL under a different 3VL**. Every cascade bug is a leak between the two NULL regimes; `OPTIONAL`→`LEFT JOIN` is the largest leak (Ontop's self-left-join elimination with nullable determinants only shipped correct in **v5.2.0, Aug 2024** — ~a decade in). [A]

## Findings
- **Proof target:** Pérez et al. (TODS 2009) SPARQL semantics + Chebotko et al. (DKE 2009, first provably semantics-preserving SPARQL→SQL); `eval_SQL(τ(Q,M),D) =_bag eval_SPARQL(Q,RDF(M,D))`. Well-designed patterns = coNP; general = PSPACE; OPTIONAL is the jump. [A]
- **Base translation:** Xiao/Kontchakov et al. (ISWC 2018) — `LEFT JOIN` + `COALESCE(shared vars)` + compatibility filter `(a=b OR a IS NULL OR b IS NULL)`, proven bag- & 3VL-faithful (Thm 3). **Adopt as the unoptimized ground truth; don't invent one.** [A thesis / B+ exact formulae]
- **NULL rules R1–R5:** never plain `ON a=b`; `COALESCE` shared vars; re-introduce NULLs the mapping stripped (padding/`IS NOT NULL`); preserve bag multiplicities; FILTER-inside vs after OPTIONAL (never push an outer FILTER onto the preserved side). [A/B]
- **Cascade order is load-bearing** (confirmed by Ontop's version timeline): IRI-template-mismatch pruning **before** self-join elimination; FD inference + transitive closure (v5.1 #681/#732) **before** FK/PK join elimination (v5.2 #783). [A- timeline / B causal]
- **15-item pitfall catalog** incl.: missing COALESCE; outer-FILTER-onto-preserved-side; DISTINCT-removal unsound under bag; **ORDER BY `NULLS FIRST` vs PostgreSQL's `NULLS LAST` default**; EBV vs SQL truthiness; empty-group `SUM` NULL-vs-0. [A/B]
- **No mechanized end-to-end SPARQL→SQL verifier exists.** [B]

## Decision (ADR-0007 + ADR-0012)
Base translation = ISWC-2018 (ground truth); cascade = semantics-preserving IQ→IQ rewrites with invariants INV-0..5. Verify with: native Oxigraph oracle (`materialise ≡ virtualise`), **NoREC internal differential** (unoptimized vs optimized IQ pinpoints the bad rule), **MR1 constraint-toggling** metamorphic test (unsound SQO), per-rule **VeriEQL** bounded equivalence, proptest generators.

## Sources
- Pérez TODS 2009: https://dl.acm.org/doi/10.1145/1567274.1567278 · Chebotko DKE 2009: https://www.sciencedirect.com/science/article/abs/pii/S0169023X09000469
- Xiao et al. ISWC 2018 (OPTIONAL for OBDA): https://arxiv.org/abs/1806.05918 · camera-ready https://titan.dcs.bbk.ac.uk/~roman/papers/iswc18-cr.pdf
- Ontop IQ: https://ontop-vkg.org/dev/internals/iq · releases: https://ontop-vkg.org/guide/releases
- Well-designed: https://arxiv.org/pdf/1712.08809 · weakly-well-designed (ICDT 2016): https://drops.dagstuhl.de/storage/00lipics/lipics-vol048-icdt2016/LIPIcs.ICDT.2016.5/LIPIcs.ICDT.2016.5.pdf
- SQLancer NoREC/TLP: https://www.usenix.org/system/files/osdi20-rigger.pdf · VeriEQL: https://arxiv.org/pdf/2403.03193 · SQL NULL DISTINCT: https://modern-sql.com/feature/is-distinct-from
