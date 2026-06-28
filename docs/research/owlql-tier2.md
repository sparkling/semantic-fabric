# Native-Rust OWL 2 QL tier-2 reasoning

**Date:** 2026-06-27 · **Decision:** ADR-0008 (tier-2 evidence-gated, deferred). **Evidence:** [A]/[B]/[C].

## Question
What does deferred **tier-2** (full DL-Lite_R: RHS-existential / tree-witness rewriting) add beyond ADR-0008's **tier-1** (hierarchy UNION expansion), and what is the SOTA build path if it is ever needed?

## Findings
- **Ontology-depth framing** (Bienvenu et al.): **depth-0** = no RHS existential (only hierarchy + domain/range `∃R⊑A` + disjointness) = exactly tier-1's FO-rewritable territory; **depth-1** = bounded RHS existentials (polynomial-size nonrecursive-Datalog rewriting); **depth-2+** = exponential blow-up. [A]
- **Tier-2 adds only answering over anonymous individuals** (`C ⊑ ∃R.D` queried via an existential join variable) — rare in OBDA over relatively-complete operational data. [A/C]
- **Cheap depth-0 gaps that are NOT tier-2** (close first if missing): `∃R⊑A` domain/range propagation; `⊥`-consistency/disjointness. [A]
- **No native-Rust OWL-QL OBDA rewriter exists.** `horned-owl` (v1.0.0) parses all OWL2 but is parse-only; `reasonable` = OWL-RL; `whelk-rs` = OWL-EL. [A]
- **DL-Lite_R → linear TGDs** is a standard, low-risk compilation; **Nemo** (TU Dresden, Rust, KR 2024) runs the restricted chase but is a *materialization* engine (architectural tension with the virtual path), has **no termination guarantee** (correct only for bounded-depth), and needs a depth guard. [A]
- A full tree-witness rewriter is 3–6 months + high correctness risk (re-treads Ontop). [C]

## Decision (ADR-0008)
**Keep tier-2 deferred but evidence-gated.** Highest-leverage check (days): grep the consumed T-Box for `ObjectSomeValuesFrom` in superclass position — **none → depth-0 → tier-1 provably complete → close tier-2 permanently.** If a real query is shown (Ontop offline oracle) to miss certain answers: grow the virtual rewriter to depth-1 first; for full DL-Lite_R on bounded-depth, compile to TGDs + Nemo as a materialize-mode tier. Never hand-roll tree-witness; Ontop (JVM) stays an offline oracle only.

## Sources
- Bienvenu et al., Complexity of OBDA with OWL 2 QL & bounded-treewidth queries: https://arxiv.org/pdf/1702.03358 · Optimal Datalog rewritings: https://arxiv.org/abs/1604.05258
- Kontchakov et al., combined approach (KR 2010 / ISWC 2013): https://aaai.org/papers/31-1282-the-combined-approach-to-query-answering-in-dl-lite/ · https://link.springer.com/chapter/10.1007/978-3-642-41335-3_20
- Nemo (KR 2024): https://proceedings.kr.org/2024/70/kr2024-0070-ivliev-et-al.pdf · https://github.com/knowsys/nemo · DL-Lite as linear TGDs (Datalog±): https://openproceedings.org/2009/conf/icdt/CaliGL09.pdf
- horned-owl (TGDK 2024): https://drops.dagstuhl.de/storage/08tgdk/tgdk-vol002/tgdk-vol002-issue002/html/TGDK.2.2.9/TGDK.2.2.9.html · W3C OWL 2 Profiles: https://www.w3.org/TR/owl2-profiles/ · Ontop releases: https://ontop-vkg.org/guide/releases.html
