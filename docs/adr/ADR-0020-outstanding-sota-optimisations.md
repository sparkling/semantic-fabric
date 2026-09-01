---
status: accepted
date: 2026-06-27
updated: 2026-09-01
tags: [sota, roadmap, optimisation, cost-driven-obda, full-text-search, caching, geosparql, service-federation, term-generation, research-register]
supersedes: []
depends-on:
  - ADR-0006
  - ADR-0007
implements:
  - ADR-0001
---

# Outstanding SOTA optimisations — research register & dispositions

## Context and Problem Statement

Six non-blocking SOTA levers for the virtualiser were deep-researched (2026-06-27) against our architecture (virtualisation-only, R2RML/relational, Oxigraph crates, streaming bounded-memory, source push-down). **None gate the engine build** — the SPARQL 1.2→SQL rewriter core comes first. This ADR records each item's **disposition**, the load-bearing technique, rough effort, and — for deferred items — the **design seam to reserve now** so it bolts on later without rework. Items marked *pursue* graduate to their own ADR when built.

**Recurring lens:** every item is judged by whether it honours the defining invariant — *the source does the set-work; engine memory is bounded by `⟨T, M⟩` + a fixed streaming budget, not by data* (ADR-0006/0010). That single test kills in-engine result caching and live `SERVICE`, and shapes the FTS surface.

**Promotion (updated 2026-09-01):** four foundational items are **binding in the load-bearing ADRs**, not roadmap here — IRI-template / term-construction **lifting** and the **plan cache** into **ADR-0007**; the **term-gen allocation discipline** and the **cross-source semi-join cost** model into **ADR-0006**. The single-source binding/cache namespace and its unverified-constraint policy are implemented; digest-addressed snapshot invalidation, structural/type reload/drift handling, a verified-constraint lifecycle and a federation caller are not. This register keeps the rationale; the binding decisions and current truth live in 0006/0007/0038.

## Considered Options

* **Cost-driven OBDA** — partly baked in (IRI-template lifting → ADR-0007, cross-source semi-join cost → ADR-0006); shape cardinality oracle and JUCQ-cover factoring stay *pursue*, gated by `sf-bench`.
* **Full-text search** — pursue when a search requirement appears; start from the narrow current PostgreSQL string-predicate pushdown and add a jena-text-style `text:search` property function.
* **Fast term-generation** — reframed as an allocation discipline (baked into ADR-0006); SIMD stays profile-gated rather than the load-bearing technique.
* **Caching** — a binding- and constraint-authority-scoped plan/rewrite cache is implemented (ADR-0007); current serving uses only a constraint-quarantined schema, while digest-addressed reload invalidation remains open. Result cache stays out-of-engine via HTTP validators to preserve the bounded-memory invariant.
* **GeoSPARQL / spatial** — defer but stay design-ready (PostGIS push-down), reserving the extension-function registry and `geo:wktLiteral` datatype seams; no evidenced spatial requirement yet.
* **External `SERVICE` federation** — out of scope (stays ADR-0002): it is the one feature that cannot honour the bounded-memory/governance invariant and has no spec pressure.

## Decision Outcome

### Dispositions

| # | Lever | Disposition | Effort | Gating signal |
|---|---|---|---|---|
| 1 | **Cost-driven OBDA** | **Partly baked in** — IRI-template lifting → ADR-0007, cross-source semi-join cost → ADR-0006; rest (shape cardinality oracle, JUCQ) stays *pursue* | S–H (staged) | foundational half landed; remainder gated by `sf-bench` |
| 2 | **Full-text search** | **Pursue when a search requirement appears** (baseline near-free) | XS→M | a query/UX needs FTS |
| 3 | **Fast term-gen** (reframed: *allocation*, not SIMD) | **Baked in → ADR-0006** (allocation discipline); SIMD stays profile-gated | S–M | landed with the term-gen spec |
| 4 | **Caching** | **Binding/policy-scoped plan cache and unverified-constraint quarantine implemented → ADR-0007**; snapshot digests/reload invalidation open · Result-cache: out-of-engine | S→L | lifecycle work plus any deployment need |
| 5 | **GeoSPARQL / spatial** | **Defer, design-ready** | ~2–4 wk when built | a spatial source/query appears |
| 6 | **External `SERVICE` federation** | **Out of scope** (stays ADR-0002) | 5–7 wk if ever | a cross-endpoint requirement + an ADR superseding 0002 |

### 1. Cost-driven OBDA — pursue

The RDBMS optimizer is **not** a safety net across *equivalent* SPARQL→SQL translations: cross-translation estimates differ by orders of magnitude, it is blind to joins over IRI-template-constructed strings, and on the cross-source path it isn't there at all (Lanti/Xiao/Calvanese, ISWC 2017). Our cost decisions:

