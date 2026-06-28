# Rust RDF Core Ecosystem — Beyond Oxigraph

**Topic:** Rust RDF data-model, parsing, serialization, and higher-level toolkits outside the Oxigraph crate family; plus SHACL and OWL crates that could resolve open semantic-fabric dependencies.

**Research round:** 2026-06-26 | Supplements: `oxigraph.md`, `rust-substrate.md`

---

## 1. Context

`semantic-fabric` is standardized on the [oxrdf](https://crates.io/crates/oxrdf) / [oxttl](https://crates.io/crates/oxttl) / [oxrdfio](https://crates.io/crates/oxrdfio) family for its RDF data model and serialization layer (ADR-0004). Two open dependencies remain:

* **ADR-0005** — a Rust SHACL engine to run the `M ⋈ T` conformance gate (`mf:MappingClassConformanceShape` etc.) over materialized/virtualized output. Flagged as unresolved; a Rust crate is preferred over shelling out.
* **ADR-0007** — OWL 2 QL entailment / tree-witness rewriting for the OBDA path. Explicitly deferred to v2, but the crate landscape must be understood to make that deferral honest.

This round surveys the full Rust RDF ecosystem to answer three questions: what else exists, what is reusable, and do either of the two dependencies have a credible Rust solution today.

---

## 2. The oxrdf / oxttl Family (Baseline)

The oxigraph project ([github.com/oxigraph/oxigraph](https://github.com/oxigraph/oxigraph)) publishes its RDF layer as standalone crates so it can be used without pulling in the full triplestore:

| Crate | Role | Latest stable | License |
|---|---|---|---|
| [oxrdf](https://crates.io/crates/oxrdf) | Core data model — `NamedNode`, `BlankNode`, `Literal`, `Triple`, `Quad`, `Dataset` | 0.2.x (Oxigraph 0.5 series) | MIT / Apache-2.0 |
| [oxttl](https://crates.io/crates/oxttl) | Streaming N-Triples, N-Quads, Turtle, TriG, N3 parser + serializer | same series | MIT / Apache-2.0 |
| [oxrdfxml](https://crates.io/crates/oxrdfxml) | RDF/XML parser + serializer | same series | MIT / Apache-2.0 |
| [oxrdfio](https://crates.io/crates/oxrdfio) | Unified format-agnostic I/O API wrapping oxttl + oxrdfxml | same series | MIT / Apache-2.0 |
| [spargebra](https://crates.io/crates/spargebra) | SPARQL 1.1 parser → algebra tree | same series | MIT / Apache-2.0 |
| [sparopt](https://crates.io/crates/sparopt) | SPARQL algebra optimizer (filter pushdown, join reorder) | same series | MIT / Apache-2.0 |
| [sparesults](https://crates.io/crates/sparesults) | SPARQL result serialization (JSON, XML, TSV) | same series | MIT / Apache-2.0 |

**Status:** Production. Oxigraph 0.5.7 shipped April 2026 ([changelog](https://raw.githubusercontent.com/oxigraph/oxigraph/master/CHANGELOG.md)). `oxrdf` supports RDF 1.2 behind the `rdf-12` feature flag. This is the correct baseline for semantic-fabric; the other crates surveyed below are evaluated against it.

---

## 3. Sophia — Champin's Trait-Based Toolkit

[sophia_rs](https://github.com/pchampin/sophia_rs) (Pierre-Antoine Champin, CNRS) takes a fundamentally different design stance: it is a **trait abstraction layer** so that multiple independent RDF implementations can interoperate without coupling to a single data representation.

### Architecture

The family is split across more than ten crates:

| Crate | Role |
|---|---|
| [sophia_api](https://crates.io/crates/sophia_api) | Foundational traits — `Term`, `Triple`, `Graph`, `Dataset`, `Parser`, `Serializer` |
| [sophia_turtle](https://crates.io/crates/sophia_turtle) | Turtle / N-Triples / TriG / N-Quads parser + serializer |
| [sophia_xml](https://crates.io/crates/sophia_xml) | RDF/XML |
| [sophia_jsonld](https://crates.io/crates/sophia_jsonld) | JSON-LD 1.1 (full spec) |
| [sophia](https://crates.io/crates/sophia) | Re-export facade bundling the above |
| [sophia_sparql](https://crates.io/crates/sophia_sparql) | SPARQL evaluation (experimental) |

### Maturity

`sophia_api` v0.9.0 was published November 2024 ([crates.io](https://crates.io/crates/sophia_api), 202 K all-time downloads). The v0.8 release, documented on the maintainer's [blog post (2024)](https://perso.liris.cnrs.fr/pierre-antoine.champin/blog/2024/sophia-v0.8/index.html), added full JSON-LD 1.1, W3C RDF Dataset Canonicalization (RDFC-1.0), and RDF-star. The project is active and academically anchored (presented at WWW 2020 Developers Track).

### Key property: the Oxigraph bridge

Oxigraph ships a **disabled-by-default `sophia` feature flag** that implements `sophia_api` traits on Oxigraph's own terms and stores ([docs.rs/oxigraph 0.2.4](https://docs.rs/oxigraph/0.2.4/oxigraph/)). This means:

> Any crate that depends only on `sophia_api` traits can accept either a pure-sophia graph or an Oxigraph store — the consumer does not need to know which one it is talking to.

The `hdt` crate (§5 below) also implements sophia_api traits on its compressed-RDF store. This pattern of cross-crate composition is sophia's primary value proposition.

### License

MIT / Apache-2.0 dual.

---

## 4. Rio — Deprecated Parser Library

[oxigraph/rio](https://github.com/oxigraph/rio) was the Oxigraph team's low-level parser library (N-Triples, N-Quads, Turtle, TriG, RDF/XML, and RDF-star variants). The repo was last touched April 2026, but the crates have been **officially deprecated** by the maintainers, who recommend [oxrdfio](https://crates.io/crates/oxrdfio) (format-agnostic), [oxttl](https://crates.io/crates/oxttl) (Turtle-family), or [oxrdfxml](https://crates.io/crates/oxrdfxml) instead. The [rio_api](https://lib.rs/crates/rio_api) and [rio_turtle](https://crates.io/crates/rio_turtle) crates remain on crates.io for existing dependents. **Do not take a new dependency on rio.**

---

## 5. hdt — Compressed RDF (Header Dictionary Triples)

[hdt](https://crates.io/crates/hdt) ([docs.rs](https://docs.rs/hdt/latest/hdt/)) is a Rust library for the HDT binary RDF compression format, published in a [2023 JOSS paper](https://joss.theoj.org/papers/10.21105/joss.05114) by Konrad Höffner and Tim Baccaert. HDT stores a large static RDF graph in a highly compressed in-memory layout that supports fast triple-pattern lookups. The crate:

* Implements **sophia_api traits**, making it composable with any sophia-aware consumer.
* Has an **experimental `sparql` feature** (backed by spareval, not oxigraph).
* Is **read-only** — cannot modify the loaded graph or swap to disk.
* Sees roughly 110 downloads/month and is used by RickView (a Linked Data browser). Last updated March 2026.

**Relevance for semantic-fabric:** Niche. HDT is appropriate for serving a large, rarely-updated materialized dump efficiently (e.g. a Linked Data fragment interface over a pre-built RDF output). It is not relevant to the R2RML engine core. License: MIT.

---

## 6. harriet — Turtle AST Parser

[harriet](https://crates.io/crates/harriet) (v0.3.1) is a **format-preserving** Turtle parser: it parses a Turtle document into a concrete syntax tree (AST) that retains all whitespace and comments, so re-serializing the parsed document yields the original input byte-for-byte. It deliberately does **not** interpret the Turtle into an RDF graph — that is left to extension crates. Last release was approximately three years ago (~mid-2023). **Dormant.** License: MIT (inferred from crates.io page).

**Relevance for semantic-fabric:** None in the engine path. harriet's only plausible niche is a Turtle formatter or linter that must round-trip documents. Even there, oxttl's streaming serializer is simpler for machine-generated output.

---

## 7. rdftk — Johnston's Toolkit

[rust-rdftk](https://github.com/johnstonskj/rust-rdftk) (Simon Johnston) is a suite of crates — [rdftk_core](https://crates.io/crates/rdftk_core), [rdftk_io](https://crates.io/crates/rdftk_io), [rdftk_names](https://crates.io/crates/rdftk_names), [rdftk_skos](https://crates.io/crates/rdftk_skos) — that targets **readability and usability over runtime performance** (per the project's own stated design philosophy). The project's README says the crates "are not yet complete." It received Debian packaging interest in 2025 ([Bug#1108622 ITP](https://www.mail-archive.com/debian-devel@lists.debian.org/msg386755.html)), which indicates some downstream adoption, but activity has been sparse compared to the oxigraph or sophia families.

**Relevance for semantic-fabric:** None. The explicit de-prioritization of performance rules out adoption in a throughput-sensitive engine. License: MIT.

---

## 8. rdf.rs — Public Domain Framework

[github.com/rust-rdf/rdf.rs](https://github.com/rust-rdf/rdf.rs) (rust-rdf organization) is a framework for RDF knowledge graphs targeting `no_std` environments with comprehensive feature flags to opt out of any capability. It is 100% public-domain (CC0) software. The crate ([rdf_rs on crates.io](https://crates.io/crates/rdf_rs)) is early-stage and has minimal download counts. Listed for completeness; not a candidate for semantic-fabric adoption.

---

## 9. SHACL: rudof (Resolves ADR-0005 Open Dependency)

[rudof](https://github.com/rudof-project/rudof) (rudof-project) is the only substantive Rust SHACL implementation available as of June 2026. It supports **ShEx, SHACL Core, and DCTAP**, plus conversions between shape formalisms. Key facts:

* **Crates:** [rudof_lib](https://docs.rs/crate/rudof_lib/latest) (v0.3.3), [shacl_ast](https://crates.io/crates/shacl_ast) (SHACL Core abstract syntax), `shacl_validation`, `shacl_ir`, [shacl_testsuite](https://crates.io/crates/shacl_testsuite), [rudof_cli](https://lib.rs/crates/rudof_cli).
* **Python bindings** via PyO3 ([pyrudof on PyPI](https://pypi.org/project/rudof/)).
* An **MCP server** ([rudof_mcp v0.3.1](https://docs.rs/crate/rudof_mcp/latest)) was presented at ESWC 2026 ([eswc2026-rudof-mcp](https://github.com/rudof-project/eswc2026-rudof-mcp)), demonstrating active cutting-edge development.
* Presented in the Demos & Posters track at **ISWC 2024** ([CEUR-WS paper](https://ceur-ws.org/Vol-3828/paper32.pdf)), confirming academic recognition and stability of the core design.
* W3C SHACL test-suite **coverage is not yet publicly quantified** — the SHACL validation roadmap is tracked in [issue #94](https://github.com/rudof-project/rudof/issues) (opened August 2024). The presence of a dedicated `shacl_testsuite` crate and active CI suggests the suite is being run, but pass-rate numbers are not published as of this writing.
* **License:** MIT / Apache-2.0.

**Assessment for ADR-0005:** rudof is the clear candidate for the `M ⋈ T` SHACL hook. The architecture (SHACL Core AST → IR → validation algorithm) maps directly to evaluating `mf:MappingClassConformanceShape` and companions over an oxrdf `Dataset`. The Oxigraph/oxrdf interop path is straightforward since rudof already depends on oxrdf types. The open risk is test-suite coverage for SHACL Core's constraint components that appear in the upstream modelling project's meta-shapes (specifically `sh:class`, `sh:datatype`, `sh:pattern`, and `sh:minCount` / `sh:maxCount` — all SHACL Core, not SHACL-SPARQL, so coverage should be high). Validate this against [shacl_testsuite](https://crates.io/crates/shacl_testsuite) before committing.

---

## 10. OWL: horned-owl and reasonable (Context for ADR-0007)

Two crates are relevant to the OWL 2 deferral in ADR-0007:

### horned-owl

[horned-owl](https://crates.io/crates/horned-owl) (Phillip Lord; [github.com/phillord/horned-owl](https://github.com/phillord/horned-owl)) is a **structural OWL 2 library** — it reads and writes OWL documents (OWL/XML, Functional Syntax) and provides an in-memory model of OWL 2 axioms. It is **not a reasoner**. v1.4.0 was published approximately 5 months before this writing (January 2026), with 45 K all-time downloads and preliminary results showing 20–40× faster I/O than the OWL API ([Dagstuhl TGDK paper](https://drops.dagstuhl.de/storage/08tgdk/tgdk-vol002/tgdk-vol002-issue002/TGDK.2.2.9/TGDK.2.2.9.pdf)). License: MIT.

### reasonable

[reasonable](https://crates.io/crates/reasonable) ([github.com/gtfierro/reasonable](https://github.com/gtfierro/reasonable)) is an **OWL 2 RL reasoner** built on [datafrog](https://crates.io/crates/datafrog) (a lightweight Datalog engine). It offers a Rust library, a binary, and Python bindings. 130 K all-time downloads; last updated May 2026.

**Critical distinction for semantic-fabric:** ADR-0007 defers OWL 2 QL entailment / tree-witness rewriting. `reasonable` implements **OWL 2 RL**, which is a different profile. OWL 2 QL is the profile used by Ontop for SPARQL-under-ontology OBDA (DL-Lite_R). OWL 2 RL is primarily for reasoning over large ABoxes under a closed-world assumption — a materialization-time concern, not a query-rewriting concern. No Rust crate implementing OWL 2 QL (DL-Lite_R classification + T-mapping saturation) was identified in this survey. If and when semantic-fabric implements v2 OWL 2 QL support, that logic will need to be built, not acquired from crates.io.

---

## 11. Comparison Table

| Crate / family | Scope | Maturity | Last active | License | Adopt? |
|---|---|---|---|---|---|
| **oxrdf / oxttl / oxrdfio** | Data model, parsing, serialization | Production | April 2026 | MIT/Apache-2.0 | Already adopted (ADR-0004) |
| **sophia_api** | Trait abstraction for interop | Stable (0.9.0) | Nov 2024 | MIT/Apache-2.0 | Adopt via optional feature if multi-backend interop needed; Oxigraph bridge exists |
| **sophia** (full) | Parsers, JSON-LD, canonicalization, SPARQL | Stable (0.8.x) | Nov 2024 | MIT/Apache-2.0 | Pull only sophia_jsonld if JSON-LD input is ever needed |
| **rio** | N-Triples/Turtle/TriG/RDF-XML parsers | **Deprecated** | April 2026 (final) | MIT/Apache-2.0 | Do not use |
| **hdt** | Compressed read-only RDF (HDT format) | Niche-stable | March 2026 | MIT | Not needed for engine core |
| **harriet** | Turtle AST (format-preserving) | **Dormant** (v0.3.1, 2023) | ~2023 | MIT | Do not use |
| **rdftk** | General RDF toolkit (readability-first) | Pre-stable, incomplete | 2024/2025 | MIT | Do not use |
| **rdf.rs** | no_std RDF framework (public domain) | Early-stage | 2024 | CC0 | Do not use |
| **rudof / shacl_validation** | SHACL Core + ShEx + DCTAP validator | Active (v0.3.3) | June 2026 | MIT/Apache-2.0 | **Candidate for ADR-0005 hook** |
| **horned-owl** | OWL 2 structure + I/O (no reasoning) | Stable (v1.4.0) | Jan 2026 | MIT | Potentially useful for reading OWL ontologies if needed |
| **reasonable** | OWL 2 **RL** reasoner (Datalog) | Active (130 K DLs) | May 2026 | MIT | Not the right profile for ADR-0007 (need QL, not RL) |

---

## 12. Should semantic-fabric Adopt Sophia's Trait Abstraction?

The question is whether `semantic-fabric` should add a dependency on `sophia_api` traits alongside or instead of working directly with `oxrdf` types throughout its codebase.

**Arguments for:** Any crate implementing sophia_api can be plugged in as a graph source or sink — `hdt` for compressed archive output, alternative in-memory graphs, future stores. The interop layer costs nothing at runtime (trait dispatch, zero-cost abstraction per Rust's design). Sophia's JSON-LD 1.1 crate (`sophia_jsonld`) is the most complete Rust JSON-LD implementation and would be the only reasonable route if semantic-fabric ever needs JSON-LD input parsing.

**Arguments against:** `semantic-fabric`'s core pipeline (R2RML parsing → mapping IR → SQL execution → RDF reconstruction → serialization) touches the RDF layer primarily at two points: building quads from SQL result rows (using `oxrdf` constructors) and serializing the output quad stream (using `oxttl`/`oxrdfio`). Neither of those points benefits from trait abstraction — the quad type is fixed (oxrdf), and the output format is determined at runtime by the user's requested MIME type. Introducing sophia_api traits at the internal API boundary would add lifetime complexity (sophia's `Term` trait design involves GAT-adjacent patterns) without practical benefit for the current architecture.

**Recommendation:** Do not adopt sophia_api traits in the semantic-fabric core crate APIs. Enable the Oxigraph `sophia` feature only if a sophia-aware consumer (e.g. the rudof SHACL runner) needs to read from an Oxigraph store — check whether rudof's next release adds direct oxrdf support before adding that bridge. Pull `sophia_jsonld` as an optional dependency only if JSON-LD becomes an input format requirement.

---

## 13. Implications for ADR-0005 and ADR-0007

**ADR-0005 (SHACL, M ⋈ T gate):** `rudof` / `shacl_validation` resolves the open dependency in principle. The recommended path is:

1. Add `rudof_lib` as a dependency of `sf-conformance`.
2. Load the upstream modelling project's meta-shapes (the four `mf:*ConformanceShape` IRIs) from their canonical Turtle file via `oxrdfio`, then pass them and the test dataset to `shacl_validation`.
3. Gate merging the hook implementation on verifying that `shacl_testsuite` passes the SHACL Core `sh:class`, `sh:datatype`, `sh:pattern`, `sh:minCount` / `sh:maxCount` constraint components — which are the ones the meta-shapes use.

This keeps the dependency minimal (rudof_lib, not the full rudof_cli) and in-process (no shell-out). Risk: rudof's W3C test-suite coverage is still unquantified; a validation sprint against the W3C suite is the required pre-condition before committing.

**ADR-0007 (OWL 2 QL, deferred):** The survey confirms the deferral is correct. No Rust crate implements OWL 2 QL / DL-Lite_R entailment for SPARQL rewriting. If v2 OWL 2 QL entailment is ever built, it will require: (a) `horned-owl` to parse the OWL 2 QL ontology file into an axiom set, (b) a custom DL-Lite_R classifier (tractable, but non-trivial to implement correctly), and (c) T-mapping saturation logic ported from Ontop's Java. `reasonable` (OWL 2 RL) is irrelevant to this path.

---

## 14. Sources

* [sophia_rs GitHub](https://github.com/pchampin/sophia_rs)
* [sophia_api on crates.io](https://crates.io/crates/sophia_api)
* [sophia_api docs.rs](https://docs.rs/sophia_api)
* [Sophia v0.8 blog post, 2024](https://perso.liris.cnrs.fr/pierre-antoine.champin/blog/2024/sophia-v0.8/index.html)
* [oxrdf on crates.io](https://crates.io/crates/oxrdf) / [docs.rs](https://docs.rs/oxrdf/latest/oxrdf/)
* [oxigraph GitHub](https://github.com/oxigraph/oxigraph)
* [Oxigraph CHANGELOG](https://raw.githubusercontent.com/oxigraph/oxigraph/master/CHANGELOG.md)
* [rio GitHub (deprecated)](https://github.com/oxigraph/rio)
* [hdt on crates.io](https://crates.io/crates/hdt) / [docs.rs](https://docs.rs/hdt/latest/hdt/)
* [hdt JOSS paper 2023](https://joss.theoj.org/papers/10.21105/joss.05114)
* [harriet on crates.io](https://crates.io/crates/harriet)
* [rust-rdftk GitHub](https://github.com/johnstonskj/rust-rdftk)
* [rdftk_core on crates.io](https://crates.io/crates/rdftk_core)
* [rdf.rs GitHub](https://github.com/rust-rdf/rdf.rs)
* [rudof GitHub](https://github.com/rudof-project/rudof)
* [rudof project overview](https://rudof-project.github.io/rudof/)
* [shacl_ast on crates.io](https://crates.io/crates/shacl_ast)
* [shacl_testsuite on crates.io](https://crates.io/crates/shacl_testsuite)
* [rudof_lib docs.rs](https://docs.rs/crate/rudof_lib/latest)
* [rudof_mcp docs.rs](https://docs.rs/crate/rudof_mcp/latest)
* [rudof CEUR-WS ISWC 2024 paper](https://ceur-ws.org/Vol-3828/paper32.pdf)
* [eswc2026-rudof-mcp reproducibility repo](https://github.com/rudof-project/eswc2026-rudof-mcp)
* [horned-owl on crates.io](https://crates.io/crates/horned-owl) / [GitHub](https://github.com/phillord/horned-owl)
* [Horned-OWL Dagstuhl paper](https://drops.dagstuhl.de/storage/08tgdk/tgdk-vol002/tgdk-vol002-issue002/TGDK.2.2.9/TGDK.2.2.9.pdf)
* [reasonable on crates.io](https://crates.io/crates/reasonable) / [GitHub](https://github.com/gtfierro/reasonable)
* [W3C SHACL spec](https://www.w3.org/TR/shacl/)
* [W3C OWL 2 Profiles](https://www.w3.org/TR/owl2-profiles/)
