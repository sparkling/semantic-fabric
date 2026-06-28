---
status: accepted
date: 2026-06-27
tags: [reasoning, entailment, owl2-ql, query-rewriting, t-saturation, transitive, ontology-depth, virtualization]
supersedes: []
depends-on:
  - ADR-0002
  - ADR-0003
  - ADR-0007
implements:
  - ADR-0001
---

# Reasoning strategy — entailment folded into the rewrite, native Rust, no runtime JVM

## Context and Problem Statement

A fabric that lets you query the *ontology* (T), not raw tables, needs some entailment — otherwise a query for a class returns only rows mapped to exactly that class and silently misses subclasses. The engine virtualises (ADR-0003), so all entailment is **folded into the SPARQL→SQL rewrite** at query time; nothing is materialised. The fabric does **not** classify T (computing T's hierarchy is upstream build-time DL work owned by the upstream reasoner); it **consumes** a pre-classified T (held in memory, ADR-0004) and uses its hierarchy to saturate queries.

**Runtime invariant:** all reasoning is native Rust, in-process, at query time — no JVM, no external reasoner, no materialised closure.

## Decision Outcome

### Tier-1 (hierarchy) — folded into the rewrite, live
Subclass/subproperty + `rdf:type` entailment is folded into the mappings at startup via **T-mapping saturation** (each class absorbs its subclasses as a UNION; each property its subproperties); `owl:inverseOf` and `owl:SymmetricProperty` fold into the rewriter; `owl:disjointWith` is a consistency check (run via the SHACL/`rudof` gate, ADR-0005). This is transitive closure over T's already-built hierarchy edges + UNION expansion — **no reasoner crate** — and the ADR-0007 cascade prunes the UNIONs back to near-native SQL. Domain/range is documentation-only (not inferred). `owl:TransitiveProperty` is not FO-rewritable, so it is served live as a `P+`/`P*` property path (recursive CTE; ADR-0007), not by tier-1 rewriting.

This native query-time lane replaces the host platform's Jena Fuseki `GenericRuleReasoner` + a safe OWL-RL rule subset (authored in the upstream modelling project): the rule subset's enabled constructs are exactly subclass/subproperty (+ transitivity), `owl:inverseOf`, `owl:TransitiveProperty`, `owl:SymmetricProperty`, and the `owl:disjointWith` check — all covered by the rewrite (subclass/subproperty/inverse/symmetric by UNION-folding; transitive by recursive CTE). **No A-Box closure is computed or stored.**

### Tier-2 (full OWL 2 QL: RHS-existential / tree-witness) — evidence-gated, deferred
Tier-2 adds only **answering over anonymous individuals** (`C ⊑ ∃R.D` queried through an existential join variable) — rare in OBDA over relatively-complete operational data, and the source of exponential rewriting blow-up once ontology depth ≥ 2. Gate it on **ontology depth**:

* **Depth-0** (T asserts no right-hand-side existential — no `ObjectSomeValuesFrom` in superclass position): tier-1 is **provably complete → tier-2 is closed**. The platform's OWL-as-documentation policy excludes `owl:Restriction`/existentials, so the generated T-Box is expected **depth-0 by construction**.
* **If a real query is shown to miss certain answers** (via the Ontop offline oracle, ADR-0005): grow the virtual rewriter to **depth-1** (polynomial-size nonrecursive-Datalog rewriting; stays virtual) first.
* **Do not hand-roll a full tree-witness rewriter** (re-treads 15 years of Ontop). `horned-owl` parses OWL 2 QL; Ontop (JVM) is the offline differential oracle only — never a serving dependency.

## Consequences
* Good — superclass queries are complete (tier-1) without tree-witness cost; the JVM is gone from the reasoning path; nothing is materialised, so there is no closure-staleness concern.
* Good — tier-2 is a measured, evidence-gated decision, not speculative capability.
* Neutral — the fabric never invokes a TBox classifier; it loads a pre-classified T (boundary check).

## Confirmation
* T-mapping saturation appears as a documented stage in the rewriter (`sf-sparql`).
* The fabric loads a pre-classified T and never classifies it.
* Tier-2 stays absent until the depth check / oracle proves it necessary.

## More Information
* **Architecture:** ADR-0003. **Rewriting (UNION-folding, recursive CTE):** ADR-0007. **Scope:** ADR-0002. **Substrate (T held in memory):** ADR-0004. **Conformance / oracle:** ADR-0005.
* **Cross-project (authoritative):** the upstream reasoning policy + a safe OWL-RL rule subset and the OWL-as-documentation policy (authored in the upstream modelling project), which this native query-time lane replaces.
* **Research:** `docs/research/` — `owlql-tier2`, `rust-reasoning-validation`.
</content>
