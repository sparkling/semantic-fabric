# Oxigraph Crate Substrate Research

**Topic:** Oxigraph (Rust) — crate-level analysis for semantic-fabric  
**Date:** 2026-06-26  
**Evidence grade:** High — primary sources (docs.rs, GitHub, crates.io, official changelogs)

---

## Executive Summary

Oxigraph is a Rust workspace of composable RDF/SPARQL crates, all dual-licensed MIT/Apache-2.0. For semantic-fabric, the split we need exists and is **architecturally clean**: `spargebra` (SPARQL parser → algebra) and `sparopt` (algebra optimizer) have **zero dependency** on any evaluator or storage backend. We consume their output AST and feed it into our own SQL-rewriting engine. We also reuse `oxrdf` as the canonical term/type system, `oxttl`/`oxrdfio` for RDF I/O, and `sparesults` for SPARQL results wire formats. The main `oxigraph` triplestore crate (RocksDB/in-memory store) is **not needed** and not pulled in. All crates are on 0.x semver, so breaking changes between minor versions are possible — pin at 0.5.x and upgrade deliberately.

---

## 1. Crate Inventory

| Crate | Version (Jun 2026) | License | Role |
|-------|-------------------|---------|------|
| [oxrdf](https://docs.rs/oxrdf/latest/oxrdf/) | **0.3.3** | MIT/Apache-2.0 | RDF data model (terms, triples, quads, graphs) |
| [oxttl](https://docs.rs/oxttl/latest/oxttl/) | **0.2.3** | MIT/Apache-2.0 | Turtle/N-Triples/N-Quads/TriG/N3 parse+serialize |
| [oxrdfio](https://docs.rs/oxrdfio/latest/oxrdfio/) | **0.2.5** | MIT/Apache-2.0 | Unified RDF I/O facade (wraps oxttl, oxrdfxml, oxjsonld) |
| [spargebra](https://docs.rs/spargebra/latest/spargebra/) | **0.4.6** | MIT/Apache-2.0 | SPARQL 1.1 parser → algebra AST |
| [sparopt](https://docs.rs/sparopt/latest/sparopt/) | **0.3.6** | MIT/Apache-2.0 | SPARQL algebra optimizer (pre-evaluation rewrites) |
| [sparesults](https://docs.rs/sparesults/latest/sparesults/) | **0.3.3** | MIT/Apache-2.0 | SPARQL results I/O (JSON/XML/CSV/TSV) |
| [spareval](https://docs.rs/spareval/latest/spareval/) | **0.2.6** | MIT/Apache-2.0 | SPARQL evaluator over `QueryableDataset` trait |
| [oxsdatatypes](https://docs.rs/oxsdatatypes/latest/oxsdatatypes/) | **0.2.2** | MIT/Apache-2.0 | XSD datatype implementations |
| [oxigraph](https://docs.rs/oxigraph/latest/oxigraph/) | **0.5.7** | MIT/Apache-2.0 | Full triplestore (RocksDB + in-memory); NOT needed |

Sources: [oxigraph changelog](https://raw.githubusercontent.com/oxigraph/oxigraph/master/CHANGELOG.md) · [GitHub repo](https://github.com/oxigraph/oxigraph) · [crates.io/oxigraph](https://crates.io/crates/oxigraph)

---

## 2. Crates to Reuse (with rationale)

### 2.1 oxrdf — the term system

[oxrdf](https://docs.rs/oxrdf/latest/oxrdf/) provides the canonical RDF data structures that every other crate in the ecosystem speaks. It carries no dependency on any evaluator or store.

Core types relevant to semantic-fabric:

- `NamedNode` / `NamedNodeRef` — IRIs (map to R2RML `rr:template` / `rr:column` / `rr:constant` outputs)  
- `BlankNode` / `BlankNodeRef` — blank nodes  
- `Literal` / `LiteralRef` — typed or language-tagged literals; the `datatype()` accessor returns a `NamedNode` for the XSD type  
- `Triple` / `TripleRef` — subject/predicate/object  
- `Quad` / `QuadRef` — triple + graph name (needed for named-graph-aware R2RML `rr:graphName` rules)  
- `Term` / `TermRef` — union of NamedNode | BlankNode | Literal | Triple  
- `Variable` / `VariableRef` — SPARQL variables (shared with spargebra)  
- `Dataset` / `Graph` — in-memory collections (useful in materialization mode before serializing)

**Verdict: reuse entirely.** This is the type currency across all crates; aligning semantic-fabric on oxrdf avoids conversion overhead and keeps API surfaces consistent.

### 2.2 oxttl — Turtle/N-Triples I/O

[oxttl](https://docs.rs/oxttl/latest/oxttl/) is a low-level, streaming parser and serializer for N-Triples, N-Quads, Turtle, TriG, and N3. It is explicitly designed for both synchronous and `async`/Tokio I/O. RDF 1.2 support is available via feature flag.

Relevant for semantic-fabric:
- **Materialization output**: serialize the generated `Quad` stream to Turtle or N-Triples via `NTriplesSerializer` / `TurtleSerializer`  
- **Streaming design**: parsers yield `Result<Quad>` iterators — fits our batch-pipeline architecture  
- **Tokio feature**: `async-tokio` feature gate for async serialization if we need backpressure-aware pipelines

**Verdict: reuse for all RDF serialization in materialization mode.** Prefer `oxttl` directly over `oxrdfio` when we only need Turtle/N-Triples (avoids pulling in JSON-LD and RDF/XML transitive deps).

### 2.3 oxrdfio — unified RDF format facade (conditional)

[oxrdfio](https://docs.rs/oxrdfio/latest/oxrdfio/) wraps `oxttl`, `oxrdfxml`, and `oxjsonld` behind a single `RdfParser` / `RdfSerializer` API keyed by `RdfFormat` enum. Async-Tokio is an optional feature.

Useful if semantic-fabric needs to accept or emit multiple RDF formats from a single code path. For the initial RDBMS-only scope, `oxttl` alone is sufficient. Consider `oxrdfio` when adding JSON-LD output for linked data publishing.

**Verdict: optional facade — include when multi-format I/O is required.**

### 2.4 spargebra — SPARQL 1.1 parser + algebra (critical path)

[spargebra](https://docs.rs/spargebra/latest/spargebra/) is the entry point for semantic-fabric's virtualization (OBDA) mode. It parses a SPARQL query string into a structured `Query` enum and exposes the full SPARQL 1.1 algebra as Rust enums.

**Dependencies:** only `oxrdf`, `oxiri`, `oxilangtag`. No evaluator, no store.

Main API:

```rust
use spargebra::SparqlParser;

let query = SparqlParser::new()
    .parse_query("SELECT ?s ?name WHERE { ?s <ex:name> ?name }").unwrap();
// query is a Query enum; the algebra is in query.dataset/pattern fields
```

**GraphPattern enum variants** (the algebra nodes our SQL translator must handle):

| Variant | SPARQL construct | SQL analog |
|---------|-----------------|-----------|
| `Bgp { patterns }` | Basic graph pattern | `JOIN` chain via R2RML triple maps |
| `Path { subject, path, object }` | Property path | Recursive CTE or loop (complex) |
| `Join { left, right }` | Inner join | SQL `INNER JOIN` |
| `LeftJoin { left, right, expression }` | OPTIONAL | SQL `LEFT JOIN` |
| `Filter { inner, expression }` | FILTER | SQL `WHERE` |
| `Union { left, right }` | UNION | SQL `UNION ALL` |
| `Extend { inner, variable, expression }` | BIND | SQL computed column |
| `Minus { left, right }` | MINUS | SQL `EXCEPT` / `NOT EXISTS` |
| `Values { variables, bindings }` | VALUES | SQL `VALUES` table |
| `Project { inner, variables }` | SELECT vars | SQL `SELECT` list |
| `Distinct { inner }` | DISTINCT | SQL `DISTINCT` |
| `Slice { inner, start, length }` | LIMIT/OFFSET | SQL `LIMIT`/`OFFSET` |
| `OrderBy { inner, expression }` | ORDER BY | SQL `ORDER BY` |
| `Group { inner, variables, aggregates }` | GROUP BY + agg | SQL `GROUP BY` |
| `Reduced { inner }` | REDUCED | optional SQL `DISTINCT` |
| `Graph { name, inner }` | GRAPH | named-graph filter |
| `Service { name, inner, .. }` | SERVICE | federated (out of scope v1) |
| `Lateral { left, right }` | LATERAL | correlated subquery (feature-gated) |

**sparql-12 feature** adds SPARQL 1.2 syntax; enable only when needed.

**Verdict: reuse as the SPARQL parse/algebra layer.** This is a clean building block — the crate's stated purpose is "intended to be a building piece for SPARQL implementations in Rust like Oxigraph." semantic-fabric consumes the `GraphPattern` tree and translates each node to SQL.

### 2.5 sparopt — SPARQL algebra optimizer

[sparopt](https://docs.rs/sparopt/latest/sparopt/) rewrites the `spargebra` `GraphPattern` tree before evaluation, applying standard rewrites (filter pushdown, join reordering, etc.). It takes spargebra algebra in and emits semantically equivalent (but more efficient) spargebra algebra out. No store dependency.

```rust
use sparopt::Optimizer;
let optimized_algebra = Optimizer::default().optimize(parsed_query_algebra);
```

Documentation coverage is 32% — the crate is marked work-in-progress, but the `Optimizer::optimize` entry point is stable enough for use.

**Verdict: run sparopt before feeding the algebra to our SQL translator.** Standard algebra rewrites (especially filter pushdown) improve generated SQL quality. Mark as an optional stage that can be bypassed if it causes regressions.

### 2.6 sparesults — SPARQL results wire format

[sparesults](https://docs.rs/sparesults/latest/sparesults/) handles SPARQL results serialization and parsing for:
- `application/sparql-results+json` (SPARQL 1.1 JSON)
- `application/sparql-results+xml` (SPARQL XML)
- `text/csv` / `text/tab-separated-values` (CSV/TSV)

In virtualization mode, our SQL-rewriting evaluator produces `QuerySolution` instances (mapping variable names to `Term` values). `sparesults` serializes these to the wire format the HTTP client requested.

```rust
use sparesults::{QueryResultsFormat, QueryResultsSerializer};

let mut writer = QueryResultsSerializer::from_format(QueryResultsFormat::Json)
    .serialize_solutions_to_writer(std::io::stdout(), vec![var_s.clone()])?;
for solution in sql_result_iter {
    writer.serialize(&solution)?;
}
writer.finish()?;
```

**Verdict: reuse for all SPARQL results I/O.** No evaluator or store dependency.

### 2.7 oxsdatatypes — XSD datatype arithmetic

[oxsdatatypes](https://docs.rs/oxsdatatypes/latest/oxsdatatypes/) implements the XSD numeric, datetime, duration, and gregorian types needed for SPARQL built-in functions. Useful when implementing FILTER expression evaluation within our SQL translator for expressions that cannot be pushed into SQL (e.g., XSD arithmetic on string-typed columns).

**Verdict: include as a dependency when implementing SPARQL expression evaluation.** Most filter expressions can be pushed to SQL; `oxsdatatypes` handles residual in-memory evaluation.

---

## 3. What NOT to Reuse

### 3.1 spareval — do not use the evaluator

[spareval](https://docs.rs/spareval/latest/spareval/) evaluates SPARQL algebra by iterating over `internal_quads_for_pattern()` calls against an in-memory `Dataset` (or any `QueryableDataset` implementor). This is the triple-pattern-at-a-time pull model — fundamentally incompatible with SQL-rewriting OBDA, where we want to push the whole query into the database as a single SQL statement.

The `QueryableDataset` trait is documented here for reference only:

```rust
// From spareval docs — NOT used by semantic-fabric
trait QueryableDataset {
    type InternalTerm: Clone + Eq + Hash;
    type Error: Error + Send + Sync + 'static;

    fn internal_quads_for_pattern(
        &self,
        subject: Option<&Self::InternalTerm>,
        predicate: Option<&Self::InternalTerm>,
        object: Option<&Self::InternalTerm>,
        graph_name: Option<Option<&Self::InternalTerm>>,
    ) -> Box<dyn Iterator<Item = Result<[Self::InternalTerm; 4], Self::Error>>>;

    fn internalize_term(&self, term: Term) -> Option<Self::InternalTerm>;
    fn externalize_term(&self, term: &Self::InternalTerm) -> Term;
    // ...
}
```

If a future version of semantic-fabric needs to fall back to in-memory evaluation (e.g., for SPARQL SERVICE or path expressions we cannot yet translate), we could implement `QueryableDataset` against materialized rows — but this is not the primary code path.

**Verdict: do not depend on spareval.** Write our own `SparqlToSqlTranslator` that consumes `spargebra::algebra::GraphPattern` and emits SQL.

### 3.2 oxigraph (main) — not needed

The `oxigraph` crate bundles a RocksDB-backed or in-memory triplestore. It brings in librocksdb-sys (a large C++ dep) and is irrelevant to semantic-fabric's RDBMS-over-R2RML architecture.

**Verdict: do not depend on `oxigraph`.** Depend only on the sub-crates above.

---

## 4. The Separation is Architecturally Clean

The clean split can be verified by the crate dependency graph:

```
oxrdf                              (terms, no store)
  └── oxttl / oxrdfio              (I/O, no store)
  └── spargebra                    (parser+algebra, no evaluator, no store)
        └── sparopt                (optimizer, no evaluator, no store)
              └── [semantic-fabric SPARQL→SQL translator]
              └── sparesults        (wire format, no evaluator)
  └── spareval                     (evaluator — NOT used by semantic-fabric)
        └── oxigraph Store          (triplestore — NOT used by semantic-fabric)
```

`spargebra` and `sparopt` have been confirmed to carry **no dependency on spareval or oxigraph**. We call `SparqlParser::new().parse_query(...)`, run it through `Optimizer::default().optimize(...)`, then walk the resulting `GraphPattern` tree in our own translator. This is precisely the architecture the Oxigraph maintainers intended ("intended to be a building piece for SPARQL implementations").

---

## 5. API Stability Assessment

All crates are on **0.x semver** — breaking changes between minor versions are expected and have occurred. The major migration was **0.4 → 0.5** (released circa late 2025), which:

- Replaced `rdf-star` with `rdf-12` feature (triple terms in subject position dropped)
- Deprecated `Store::query` in favor of a new `SparqlEvaluator` API
- Introduced stronger lifetime bounds on transactions
- Rewrote RocksDB transactions to use `WriteBatchWithIndex` (irrelevant to us)
- Added `spareval::QueryEvaluator::prepare()` method

The 0.5.x series (currently at **0.5.7**, 2026-04-19) is stable within the minor line. We should:
1. Pin `spargebra = "0.4.6"`, `oxrdf = "0.3.3"`, etc. with exact minor versions in `Cargo.toml`
2. Track the upstream changelog before upgrading
3. Avoid the `rdf-star` feature entirely (it's gone in 0.5)

The sub-crates (`spargebra`, `oxrdf`, `oxttl`, `sparesults`) have consistently followed the oxigraph main version train in lockstep, which makes coordinated upgrades straightforward.

---

## 6. Licensing Summary

Every crate in the Oxigraph workspace is dual-licensed **MIT OR Apache-2.0**, with "at your option" contributor grant. This is the most permissive licensing available in the Rust ecosystem and imposes no constraints on semantic-fabric's own licensing choice. Dependency on any of these crates is safe for commercial use.

Sources: [oxrdf license](https://docs.rs/oxrdf/latest/oxrdf/) · [spargebra license](https://docs.rs/spargebra/latest/spargebra/) · [GitHub license](https://github.com/oxigraph/oxigraph/blob/main/LICENSE-MIT)

---

## 7. Recommended Cargo.toml Fragment

```toml
[dependencies]
# RDF term system
oxrdf = "0.3"

# RDF I/O (materialization output)
oxttl = "0.2"
# oxrdfio = "0.2"   # add when multi-format output needed

# SPARQL parse + algebra (virtualization entry point)
spargebra = "0.4"
sparopt  = "0.3"

# SPARQL results wire format (virtualization output)
sparesults = "0.3"

# XSD datatype arithmetic for FILTER expressions
oxsdatatypes = "0.2"

# spareval = ...   # NOT included
# oxigraph = ...   # NOT included
```

---

## 8. Open Questions

1. **Path expressions**: `GraphPattern::Path` (property paths) do not translate cleanly to flat SQL. Plan for a recursive CTE strategy or a flag that rejects path queries in v1.

2. **SPARQL 1.2 feature flag**: should semantic-fabric expose the `sparql-12` / `rdf-12` feature gate from day one, or stay on strict 1.1?

3. **sparopt maturity**: sparopt has only 32% API doc coverage and is marked "work in progress." Evaluate whether to run it unconditionally or make it opt-in per query.

4. **SERVICE federated queries**: `GraphPattern::Service` cannot be SQL-rewritten without a SPARQL endpoint registry. Plan to return a `501 Not Implemented` in v1.

5. **Named graphs in R2RML**: `GraphPattern::Graph` requires named-graph awareness in the mapping engine. Confirm our R2RML IR models `rr:graphName` correctly.

6. **API stability watchpoint**: watch [oxigraph releases](https://github.com/oxigraph/oxigraph/releases) for 0.5→0.6 migration; spargebra `GraphPattern` enum variants are likely to gain variants (additive) rather than lose them.

---

## Sources

- [oxigraph GitHub](https://github.com/oxigraph/oxigraph) — main repository
- [oxigraph CHANGELOG](https://raw.githubusercontent.com/oxigraph/oxigraph/master/CHANGELOG.md) — version history
- [Oxigraph lib/README.md](https://github.com/oxigraph/oxigraph/blob/main/lib/README.md) — crate inventory
- [spargebra docs.rs](https://docs.rs/spargebra/latest/spargebra/) — v0.4.6, algebra API
- [spareval docs.rs](https://docs.rs/spareval/latest/spareval/) — v0.2.6, QueryableDataset trait
- [oxrdf docs.rs](https://docs.rs/oxrdf/latest/oxrdf/) — v0.3.3, term types
- [oxttl docs.rs](https://docs.rs/oxttl/latest/oxttl/) — v0.2.3, RDF serializers
- [oxrdfio docs.rs](https://docs.rs/oxrdfio/latest/oxrdfio/) — v0.2.5, unified I/O facade
- [sparopt docs.rs](https://docs.rs/sparopt/latest/sparopt/) — v0.3.6, optimizer
- [sparesults docs.rs](https://docs.rs/sparesults/latest/sparesults/) — v0.3.3, results formats
- [oxsdatatypes docs.rs](https://docs.rs/oxsdatatypes/latest/oxsdatatypes/) — v0.2.2, XSD types
- [DeepWiki Oxigraph architecture](https://deepwiki.com/oxigraph/oxigraph) — architecture overview
- [spargebra algebra.rs (GitHub)](https://github.com/oxigraph/oxigraph/tree/main/lib/spargebra) — GraphPattern source
- [oxigraph v0.5.0 release](https://github.com/oxigraph/oxigraph/releases/tag/v0.5.0) — major migration notes
- [crates.io/oxigraph](https://crates.io/crates/oxigraph) — package registry
