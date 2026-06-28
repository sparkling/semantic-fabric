# Rust SPARQL Engines, Graph Databases, and Linked-Data Applications

**Topic:** Rust engine/app layer — competitive landscape and reuse candidates for semantic-fabric  
**Date:** 2026-06-26  
**Evidence grade:** High — primary sources (GitHub, docs.rs, crates.io, IEEE paper)

---

## Executive Summary

The Rust linked-data ecosystem has matured substantially but remains smaller than the Java/Python incumbents. **Oxigraph** (v0.5.7) is the anchor: a near-conformant SPARQL 1.1/1.2 store whose component crates (`spargebra`, `sparopt`, `oxrdf`, `oxttl`) semantic-fabric already pins as its core substrate. A research fork, **rdf-fusion**, demonstrates columnar SPARQL over Apache DataFusion but is explicitly experimental and missing persistence. **Grafeo** is the only other Rust-native multi-query-language graph database with full SHACL support. **terminusdb-store** provides Rust triple-storage but its product surface uses WOQL, not SPARQL. On the conformance-tooling side, two credible **Rust SHACL engines** resolve ADR-0005's open dependency: `rudof` (v0.3.4, June 2026, full SHACL core + ShEx) and `oxirs-shacl` (v0.3.1, 27/27 W3C constraint types + experimental SHACL-SPARQL). For OWL 2 reasoning (ADR-0007), `reasonable` implements OWL 2 **RL** efficiently but no Rust-native **OWL 2 QL** engine exists — the ADR-0007 deferral stands. The prior finding that **no Rust-native R2RML or RML mapping engine exists** is confirmed; semantic-fabric is greenfield in this niche.

---

## 1. RDF4J — Java Prior Art

