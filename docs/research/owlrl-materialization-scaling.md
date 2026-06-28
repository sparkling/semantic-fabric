# SOTA — scalable & incremental materialization of a safe OWL-RL rule subset in Rust

**Research key:** `owlrl-materialization-scaling`
**Date:** 2026-06-27 (round-2 deep-research)
**Scope:** the optional safe OWL 2 RL rule-subset closure over the materialized A-Box — a SMALL fixed rule set (rdfs5/7/9/11, owl:inverseOf, owl:TransitiveProperty, owl:SymmetricProperty, + owl:disjointWith consistency check; ≈7–8 rules), excluding sameAs/Functional/IFP/domain-range/equivalentClass. Hard constraint: Rust-native, in-process, **no JVM**.
**Decision recorded in:** ADR-0008 (decision update 2026-06-27).

## Bottom line

1. **Quick-start: `reasonable`** (OWL-RL on `datafrog`; v0.4.x active). It already ships **additions-only incremental** (`update_graph()`); the safe rule subset's behaviour is "free" because the excluded blow-up rules stay dormant when their premises (sameAs/domain-range/equivalentClass triples) are absent. Lowest-risk path to a working closure.
2. **Controlled core: hand-roll semi-naive over `datafrog` (or Ascent)** for the ~7 rules — with so few rules, hand-rolling **beats** a general OWL-RL library on memory + control: dictionary-encode terms to `u32` (reuse Oxigraph term IDs), instantiate only the 6–7 indices the rules need, skip all dormant-rule machinery.
3. **The only real blow-up = `owl:TransitiveProperty` over the A-Box** (O(N²)); rdfs5/rdfs11 transitivity run over the *small* T-Box and are cheap. Mitigate via Ascent BYODS `trrel_uf` (labeled union-find, O(N) per SCC) **or — chosen (ADR-0008) — don't materialise it**: serve transitivity at query time via `P+`/`P*` against Oxigraph (which evaluates property paths), opt-in `--materialize-transitive` for Fuseki parity.
4. **Incremental scaling path: `differential-dataflow`** (Rust, in-proc; the engine behind FlowLog/DDlog) — true add **and** delete maintenance, ~25× faster than full re-materialization for OWL-RL, ~2–3× memory tax. Flag, not v1.
5. `reasonable` = quick start; hand-roll = controlled core; differential-dataflow = incremental path; **Nemo / RDFox / Soufflé = reference only** (Nemo: no incremental + heavyweight; RDFox: commercial/C++; Soufflé: C++).

## Rust engine options

- **`reasonable`** (gtfierro; BSD-3; ~130K downloads; active to 2026-05): OWL-2-RL Datalog rules as a `datafrog` semi-naive fixpoint; **full working set in RAM** (no streaming/out-of-core); additions-only incremental shipped; single-threaded; ~7× faster than Allegro / 38× than Python OWLRL.
- **`datafrog`** (rust-lang): the borrow-checker's engine — `Relation` = sorted `Vec`; `Variable` = stable/recent/to_add semi-naive; in-memory, batch, no incremental-across-updates. Minimal deps, max control, the exact engine `reasonable` validates.
- **Ascent** (s-arash; OOPSLA 2023): `ascent!` macro; semi-naive; **lattices**; **rayon parallelism** (`ascent_par!`); **BYODS** — `ascent-byods-rels` provides `eqrel` (equivalence) and `trrel_uf` (transitive, O(N)/SCC, semi-naive-preserving). Best for the transitive rule if materialised.
- **`crepe`** (compile-time macro; semi-naive + stratified negation; quieter) — weakest of the three here.
- **`Nemo`** (TU Dresden, KR2024): in-memory columnar (VLog lineage) + leapfrog-triejoin; existential rules (restricted chase); excellent scale (LUBM-1k 186.7M facts/163 s) but **no incremental**, in-memory only, heavier dep + unused existential machinery → benchmark oracle.

## Memory & incrementality

- **Memory-bounded?** Not inherently — every in-process Rust option holds the full closure in RAM, and **no mature out-of-core Rust Datalog exists**. Make it bounded-in-practice via (1) integer dictionary encoding (RDFox-style 37–101 B/triple), (2) **not materialising `owl:TransitiveProperty`**, (3) specialised TC structures (`trrel_uf`). The safe rule subset's exclusions already remove the worst offenders → realistic closures are small.
- **Incremental?** Batch engines (datafrog/Ascent/crepe/Nemo) are monotone/additive (re-materialise on deletion). True add+delete IVM = `differential-dataflow`/`timely` (arrangements; recursion via `iterate`) or DBSP/Feldera (Z-sets; can GC streaming state). FlowLog (VLDB 2026, Rust on DD) quantifies the incrementality "memory tax" ≈ 2–3× over batch. **RDFox B/F (Backward/Forward)** is the algorithmic gold standard for deletion maintenance (orders of magnitude better than DRed) if ever hand-implemented.

**Ladder:** (1) ship `reasonable.update_graph()` or re-run-on-change; (2) if deletions/ontology-edits get hot, move the closure onto `differential-dataflow`; (3) keep RDFox B/F as the reference for hand-implemented deletion maintenance.

## Reference systems (informative, not adoptable)
**RDFox** (commercial/C++): in-RAM dictionary-encoded, lock-free parallel materialization (13.9× on 16 cores), incremental DRed+B/F, modular materialization — steal the *techniques*. **Soufflé** (C++): compiled Datalog, Brie trie + concurrent B-tree with traversal "hints". **VLog**: the columnar memory-efficiency reference Nemo descends from.

## Integration note
Oxigraph does **no** OWL-RL reasoning — the closure is produced by a separate component (`reasonable`/hand-roll/DD) and the derived triples inserted back into the served store. Reuse Oxigraph's term IDs as the dictionary to avoid a second encoding pass.

## Evidence grades
- reasonable basis/incremental; datafrog model; Ascent parallel/BYODS/`trrel_uf` complexity; Nemo storage/scale/no-incremental; FlowLog memory+speed; RDFox B/F-vs-DRed; TC quadratic blow-up — **High**.
- exact 25× DD-vs-remat — **Med** (single workshop paper; an MDPI sibling was retracted — corroborated directionally by FlowLog/DBSP); `trrel_uf` soundness on arbitrary directed TransitiveProperty — **Med** (validate per ontology).
- no mature out-of-core Rust Datalog — **Low** (absence of evidence).

## Sources
- https://github.com/gtfierro/reasonable · https://docs.rs/reasonable · https://github.com/rust-lang/datafrog · https://github.com/s-arash/ascent · https://kmicinski.com/assets/byods.pdf (BYODS, OOPSLA 2023)
- https://proceedings.kr.org/2024/70/kr2024-0070-ivliev-et-al.pdf (Nemo) · https://github.com/knowsys/nemo
- https://dl.acm.org/doi/fullHtml/10.1145/3639592.3639622 (differential dataflow for Datalog) · https://www.vldb.org/pvldb/vol19/p361-zhao.pdf (FlowLog) · https://www.vldb.org/pvldb/vol16/p1601-budiu.pdf (DBSP)
- https://www.cs.ox.ac.uk/boris.motik/pubs/mnph15incremental-BF.pdf (RDFox B/F) · https://ojs.aaai.org/index.php/AAAI/article/view/8730 (RDFox parallel) · https://souffle-lang.github.io/pdf/pmam19.pdf (Soufflé)
- https://www.w3.org/TR/owl2-profiles/ (OWL 2 RL)
