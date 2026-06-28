# RDF4J — Java prior art survey for semantic-fabric

> Research date: 2026-06-26. KEY = `rdf4j`.

---

## 1. What is Eclipse RDF4J?

[Eclipse RDF4J](https://rdf4j.org/) (formerly OpenRDF Sesame) is the dominant Java framework for RDF storage, parsing, and querying. It is the substrate on which [Ontop](https://ontop-vkg.org/) is built: every Repository, every SPARQL evaluation, every graph write in Ontop goes through RDF4J. Understanding its design is therefore understanding half of the correctness reference we cross-check against (ADR-0005).

**Licence:** [Eclipse Distribution License 1.0 (EDL)](https://www.eclipse.org/org/documents/edl-v10.php) — BSD-3-Clause equivalent; permissive.

**Current versions:** 5.3.1 stable (April 2026); 6.0.0-M2 preview released June 2026, adding RDF 1.2 / SPARQL 1.2 support and Java 25.

---

## 2. SAIL — Storage And Inference Layer

The cornerstone of RDF4J's design is the [SAIL API](https://rdf4j.org/documentation/reference/sail/), a clean decoupling interface between the storage layer and the rest of the framework.

**Two interfaces define the contract:**

- `Sail` — the database entry point (open, shut down, configure).
- `SailConnection` — a transactional session (query, add/remove statements, commit, rollback).

**The `StackableSail` interface enables chain-of-responsibility composition.** Each SAIL either terminates the chain (persistence) or wraps another SAIL and intercepts operations before delegating:

```
ShaclSail
  └── SchemaCachingRDFSInferencer
        └── NativeStore    ← base SAIL; no StackableSail
```

Concrete implementations:

| Role | SAIL | Notes |
|---|---|---|
| Persistence | MemoryStore | RAM only, optional disk dump; fast for small datasets |
| Persistence | NativeStore | B-tree indexed on disk; good for 100K–100M triples |
| Persistence | LMDB Store | Memory-mapped; 6.0 adds improved cardinality estimation |
| Persistence | ElasticsearchStore | Stores triples directly in ES clusters |
| Adapter | SchemaCachingRDFSInferencer | Forward-chaining RDFS; caches schema axioms |
| Adapter | LuceneSail / SolrSail / ESail | Full-text + geospatial index overlay |
| Adapter | ShaclSail | Transaction-time SHACL validation (see §5) |

The infrastructure concern (concurrency, lifecycle) is handled in `AbstractSail`/`AbstractSailConnection`, which delegate core logic to protected `*Internal` methods — the Template Method pattern. This separates infrastructure from domain logic cleanly.

---

## 3. Rio — streaming RDF parser/writer

[Rio](https://rdf4j.org/documentation/programming/rio/) is RDF4J's I/O layer. Its design choices are directly mirrored by Rust's oxttl.

**Push-model:** Parsers drive output to an `RDFHandler` callback interface:

```
startRDF()  →  handleNamespace(prefix, iri) *  →  handleStatement(stmt) *  →  endRDF()
```

Because `RDFWriter` also implements `RDFHandler`, a parser and a writer can be chained directly — format conversion is a zero-copy, zero-intermediate-buffer pipeline. Errors can be made non-fatal (continue with `ParseErrorListener`), which maps to Rust's `Result`-streaming pattern.

Formats supported: Turtle, N-Triples, N-Quads, RDF/XML, JSON-LD, TriG, TriX, BinaryRDF, NDJSON-LD.

---

## 4. Repository / Connection API

The [Repository API](https://rdf4j.org/documentation/programming/repository/) is the high-level interface above SAIL.

**Three `Repository` implementations:**

- `SailRepository` — wraps a SAIL stack for local storage.
- `HTTPRepository` — proxies a remote RDF4J Server.
- `SPARQLRepository` — wraps any remote SPARQL endpoint.

`RepositoryConnection` is intentionally **not thread-safe**: each thread (or in async terms, each task) must acquire its own connection. The `try-with-resources` lifecycle maps directly to Rust's RAII/Drop idiom.

Query execution returns lazy iterators: `TupleQueryResult` (SELECT bindings), `GraphQueryResult` (CONSTRUCT/DESCRIBE statements), `BooleanQueryResult` (ASK). The pipeline is:

```
Repository → RepositoryConnection → compile query → SAIL evaluation → lazy result
```

**Transaction isolation levels** run from `NONE` (fastest, weakest) through `SNAPSHOT` to `SERIALIZABLE`. The [ShaclSail](https://rdf4j.org/documentation/programming/shacl/) exploits SNAPSHOT isolation with serializable validation — providing effective serializability at 2–4× the throughput of true SERIALIZABLE transactions.

**Federation (FedX)** is integrated: `FedXRepository` distributes query evaluation across member repositories. RDF4J 6.0.0-M2 adds grouped source selection to reduce round-trips.

---

## 5. SHACL SAIL — ShaclSail

[ShaclSail](https://rdf4j.org/documentation/programming/shacl/) is a `StackableSail` that validates at **transaction commit**, not at query time. It creates validation plans (analogous to query plans), runs SPARQL SELECT queries against the base SAIL to collect focus nodes, and checks each active shape.

**Constraint coverage** (SHACL Core): `sh:minCount`, `sh:maxCount`, `sh:qualifiedMinCount/MaxCount`, `sh:datatype`, `sh:class`, `sh:nodeKind`, `sh:hasValue`, `sh:in`, string constraints (`sh:minLength`, `sh:maxLength`, `sh:pattern`, `sh:languageIn`), numeric constraints (`sh:minExclusive/Inclusive`, `sh:maxExclusive/Inclusive`), logical operators (`sh:and`, `sh:or`, `sh:not`), property paths (single, inverse, sequence, alternation), `sh:sparql`, `sh:node`, `sh:property`. RDFS subclass reasoning is on by default.

**Performance features:** parallel validation enabled by default; result caching; auto-escalation to bulk mode above 500 000 statements per transaction. Shapes live in the reserved named graph `http://rdf4j.org/schema/rdf4j#SHACLShapeGraph`, architecturally isolated from data graphs.

**Benchmark context** (LUBM dataset, from the [rudof ISWC 2024 paper](https://ceur-ws.org/Vol-3828/paper32.pdf)):

| Validator | Time (ms) |
|---|---|
| RDF4J ShaclSail | 1.64 |
| **rudof (Rust)** | **7.90** |
| Apache Jena | 60.36 |
| TopQuadrant | 85.74 |
| pyrudof | 39 364 |
| pySHACL | 72 228 |

RDF4J's ShaclSail is the fastest Java validator; rudof is ~4.8× slower but still ~7.6× faster than Jena and ~8000× faster than Python.

---

## 6. RDFS/OWL Inferencing

RDF4J ships `SchemaCachingRDFSInferencer` for forward-chaining RDFS entailment. No built-in OWL 2 QL reasoner exists in RDF4J; Ontop implements OWL 2 QL entailment itself (T-mapping saturation, tree-witness rewriting) above the SAIL layer — it uses RDF4J for I/O and SPARQL execution, not for reasoning.

---

## 7. GeoSPARQL

The optional `rdf4j-queryalgebra-geosparql` module extends the SPARQL algebra with geospatial functions (`geof:distance`, `geof:buffer`, `geof:envelope`, `geof:intersection`, etc.) over WKT literals. The LuceneSAIL and its Solr/Elasticsearch variants add spatial indexing for large datasets. GeoSPARQL is a module concern, not a core one — a good pattern.

---

## 8. Architectural patterns semantic-fabric should adopt

### 8.1 SAIL stackable composition → Rust generic wrapper types

The `StackableSail<S: Store>` pattern translates directly to Rust generics with zero runtime cost:

```rust
struct ShaclLayer<S: Store> { inner: S, shapes: ShapesGraph }
impl<S: Store> Store for ShaclLayer<S> { ... }
```

Construction time determines the active layers; unused layers add no overhead. This is the Decorator pattern expressed as type-system composition, not dynamic dispatch.

### 8.2 Rio push-model → oxttl already uses this

oxttl's parser already pushes quads through a callback. The lesson: **semantic-fabric's materialization output path must follow the same model** — write triples into an `RDFHandler`-equivalent sink rather than buffering the full output graph. This is what allows SF100 (35 M triples) to stream without OOM.

### 8.3 Repository/Connection → Arc + async connection pool

Repository = `Arc<Backend>` shared across tasks. Connection = a pooled, per-task guard that is `Drop`-closed. In async Rust, "per-task" replaces "per-thread". Lazy query results become `Stream` items rather than `Iterator`.

### 8.4 SHACL as a commit-time interceptor

The ShaclSail validates on commit, not inline. For the M⋈T gate (ADR-0005), this maps to: run SHACL validation after materialization completes (or over the M⋈T closure for the virtualized path), not as a per-triple check during rewriting. Shape isolation in a reserved named graph prevents data from overriding constraint logic.

### 8.5 Explicit query algebra as IR

RDF4J's pipeline (string → algebra → evaluation strategy) is exactly semantic-fabric's pipeline (spargebra → IQ tree → SQL). The key lesson: **every optimization lives in the algebra transform step**, not in the emitter. The algebra is the only correct place to eliminate self-joins, prune empty branches, push filters.

---

## 9. Rust ecosystem implications

### 9.1 ADR-0005 open dependency: Rust SHACL engine

Three Rust SHACL crates exist:

| Crate | Version | Licence | Oxigraph dep | Status |
|---|---|---|---|---|
| [rudof / shacl_validation](https://github.com/rudof-project/rudof) | 0.3.4 (Jun 2026) | MIT / Apache-2.0 | Yes (sparql_service → Oxigraph) | Active; ISWC 2024; 114 stars |
| [oxirs-shacl](https://lib.rs/crates/oxirs-shacl) | early | unknown | Yes (OxiRS) | Core constraints + property paths; pre-alpha |
| [shacl-rust](https://github.com/ensaremirerol/shacl-rust) | early | unknown | unknown | Basic; unclear maintenance |

**Recommendation:** `rudof`'s `shacl_validation` crate resolves ADR-0005's open dependency. It is MIT/Apache-2.0 licensed, uses the same Oxigraph substrate as semantic-fabric, covers the SHACL Core constraints that the M⋈T meta-shapes require (`sh:class`, `sh:datatype`, `sh:property`, `sh:path`, `sh:in`, `sh:hasValue`), and has peer-reviewed validation (ISWC 2024). The 7.9ms LUBM result puts the M⋈T gate well inside an acceptable CI budget. Risk: documentation coverage is only 11.94%; the API surface will need direct testing against the four meta-shape IRIs from the upstream modelling project's mapping-conformance requirements.

### 9.2 ADR-0007 deferred dependency: OWL 2 QL reasoning

ADR-0007 explicitly defers OWL 2 QL (`501 Not Implemented`). For the eventual implementation, two Rust crates exist:

| Crate | Version | OWL Profile | Notes |
|---|---|---|---|
| [reasonable](https://github.com/gtfierro/reasonable) | 0.4.4 (May 2026) | OWL 2 **RL** | DataFrog Datalog rules; oxrdf 0.3.3; BSD-3-Clause; 7× faster than Allegro; eq-ref, cls-maxc unimplemented |
| [horned-owl](https://crates.io/crates/horned-owl) | 0.15+ (Jan 2026) | Parsing only | OWL ontology I/O; 20–40× faster than Java OWL API; not a reasoner |

**Critical mismatch:** `reasonable` implements OWL 2 **RL** (materialisation rules via Datalog), not OWL 2 **QL**. OBDA/tree-witness rewriting requires QL semantics. There is no production Rust OWL 2 QL reasoner as of mid-2026. This reinforces ADR-0007's deferral; when the time comes, the most viable path is likely a native Rust implementation of the QL→Datalog rewriting described in the Ontop literature (`ontop.md` §12), with `horned-owl` for ontology parsing and `reasonable` as inspiration for the Datalog evaluation engine.

---

## 10. Comparison table: RDF4J components → semantic-fabric analogs

| RDF4J concept | semantic-fabric analog | Notes |
|---|---|---|
| SAIL / StackableSail | `trait Store` + generic wrapper types | Type-system composition, zero-cost |
| MemoryStore | in-memory `oxrdf::Dataset` | Already available |
| NativeStore | future: indexed disk store | Not in scope v1 |
| LMDB Store | future: memory-mapped backend | Not in scope v1 |
| ShaclSail | `ShaclLayer<S>` using `rudof` | Resolves ADR-0005 |
| SchemaCachingRDFSInferencer | deferred | Not in v1 scope |
| Rio RDFHandler (push parser) | oxttl callbacks | Already in place |
| Repository/Connection | `Arc<Pool<Backend>>` + RAII guard | Async-task-scoped |
| SPARQL string → algebra | spargebra | Already adopted (ADR-0004) |
| IQ-style relational tree | `sf-sparql` IQ tree | Implemented per ADR-0007 |
| FedX federation | deferred (501) | ADR-0007 §v1 deferrals |
| GeoSPARQL module | deferred | Out of scope |

---

## Sources

- [RDF4J home — rdf4j.org](https://rdf4j.org/)
- [SAIL API reference](https://rdf4j.org/documentation/reference/sail/)
- [Rio parsing and writing](https://rdf4j.org/documentation/programming/rio/)
- [Repository API](https://rdf4j.org/documentation/programming/repository/)
- [SHACL validation](https://rdf4j.org/documentation/programming/shacl/)
- [Repository and SAIL configuration](https://rdf4j.org/documentation/reference/configuration/)
- [GeoSPARQL](https://rdf4j.org/documentation/programming/geosparql/)
- [RDF4J GitHub (eclipse-rdf4j/rdf4j)](https://github.com/eclipse-rdf4j/rdf4j)
- [rudof project](https://rudof-project.github.io/rudof/)
- [rudof GitHub (rudof-project/rudof)](https://github.com/rudof-project/rudof)
- [rudof ISWC 2024 paper — CEUR Vol-3828/paper32](https://ceur-ws.org/Vol-3828/paper32.pdf)
- [shacl_validation on docs.rs](https://docs.rs/shacl_validation/latest/shacl_validation/)
- [reasonable on docs.rs](https://docs.rs/reasonable/latest/reasonable/)
- [horned-owl on crates.io](https://crates.io/crates/horned-owl)
- [sophia Rust toolkit](https://pchampin.github.io/sophia_rs/ch00_introduction.html)
- [sophia v0.8 announcement](https://perso.liris.cnrs.fr/pierre-antoine.champin/blog/2024/sophia-v0.8/index.html)
