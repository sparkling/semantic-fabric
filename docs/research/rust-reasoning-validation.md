# Rust crates for RDF validation + reasoning

**Research round:** rust-reasoning-validation  
**Date:** 2026-06-26  
**Covers:** ADR-0005 (SHACL runner for `M ⋈ T` gate) and ADR-0007 (deferred OWL 2 QL entailment)

---

## 1. Motivation

Two open dependencies are blocking or shaping semantic-fabric:

* **ADR-0005** — the `M ⋈ T` mapping-output gate must run four SHACL meta-shapes (`mf:MappingClassConformanceShape`, `mf:MappingPredicateConformanceShape`, `mf:MappingDatatypeConformanceShape`, `mf:EntitySubjectGroundingShape`) from the upstream modelling project's mapping-conformance requirements. ADR-0005 §outcome explicitly flags the Rust SHACL engine choice as an _open dependency_.
* **ADR-0007** — OWL 2 QL entailment / tree-witness rewriting is deferred to v2 ("501 Not Implemented"). The decision to defer is final for v1, but the right Rust crate to eventually fill this slot is still unmapped.

This report surveys the candidates.

---

## 2. SHACL / ShEx engines

### 2.1 rudof (primary candidate for ADR-0005)

| Attribute | Value |
|-----------|-------|
| Crate(s) | `rudof_lib`, `shacl_ast`, `shacl_validation`, `shacl_ir`, `shacl_testsuite` |
| Repo | [github.com/rudof-project/rudof](https://github.com/rudof-project/rudof) |
| Latest | v0.3.3 (2026-06-05) |
| License | Apache-2.0 / MIT |
| Maintainer | José E. Labra Gayo (WESO, University of Oviedo) |
| Covers | SHACL Core, ShEx, DCTAP, format conversions |
| Backends | File-based RDF (Turtle/N-Triples/N-Quads) and SPARQL endpoint via `srdf` abstraction |

rudof is the most mature pure-Rust SHACL implementation ([ISWC 2024 demo paper](https://ceur-ws.org/Vol-3828/paper32.pdf)). Its internal architecture separates `shacl_ast` (SHACL abstract syntax tree), `shacl_ir` (intermediate representation), and `shacl_validation` (the validation engine), all surfaced through the `Rudof` façade in `rudof_lib`.

**RDF backend — Oxigraph alignment.** rudof's `srdf` abstraction has two concrete implementations: one backed by in-memory RDF datasets (using oxrdf types internally) and one backed by SPARQL. Because semantic-fabric already depends on the Oxigraph family (`oxrdf`, `oxttl`, `spargebra`, `oxigraph`), rudof's Oxigraph-backed srdf implementation slots in without pulling a second RDF stack.

**SHACL Core coverage.** rudof supports the constraint components that matter for the four `M ⋈ T` shapes:
- Node shapes (`sh:NodeShape`), property shapes (`sh:property`)
- `sh:class`, `sh:datatype`, `sh:nodeKind`
- Cardinality (`sh:minCount`, `sh:maxCount`)
- Value constraints (`sh:in`, `sh:pattern`, `sh:hasValue`)
- Shape references (`sh:node`, `sh:qualifiedValueShape`)
- Property paths (sequence, alternative, inverse)
- Logical operators (`sh:and`, `sh:or`, `sh:not`)

The `shacl_testsuite` crate runs the W3C SHACL test suite; compliance is not yet formally submitted to the [W3C implementation report](https://w3c.github.io/data-shapes/data-shapes-test-suite/), but the test runner exists and is integrated.

**SHACL Advanced / SPARQL constraints.** `sh:SPARQLConstraint` and SHACL-SPARQL targets are not fully implemented (rudof's stated scope is SHACL Core + ShEx). The four `M ⋈ T` meta-shapes from the upstream modelling project rely on SHACL Core property and node constraints only (class, predicate, datatype, nodeKind checks), so this gap is not blocking.

**MCP server.** [`rudof_mcp`](https://docs.rs/crate/rudof_mcp/latest) (v0.3.1) exposes rudof via MCP — an integration bonus for tooling, not required for semantic-fabric's programmatic use.

**Python bindings.** `pyrudof` (v0.3.3, released 2026-06-24 per the search) confirms active maintenance.

**Verdict on ADR-0005:** rudof **resolves the open dependency**. The `shacl_validation` crate's programmatic API, backed by the Oxigraph-compatible srdf, can run the four `M ⋈ T` shapes against either materialized N-Quads output or the `M ⋈ T` closure graph. No shelling out required. Evidence quality: high (primary crate docs + ISWC demo paper + confirmed active release cycle).

### 2.2 oxirs-shacl (secondary / early-stage)

| Attribute | Value |
|-----------|-------|
| Crate | `oxirs-shacl` |
| Repo | [github.com/cool-japan/oxirs](https://github.com/cool-japan/oxirs) |
| License | not confirmed |
| Status | Early development; API not stabilised |

[oxirs-shacl](https://crates.io/crates/oxirs-shacl) is part of the OxiRS platform (a broader Semantic Web framework aiming to cover SPARQL 1.2, GraphQL, and AI reasoning). It targets SHACL Core + SHACL-SPARQL and sits directly on the Oxigraph stack, which would make it a natural fit for semantic-fabric. However, the public API is explicitly described as being finalized, and W3C test suite compliance is listed as a development _goal_, not a current state. Do not rely on this for the ADR-0005 hook; revisit in 6–12 months.

### 2.3 Comparison: SHACL candidates

| | rudof 0.3.3 | oxirs-shacl (early) |
|---|---|---|
| SHACL Core | Yes (Core constraints) | Claimed, API unstable |
| SHACL Advanced / SPARQL | Partial / not complete | Claimed |
| Oxigraph backend | Yes (via srdf/oxrdf) | Yes (native) |
| W3C test suite | Runner exists | Goal only |
| License | Apache-2.0 / MIT | Unknown |
| Production readiness | Yes (v0.3.3, ISWC paper) | No |
| ADR-0005 fit | **Resolves gap** | Deferred |

---

## 3. OWL reasoning

### 3.1 horned-owl — OWL 2 parser / data model

| Attribute | Value |
|-----------|-------|
| Crate | `horned-owl` |
| Repo | [github.com/phillord/horned-owl](https://github.com/phillord/horned-owl) |
| Latest | v1.4.0 (2026-01-09) |
| License | MIT / Apache-2.0 |
| Maintainer | Phillip Lord (Newcastle University) |
| Covers | OWL 2 parsing, serialization, SWRL; NOT a reasoner |

horned-owl is the reference Rust library for OWL 2 file I/O ([TGDK 2025 paper](https://drops.dagstuhl.de/entities/document/10.4230/TGDK.2.2.9)). It reads and writes RDF/XML, OWL Functional Syntax, Manchester syntax, and related formats, and represents the complete OWL 2 data model including SWRL rules. It is deliberately scope-limited to parsing and manipulation — it performs no inference.

**OWL 2 QL profile.** horned-owl does not contain a profile validator or classifier for OWL 2 QL / EL / RL as of v1.4.0 (unlike the Java OWL API which does). It cannot answer SPARQL queries under the OWL 2 QL entailment regime.

**ADR-0007 relevance.** horned-owl is the right crate to **load and inspect an OWL ontology** (e.g., to extract axioms for transformation into Datalog rules). Any future OWL 2 QL reasoning pipeline in Rust would need horned-owl as the I/O layer. It does not replace the missing reasoner.

### 3.2 whelk-rs — OWL EL reasoner (experimental Rust port)

| Attribute | Value |
|-----------|-------|
| Repo | [github.com/INCATools/whelk-rs](https://github.com/INCATools/whelk-rs) |
| Crates.io | Not published (GitHub only) |
| Status | Experimental |
| Profile | OWL 2 **EL** only |
| Upstream | Scala original Whelk v1.2.1 ([TGDK 2025 paper](https://drops.dagstuhl.de/entities/document/10.4230/TGDK.2.2.7)) |

whelk-rs is an experimental Rust port of the Whelk OWL EL+RL reasoner from INCATools. The original Scala Whelk supports complex class expression queries and incremental reasoning, but whelk-rs lags well behind and is not published to crates.io. OWL 2 EL is a strictly weaker profile than OWL 2 QL: EL supports existential restrictions but not inverse properties or number restrictions; QL (= DL-Lite_R) supports inverse properties and number restrictions but uses open-world OBDA semantics. The two profiles are not interchangeable for SPARQL entailment over relational data, which is what ADR-0007 needs.

**ADR-0007 relevance:** does not resolve the gap.

### 3.3 reasonable — OWL 2 RL reasoner

| Attribute | Value |
|-----------|-------|
| Crate | `reasonable` |
| Repo | [github.com/gtfierro/reasonable](https://github.com/gtfierro/reasonable) |
| Latest | v0.4.1 |
| License | BSD-3-Clause |
| Profile | OWL 2 **RL** |
| Engine | [DataFrog](https://github.com/frankmcsherry/datafrog) Datalog |

[reasonable](https://crates.io/crates/reasonable) implements the complete OWL 2 RL rule set (W3C-defined Datalog rules) and reports violations as structured diagnostics. Benchmarks show ~7× faster than Allegro and ~38× faster than OWLRL Python on Brick ontology workloads. It offers a Rust API, CLI, and Python bindings.

**Profile mismatch with ADR-0007.** OWL 2 RL and OWL 2 QL are different profiles with different semantics. RL uses closed-world semantics and is optimal for materializing inferences over large ABoxes; QL uses open-world (OBDA) semantics and is designed for query rewriting over relational data. Ontop's tree-witness rewriting and tautology elimination work under OWL 2 QL / DL-Lite_R, not RL. `reasonable` cannot fill the ADR-0007 slot.

**Possible auxiliary use.** `reasonable` could be useful for materializing OWL 2 RL closure over the `M ⋈ T` output graph as an additional conformance check (e.g., detecting owl:sameAs, rdfs:subClassOf violations), but this is not what ADR-0007 specifies.

### 3.4 OWL 2 QL / DL-Lite gap assessment

No Rust crate implements OWL 2 QL entailment or tree-witness SPARQL rewriting as of June 2026. The options for ADR-0007's deferred path are:

1. **Subprocess call to Ontop** — already the differential oracle in ADR-0005; reusing it for QL entailment is architecturally consistent.
2. **Nemo as OWL 2 QL rewriter** — see §4.
3. **New Rust implementation** — the theoretical path exists (DL-Lite_R → Datalog + existential rules → any Datalog engine), but this is research-level work, not library reuse.

---

## 4. Datalog / rules — Nemo (TU Dresden)

| Attribute | Value |
|-----------|-------|
| Repo | [github.com/knowsys/nemo](https://github.com/knowsys/nemo) |
| Crates.io | `nemo` (library crate, alongside `nemo-cli`) |
| License | Apache-2.0 |
| Maintainer | ICCL / Knowsys group, TU Dresden (Markus Krötzsch et al.) |
| Language | Rust |

Nemo is a pure-Rust in-memory rule engine for large-scale Datalog reasoning ([KR 2024 paper](https://proceedings.kr.org/2024/70/kr2024-0070-ivliev-et-al.pdf)). Its rule language supports:

- Stratified negation
- Arithmetic and comparison functions
- Aggregates
- **Existential rules** (tuple-generating dependencies / TGDs) — this is the key capability for OWL 2 QL

**Input formats:** CSV/TSV, N-Triples, Turtle, RDF/XML, N-Quads, TriG, SPARQL endpoints.

**Relevance to ADR-0007.** OWL 2 QL (= DL-Lite_R) axioms can be compiled to existential Datalog rules — this is the theoretical basis of all DL-Lite OBDA systems. Nemo's TGD support makes it a viable *rule executor* in a future OWL 2 QL pipeline where:
1. horned-owl parses the OWL ontology,
2. a custom Rust transform compiles DL-Lite axioms to Nemo rules,
3. Nemo executes query rewriting/materialisation.

A 2025 ESWC paper from the same group covers "SPARQLing Datalog for Rule-Based Reasoning over Large Knowledge Graphs" — the direct technical bridge. Nemo is not a drop-in OWL 2 QL engine, but it is the best Rust substrate for building one. Evidence quality: medium (the OWL→Nemo compilation layer does not exist as a library; the theoretical path is well-established but the implementation gap is real).

---

## 5. Summary comparison table

| Crate | Domain | Profile/scope | License | Maturity | ADR fit |
|-------|--------|--------------|---------|----------|---------|
| `rudof` / `shacl_validation` | SHACL + ShEx | SHACL Core | Apache/MIT | High (v0.3.3) | **Resolves ADR-0005** |
| `oxirs-shacl` | SHACL + SPARQL | SHACL Core + Advanced | TBD | Low (unstable API) | Revisit 2027 |
| `horned-owl` | OWL parsing | OWL 2 Full (parser only) | MIT/Apache | High (v1.4.0) | ADR-0007 I/O layer |
| `whelk-rs` | OWL reasoning | OWL 2 EL | BSD-3 | Low (experimental) | Wrong profile |
| `reasonable` | OWL reasoning | OWL 2 RL | BSD-3 | Medium (v0.4.1) | Wrong profile |
| `nemo` | Datalog + TGDs | Existential Datalog | Apache-2.0 | Medium (active research) | ADR-0007 future substrate |

---

## 6. Direct answers to the two open ADR questions

**Does rudof resolve ADR-0005's SHACL-runner gap?**

**Yes.** rudof 0.3.3 (Apache/MIT, active) provides a programmatic Rust API (`shacl_validation::validate`) that covers all SHACL Core constraints present in the four `M ⋈ T` meta-shapes (class, predicate, datatype, nodeKind, cardinality, node references). Its `srdf` abstraction integrates with Oxigraph-backed graphs — the same RDF stack semantic-fabric already depends on. No subprocess shelling is needed. The `shacl_testsuite` crate (W3C test runner) can be added to CI alongside the existing W3C RDB2RDF suite. The only remaining engineering step is wiring `shacl_validation` into the `sf-conformance` hook that ADR-0005 scaffolded.

**Do horned-owl and whelk-rs inform the deferred OWL 2 QL path (ADR-0007)?**

**Partially.** horned-owl (v1.4.0, MIT/Apache) is the correct Rust crate for parsing OWL 2 ontologies and will be the I/O layer in any future OWL 2 QL pipeline. whelk-rs covers only OWL 2 EL and is not production-ready in Rust — it does not help. reasonable covers OWL 2 RL (closed-world), a different profile from OWL 2 QL (open-world OBDA). **No Rust crate implements OWL 2 QL reasoning as of June 2026.** The most viable future path is: horned-owl (parse) + a custom DL-Lite_R → existential-rules compiler + Nemo (execute), with Ontop remaining the subprocess fallback for correctness cross-checking. The ADR-0007 deferral is sound given the current ecosystem state.

---

## 7. Integration sketch for rudof → ADR-0005

```toml
# sf-conformance/Cargo.toml
[dependencies]
rudof_lib    = "0.3"
oxigraph     = { workspace = true }   # already in the graph
spargebra    = { workspace = true }
```

```rust
// sf-conformance/src/m_join_t.rs
use rudof_lib::{Rudof, RudofConfig};
use rudof_lib::shacl::ShaclValidation;

pub fn validate_m_join_t(graph: &oxigraph::store::Store, shapes_ttl: &str) -> ValidationReport {
    let mut rudof = Rudof::new(&RudofConfig::default());
    rudof.read_shacl(shapes_ttl, …)?;
    rudof.read_data_from_store(graph)?;
    rudof.validate_shacl()?
}
```

The shape IRI set (`mf:MappingClassConformanceShape` et al.) lives in the upstream modelling project's `validation-constraints-meta-shapes.ttl` — load it as a static file or embed it at build time.

---

## 8. Sources

* [rudof overview](https://rudof-project.github.io/rudof/) — project documentation
* [rudof on crates.io](https://crates.io/crates/rudof) — v0.3.3, 2026-06-05
* [shacl_validation crate](https://crates.io/crates/shacl_validation) — SHACL Core validation engine
* [shacl_ast crate](https://crates.io/crates/shacl_ast) — SHACL abstract syntax tree
* [rudof ISWC 2024 demo paper](https://ceur-ws.org/Vol-3828/paper32.pdf) — CEUR Vol-3828, paper32
* [rudof architecture](https://rudof-project.github.io/rudof/internals/architecture.html) — srdf abstraction + backends
* [rudof_lib docs.rs](https://docs.rs/crate/rudof_lib/latest) — API reference
* [oxirs-shacl crate](https://crates.io/crates/oxirs-shacl) — early-stage alternative
* [OxiRS GitHub](https://github.com/cool-japan/oxirs) — parent project
* [horned-owl on crates.io](https://crates.io/crates/horned-owl) — v1.4.0
* [horned-owl GitHub](https://github.com/phillord/horned-owl) — Phillip Lord, Newcastle University
* [Horned-OWL TGDK 2025 paper](https://drops.dagstuhl.de/entities/document/10.4230/TGDK.2.2.9) — "Flying Further and Faster with Ontologies"
* [horned-owl docs.rs](https://docs.rs/horned-owl) — API reference
* [whelk-rs GitHub](https://github.com/INCATools/whelk-rs) — OWL EL experimental Rust port
* [Whelk TGDK 2025 paper](https://drops.dagstuhl.de/entities/document/10.4230/TGDK.2.2.7) — Whelk OWL EL+RL
* [reasonable on crates.io](https://crates.io/crates/reasonable) — v0.4.1, BSD-3
* [reasonable GitHub](https://github.com/gtfierro/reasonable) — OWL 2 RL on DataFrog
* [Nemo GitHub](https://github.com/knowsys/nemo) — TU Dresden Knowsys group
* [Nemo KR 2024 paper](https://proceedings.kr.org/2024/70/kr2024-0070-ivliev-et-al.pdf) — rule reasoning toolkit
* [W3C SHACL test suite](https://w3c.github.io/data-shapes/data-shapes-test-suite/) — implementation report
* [W3C OWL 2 Profiles](https://www.w3.org/TR/owl2-profiles/) — QL / EL / RL definitions