[Eclipse RDF4J](https://rdf4j.org/) is the comprehensive Java framework that underpins Ontop. It provides the Repository API (central SPARQL endpoint abstraction), Rio (RDF parsers/serializers), an LMDB-backed store, SAIL (Storage and Inference Layer) extensions for SHACL and Lucene full-text, and FedX for federated SPARQL. Ontop integrates as an RDF4J SAIL: it intercepts SPARQL queries, rewrites them to SQL via the OWL 2 QL unfolding algorithm, executes against the RDBMS, and returns results through RDF4J's result-set API. RDF4J thus provides Ontop with the SPARQL protocol layer, result serialization, and connection pooling that semantic-fabric must build from scratch in Rust. There is no Rust equivalent of the full RDF4J framework scope; semantic-fabric reproduces this role using Oxigraph component crates (`spargebra` for parsing, `sparesults` for wire formats, `oxrdf` for the data model).

**Role:** Java prior art / architectural reference for the SPARQL-endpoint + OBDA integration layer. Not a reuse candidate.

---

## 2. Oxigraph (Recap)

[Oxigraph](https://github.com/oxigraph/oxigraph) (v0.5.7, April 2026) is the most mature Rust SPARQL store. It supports SPARQL 1.1 with near-full conformance and RDF 1.2 / SPARQL 1.2 behind `rdf-12` / `sparql-12` feature flags. License: MIT/Apache-2.0. The prior research note (`oxigraph.md`) documents the precise crate-split semantic-fabric uses — consuming `spargebra`, `sparopt`, `oxrdf`, `oxttl`, `oxrdfio`, and `sparesults` without pulling in the full RocksDB triplestore.

**Role for semantic-fabric:** Core substrate (already confirmed). The full `oxigraph` store crate is not needed; only the component crates.

---

## 3. rdf-fusion — Columnar SPARQL over DataFusion

[rdf-fusion](https://github.com/tobixdev/rdf-fusion) ([crates.io](https://crates.io/crates/rdf-fusion)) is an embeddable SPARQL engine forked from Oxigraph and rebuilt on Apache DataFusion's vectorised execution engine. Published in [IEEE Access 2025](https://ieeexplore.ieee.org/document/11208525/) (*RDF Fusion: An Extensible SPARQL Engine for Hybrid Data Models*), it is explicitly targeting IoT and time-series hybrid workloads where analytical throughput matters more than latency. License: Apache 2.0 (MIT portions from Oxigraph). Maturity: **experimental** — the README warns that APIs, encodings, and storage formats are all subject to breaking change, and it currently lacks persistent storage and full SPARQL 1.1 coverage.

| Aspect | Detail |
|--------|--------|
| Approach | Vectorised/columnar via Arrow; DataFusion logical plans |
| SPARQL coverage | Incomplete — no persistence, no RDF 1.2 |
| Publication | IEEE Access vol. 13, 2025 (Schwarzinger et al.) |
| License | Apache 2.0 |
| Maturity | Experimental; not production-ready |

**Role for semantic-fabric:** Interesting **prior art** for the SPARQL→SQL rewriting approach if extended to relational sources; rdf-fusion is built on DataFusion (note: semantic-fabric does **not** use DataFusion — dropped per ADR-0015; rdf-fusion is prior art only). Not a reuse candidate today, but the architectural direction (columnar SPARQL algebra evaluation) is directly analogous to what semantic-fabric's SPARQL→SQL path achieves. Monitor for persistence layer addition.

---

## 4. Grafeo — Pure-Rust Multi-Language Graph Database

[Grafeo](https://github.com/GrafeoDB/grafeo) (v0.5.42, May 2026) is a pure-Rust embedded/standalone graph database supporting six query languages: GQL (ISO/IEC 39075), openCypher, Gremlin, GraphQL, SPARQL 1.1, and SQL/PGQ (SQL:2023). License: Apache 2.0. It includes SHACL validation via its `grafeo-engine` crate with all 28 W3C SHACL Core constraint types plus SHACL-SPARQL. It does not document R2RML or OBDA capabilities.

| Aspect | Detail |
|--------|--------|
| Query languages | 6 (incl. SPARQL 1.1, GQL, Cypher) |
| SHACL | 28 core constraints + SHACL-SPARQL |
| OWL reasoning | Not documented |
| R2RML/OBDA | Not present |
| License | Apache 2.0 |
| Maturity | Active; 57 releases, 1 500+ commits |

**Role for semantic-fabric:** **Competitor** — especially for the SPARQL endpoint use case. Grafeo is broader in query-language scope but does not address the RDBMS-as-primary-store (OBDA/R2RML) angle. Its SHACL engine (`grafeo-engine`) is a third candidate for the ADR-0005 conformance gate, but the project's size and broad scope makes it harder to vendor as a standalone validator.

---

## 5. terminusdb-store — Rust Triple Storage

[terminusdb-store](https://github.com/terminusdb/terminusdb-store) (v0.19.2+, Apache 2.0) is the Rust storage layer for TerminusDB. It provides succinct-data-structure triple storage with Delta encoding and Tokio-async I/O. TerminusDB the product exposes WOQL (a Datalog variant), not SPARQL. DFRNT assumed stewardship in 2025. The store itself is purely a storage layer — no SPARQL parser or rewriter.

**Role for semantic-fabric:** **Complementary** — the succinct-data-structure storage idea (HDT-style immutable layers) is interesting for materialization output, but semantic-fabric's primary store is an RDBMS; the terminusdb-store design philosophy does not transfer. Not a reuse candidate.

---

## 6. sophia\_rs — RDF Toolkit

[sophia\_rs](https://github.com/pchampin/sophia_rs) is a Rust toolkit for RDF and Linked Data maintained by Pierre-Antoine Champin (W3C). It defines a generic API (`sophia_api`) for interoperable RDF implementations, plus parsers, serializers, an in-memory graph, JSON-LD, and a partial SPARQL 1.2 client (`sophia_sparql`). License: Apache 2.0 / CECILL-B dual. It has ~980 commits and 324 stars; actively maintained.

**Role for semantic-fabric:** **Complementary / watch list** — the `sophia_api` trait set is a reference for generic RDF interoperability. The partial SPARQL implementation does not compete with semantic-fabric's rewriting approach. No SHACL or OWL reasoning included.

---

## 7. Rust SHACL Engines (ADR-0005 Resolution)

ADR-0005 left the SHACL engine for the `M ⋈ T` conformance gate undecided. Two production-ready Rust crates now resolve this:

### 7a. rudof

[rudof](https://github.com/rudof-project/rudof) (v0.3.4, June 16 2026) is the most comprehensive Rust shapes library, covering SHACL Core, ShEx, and DCTAP, with conversion between formalisms. License: Apache 2.0 + MIT. It has 164 releases and 2 764 commits, and is actively maintained. Sub-crates of interest: `shacl_ast` (SHACL abstract syntax), `shacl_validation` (validator), `shex_validation` (ShEx), `shapes_converter` (cross-formalism conversion). The W3C SHACL test-suite coverage is documented per crate but not explicitly stated as 100% on the README.

### 7b. oxirs-shacl

[oxirs-shacl](https://docs.rs/oxirs-shacl) (v0.3.1, Apache 2.0) from the cool-japan organisation claims **27/27 W3C SHACL Core constraint types** plus experimental SHACL-SPARQL and SHACL Advanced Features. It is marked **Production Release** with stable public APIs and 78.66% documentation coverage.

### Comparison

| Crate | Version | W3C Core | SHACL-SPARQL | ShEx | License | Maturity |
|-------|---------|----------|--------------|------|---------|---------|
| [rudof](https://github.com/rudof-project/rudof) | 0.3.4 | Yes (full) | Not stated | Yes | Apache/MIT | Production (164 releases) |
| [oxirs-shacl](https://docs.rs/oxirs-shacl) | 0.3.1 | 27/27 explicit | Experimental | No | Apache 2.0 | Production Release |
| [shacl-rust](https://github.com/ensaremirerol/shacl-rust) | ? | Partial | No | No | ? | Early |
| grafeo-engine | in v0.5.x | 28/28 | Yes | No | Apache 2.0 | Active |

**ADR-0005 recommendation:** `oxirs-shacl` is the simplest integration — it is a focused validator, Apache 2.0, already production-labelled with explicit W3C constraint coverage. `rudof` is the better choice if semantic-fabric also needs ShEx support or shape-formalism conversion downstream. Both are viable; the ADR-0005 decision should be made based on whether ShEx conformance (beyond SHACL) is in scope for the `M ⋈ T` gate.

---

## 8. OWL 2 Reasoning in Rust (ADR-0007)

### 8a. reasonable — OWL 2 RL

[reasonable](https://github.com/gtfierro/reasonable) (v0.4.4, May 28 2026, BSD-3-Clause) implements **OWL 2 RL** rules using [DataFrog](https://github.com/rust-lang/datafrog) (a Datalog engine). It provides a library, CLI binary, and Python bindings. Performance benchmarks on Brick model data show ~7× faster than Allegro and ~38× faster than the Python `owlrl` package. It does **not** support OWL 2 QL. The rule coverage is comprehensive across RDFS, equality, property, and class axioms, with some exclusions (max-cardinality rules, equality reflexivity).

### 8b. horned-owl — OWL 2 Parser/Manipulation Library

[horned-owl](https://github.com/phillord/horned-owl) (v1.0.0, LGPL-3.0/GPL-3.0) is a Rust library for parsing and manipulating OWL 2 ontologies. It supports OWL 2 across all serialization formats (RDF/XML, OWL/XML, Functional Syntax, Manchester Syntax) and SWRL, with reported 20–40× speedup over the Java OWL API for large ontologies. It is a **parser/manipulator, not a reasoner** — it does not perform classification or entailment. Used by `whelk-rs` and `py-horned-owl`.

### 8c. whelk-rs — OWL EL Reasoner (Experimental)

[whelk-rs](https://github.com/INCATools/whelk-rs) is an experimental Rust port of the Whelk OWL EL reasoner (MIT license, 31 commits, no versioned releases). It targets biomedical ontologies and reports ~2× speedup over the Scala original. Status: experimental, not production-ready.

### 8d. OWL 2 QL Gap

**No Rust-native OWL 2 QL reasoner exists.** OWL 2 QL is the profile Ontop uses for SPARQL-to-SQL entailment (DL-Lite_R family). The incumbent Java implementations (Quest/Ontop, Mastro) remain the only complete OWL 2 QL systems. ADR-0007's deferral of OWL 2 QL reasoning is therefore correct — the Rust ecosystem does not yet offer a drop-in replacement. If semantic-fabric later adds SPARQL entailment under OWL 2 QL, it must either implement the unfolding algorithm from first principles (as Ontop does) or call out to a JVM via FFI/subprocess.

| Crate | Profile | Maturity | License | Notes |
|-------|---------|---------|---------|-------|
| [reasonable](https://github.com/gtfierro/reasonable) | OWL 2 RL | Active (v0.4.4) | BSD-3 | DataFrog Datalog; no QL |
| [whelk-rs](https://github.com/INCATools/whelk-rs) | OWL EL | Experimental | MIT | No releases |
| [horned-owl](https://github.com/phillord/horned-owl) | OWL 2 parse | Stable (v1.0.0) | LGPL/GPL | Parser only; no reasoning |
| — | OWL 2 QL | **Gap** | — | No Rust implementation |

---

## 9. R2RML / RML in Rust — Confirmation of Absence

A targeted search of crates.io (keywords: `r2rml`, `rml`, `rml-mapping`, `obda`) plus GitHub topic pages confirms:

- The `rml` crate on crates.io (last published August 2021) is an unrelated project — a Rust `rml!` macro for generating Markdown, not an RML/R2RML mapping engine.
- The `rml-core` crate is an N-Gram language model crate ("Rust Language Model"), unrelated.
- No active Rust crate implements W3C R2RML mapping execution or SPARQL-to-SQL rewriting via a mapping layer.
- All production R2RML/RML implementations remain in Java (Ontop, RML-Mapper, SDM-RDFizer via a Scala/JVM path), Python (Morph-KGC), or .NET (r2rml4net).

**The prior finding is confirmed: semantic-fabric is the only planned Rust-native R2RML engine.** This is both a greenfield opportunity and a burden — all test-suite compliance work (W3C R2RML tests, covered in `r2rml-spec-tests.md`) must be built from scratch.

---

## 10. Notable Complementary Rust LD Projects

| Project | Role | License | URL |
|---------|------|---------|-----|
| [sophia_rs](https://github.com/pchampin/sophia_rs) | Generic RDF API + partial SPARQL | Apache/CECILL-B | RDF interop traits |
| [manas](https://github.com/manomayam/manas) | Solid Protocol server (alpha) | ? | LD application layer |
| [terminusdb-store](https://github.com/terminusdb/terminusdb-store) | Succinct triple store layer | Apache 2.0 | Storage reference |
| [nanopub](https://crates.io/crates/nanopub) | Nanopublication toolkit (via sophia) | Apache 2.0 | LD application |

---

## 11. Competitive Landscape Summary

| Project | Language | R2RML/OBDA | SPARQL | SHACL | OWL | Role vs semantic-fabric |
|---------|----------|-----------|--------|-------|-----|------------------------|
| Ontop | Java/RDF4J | Yes (OWL2QL) | Full | No | 2 QL | Primary prior art |
| Oxigraph | Rust | No | 1.1+1.2 | No | No | Core substrate |
| rdf-fusion | Rust | No | Partial | No | No | Prior art / watch |
| Grafeo | Rust | No | 1.1 | Yes (28) | No | Competitor (endpoint) |
| terminusdb-store | Rust | No | No (WOQL) | No | No | Complementary |
| semantic-fabric | Rust | **Yes (target)** | via rewrite | via rudof/oxirs | deferred | — |

---

## Sources

- [GitHub — oxigraph/oxigraph](https://github.com/oxigraph/oxigraph) — Oxigraph SPARQL store
- [crates.io — oxigraph](https://crates.io/crates/oxigraph) — v0.5.7 release info
- [GitHub — tobixdev/rdf-fusion](https://github.com/tobixdev/rdf-fusion) — rdf-fusion repo
- [crates.io — rdf-fusion](https://crates.io/crates/rdf-fusion) — rdf-fusion crate page
- [IEEE Access — RDF Fusion paper](https://ieeexplore.ieee.org/document/11208525/) — Schwarzinger et al., 2025
- [GitHub — GrafeoDB/grafeo](https://github.com/GrafeoDB/grafeo) — Grafeo graph database
- [lib.rs — grafeo-engine](https://lib.rs/crates/grafeo-engine) — grafeo-engine crate
- [GitHub — terminusdb/terminusdb-store](https://github.com/terminusdb/terminusdb-store) — Rust triple store
- [GitHub — pchampin/sophia_rs](https://github.com/pchampin/sophia_rs) — Sophia RDF toolkit
- [GitHub — rudof-project/rudof](https://github.com/rudof-project/rudof) — rudof SHACL/ShEx library
- [docs.rs — oxirs-shacl](https://docs.rs/oxirs-shacl) — oxirs-shacl SHACL validator
- [lib.rs — shacl-rust](https://lib.rs/crates/shacl-rust) — shacl-rust validator
- [GitHub — gtfierro/reasonable](https://github.com/gtfierro/reasonable) — OWL 2 RL reasoner
- [GitHub — phillord/horned-owl](https://github.com/phillord/horned-owl) — horned-owl OWL library
- [GitHub — INCATools/whelk-rs](https://github.com/INCATools/whelk-rs) — whelk-rs OWL EL
- [GitHub — manomayam/manas](https://github.com/manomayam/manas/) — Solid server in Rust
- [rdf4j.org — Programming docs](https://rdf4j.org/documentation/programming/) — RDF4J framework
- [GitHub — ontop/ontop wiki — RDF4J endpoint](https://github.com/ontop/ontop/wiki/RDF4J-SPARQL-endpoint-installation) — Ontop/RDF4J integration
- [GitHub topics — r2rml](https://github.com/topics/r2rml) — R2RML ecosystem survey
- [awesome-kgc-tools](https://kg-construct.github.io/awesome-kgc-tools/) — KGC tool landscape