- **Single-source — IRI-template / term-construction lifting** (M, high): keep `concat`/`cast` term-building *out* of join & filter predicates; join on raw key columns, build RDF terms only in the final projection — restores index use *and* repairs the DB's own estimates (ADR-0007 already notes "databases cannot see through IRI templates"). **→ Baked into ADR-0007** (*Term-construction lifting*, a translation discipline in the base translation).
- **Cross-source — semi-join cost** (S–M, high): side selection (ship the smaller side's keys), reducer form/sizing (`IN`-list vs temp-table vs Bloom by distinct-key estimate), and a "skip the reducer if reduction ≈ 1" gate. **Closes ADR-0006's named weak point**; needs only the catalog read ADR-0006 already mandates. **→ Baked into ADR-0006** (*Cross-source semi-join cost*).
- **Cardinality oracle:** an `EXPLAIN (FORMAT JSON)` row-count probe for *leaf sub-patterns* (reuse the DB's full estimator, one cached round-trip) — **never** to compare two whole equivalent translations (the unreliable case); catalog stats + HLL/Bloom sketches for cross-source distinct counts.
- Build a thin **`sf-cost`** (shape costs in `sf-sparql`, semi-join costs in `sf-sql`); existing deps only; DataFusion cost code is reference-only (barred on relational crates). **Defer** JUCQ-cover factoring until `sf-bench` shows UCQ blow-up; **skip** ML cardinality (non-deterministic; fights the ADR-0012 oracle).
- Guard: cost may pick only among **equivalent** plans — the NoREC differential (ADR-0012) enforces `=_bag`. **Self-join elimination stays constraint-gated, not cost-gated** (ADR-0007).

### 2. Full-text search — pursue when needed

No standard SPARQL FTS construct exists → it's a vendor extension. Adopt a **jena-text-style `text:search` property function that names the property in its args** — this lets the planner resolve the column via M and push a source-side FTS predicate **without materialising the literal** (honours bounded-memory). Push per dialect: PG `to_tsvector @@ websearch_to_tsquery` + `ts_rank_cd` (+ `pg_trgm` fuzzy mode), MySQL `MATCH…AGAINST`, SQLite FTS5 (best-effort/CI). **Current baseline, not FTS:** PostgreSQL-only case-sensitive `LIKE` for `CONTAINS`/`STRSTARTS`/`STRENDS` on raw column-backed literals, plus `~`/`~*` for regex; SQLite/MySQL reject these pushdowns and no `ILIKE` lowering exists. **⟨T⟩ label search:** `spareval` substring (zero deps) → `fst` (prefix/fuzzy typeahead) → `tantivy` only if BM25 is ever needed; route the same `text:search` predicate to SQL-or-in-memory by whether the property is M-mapped. *Seam to reserve:* an extension-predicate registry + an FTS-index registry (sidecar config — M is authored upstream). Query string is always a bound parameter.

### 3. Fast term-generation — pursue allocation wins (SIMD is a misframe)

**→ Baked into ADR-0006** (*Term generation — allocation discipline*); the detail below is the rationale.

The hot path's dominant cost is **small-object allocation, not byte-level SIMD** — and `portable_simd` is nightly-only (we pin stable 1.96), so SIMD means the runtime-detected stable crates only, profile-gated. Pursue the real wins:
- **Precompute constant terms once** (predicate/type/datatype IRIs, template literal segments) and emit via **`NamedNodeRef`** (zero-copy); use **`NamedNode::new_unchecked`** for template-constructed IRIs (skip RFC-3987 re-validation).
- A **`generate_into(&mut String)` / visitor** term API to kill the per-call owned `Term`/`String` (verify `sparesults` accepts borrowed terms first; else the SELECT path keeps one forced alloc, CONSTRUCT still wins).
- Precompile `rr:template` to a segment list (no per-row placeholder scan); a fast global allocator (mimalloc/jemalloc); `lasso` for the *mapping IR* symbol table (parse-time) — **never** for per-row data (append-only → unbounded → breaks the invariant; at most a small fixed-size LRU for proven low-cardinality columns).
- **Datatype formatting stays on `oxsdatatypes`** (hand-written XSD-canonical) — **do not** use `ryu` (shortest-round-trip ≠ XSD-canonical → conformance bug). SIMD (`simdutf8` over raw column bytes; nibble-table percent-encode classification) only if profiling shows it; typical OBDA keys are short clean PKs needing no escaping.

### 4. Caching — plan-cache early, result-cache out-of-engine

- **Plan/rewrite cache (implemented first stage):** one immutable compiler binding owns mapping, dialect, T-box, compiler-safe schema, explicit constraint authority and `quick_cache`; the exact scope plus full canonical algebra and structural hash prevent cross-binding/policy reuse. Current serving strips PK, UNIQUE, FK, functional-dependency and NOT-NULL claims while retaining names, types and estimates. The desired data/schema-constant split, digest-addressed `⟨T,M,schema,capability,policy⟩` generation, structural/type drift detection, verified-constraint lease and atomic reload remain open. PostgreSQL uses native client preparation; no `deadpool` prepared-statement cache is claimed. **→ ADR-0007** (*Performance*).
- **T-mapping / saturation cache (S–M):** saturation logic exists, but it is not materialized once at startup: the flat path expands a non-empty T-box on each cache miss and the tree path consults it during each compile. Move that work into an immutable startup snapshot with a per-predicate unfold index, then formalise atomic replacement on the same generation boundary; defer disk persistence until startup is *measured* slow. Constraint-based offline branch pruning remains disabled for unverified serving schemas.
- **Result cache (last; out-of-engine):** an in-engine result store **violates the bounded-memory invariant** (instance data in RAM) and the live-freshness guarantee. Recommend the endpoint emit **HTTP validators** instead — `ETag = hash(epoch ⊕ touched-table version watermarks)`, `Cache-Control: max-age` from a declared staleness SLA — and delegate byte-storage to an external cache/proxy; the engine stores zero instance data. An in-engine `moka` cache (TTL + byte-weigher + table-dependency invalidation) only if a deployment demands it: opt-in, default-off, result-size-capped.

### 5. GeoSPARQL / spatial — defer, design-ready

GeoSPARQL→PostGIS push-down is plausible because the product owns SQL emission, FILTER pushdown and datatype handling, and **Ontop-spatial** (JWS 2019) is the direct precedent. There is still **zero evidenced spatial requirement** and no extension-function registry, FTS-index registry or `geo:wktLiteral` hook is implemented. A future binding ADR must first add and test those seams, then keep geometry native and emit `ST_AsText` only at the result boundary; non-spatial sources return `501` rather than using an in-engine `geos`/`geo` fallback.

### 6. External `SERVICE` federation — out of scope

Querying a remote SPARQL endpoint mid-query is the one feature that **cannot honour the bounded-memory/governance invariant** (the remote is uncontrolled — no SQL push-down, no `transaction_timeout`, no guaranteed streaming), it is orthogonal to OBDA-over-our-own-sources (our cross-RDBMS semi-join is a different thing), and there is **no spec pressure** (SPARQL 1.2 Federated Query is a maintenance-only update). `spargebra` parses `SERVICE`; we do not execute it (`501`; ADR-0002/0007). If a concrete cross-endpoint requirement ever appears, prefer **caching the endpoint locally** over live `SERVICE`, and record a new ADR superseding the ADR-0002 deferral (design = a `reqwest` SPARQL-protocol client + `sparesults` ingestion + a block **bind-join**, ~5–7 wk; outbound calls add an SSRF/credential threat class needing an endpoint allow-list).

### Consequences

* Good, because the non-blocking SOTA surface is recorded with dispositions + design seams; nothing is forgotten, and deferred items have reserved seams so they bolt on without rework.
* Good, because two items (cost-driven cross-source semi-join, the plan cache) are high-ROI and already implied by ADR-0006/0007, so they land naturally with the engine.
* Neutral, because most items are profile- or requirement-gated; this is a roadmap, not a build order beyond "rewriter core first."

### Confirmation

* Each *pursue* item graduates to a binding ADR when built. The four foundational items live in ADR-0006/0007; deferred extension-function, `geo:wktLiteral` and FTS-index seams are not current scaffolding and must be introduced with their own executable contract before use.

## More Information

* **Touches:** ADR-0006 (execution/semi-join — cost #1, term-gen #3), ADR-0007 (cascade — cost, plan cache and narrow PostgreSQL string-predicate pushdown), ADR-0008 (defer posture), ADR-0010 (the bounded-memory invariant result-caching must not break), ADR-0011 (cache metrics/config), ADR-0015 (datatype hooks for FTS score / `geo:wktLiteral`).
* **Research (2026-06-27, deep-research campaign):** the original synthesis + six per-thread deliverables are **not retrievable** (no file on disk; absent from this project's AgentDB). A **records-only index** is now stored in the upstream design corpus' memory index (namespace `semantic-fabric`, key `sota-outstanding-synthesis-2026-06-27`) — retrievable via memory search; this ADR is the authoritative home. Key external refs — Lanti/Xiao/Calvanese ISWC 2017 (cost-driven OBDA), CostFed (federation cost), Ontop-spatial JWS 2019 (GeoSPARQL), Apache `jena-text` (FTS surface), `oxsdatatypes` (XSD-canonical formatting).
