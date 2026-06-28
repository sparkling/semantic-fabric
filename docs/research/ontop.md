# Prior Art: Ontop — SPARQL-to-SQL Rewriting for Virtual Knowledge Graphs

**Topic:** Ontop OBDA/VKG system — prior art for semantic-fabric's virtualization mode
**Key references:** [SWJ 2016/2017 paper](https://journals.sagepub.com/doi/10.3233/SW-160217) · [ISWC 2020 system paper](https://research.bcgl.fr/pdfs/ontop-iswc20.pdf) · [ontop-vkg.org](https://ontop-vkg.org/guide/) · [GitHub](https://github.com/ontop/ontop)
**Date of research:** 2026-06-26

---

## 1. What Ontop Is

Ontop is the canonical open-source implementation of Ontology-Based Data Access (OBDA) / Virtual Knowledge Graph (VKG) systems. Its core claim: translate any SPARQL 1.1 query expressed over a virtual RDF graph — defined by R2RML mappings from relational tables — into an equivalent SQL query, execute it, and return RDF results. No triples are ever materialized unless the user explicitly requests it. The project originated at the Free University of Bozen-Bolzano; the foundational [Semantic Web Journal paper](https://journals.sagepub.com/doi/10.3233/SW-160217) (Calvanese et al., 2016/2017) won the SWJ Outstanding Paper Award. A second [system paper at ISWC 2020](https://research.bcgl.fr/pdfs/ontop-iswc20.pdf) described the architecture after the rewrite to the IQ-based engine.

Ontop is Java (87% of code), built on RDF4J and OWLAPI, uses JDBC for database connectivity, and requires Java 11+.

---

## 2. The SPARQL-to-SQL Pipeline

The query answering process has two temporal stages — **offline** (at startup) and **online** (per query) — and five algorithmic phases:

### 2.1 Offline: T-Mapping Construction

1. **Ontology classification.** The OWL 2 QL TBox is classified using a lightweight DL-Lite reasoner to compute the complete class and property hierarchies (subclass/subproperty, domain/range).
2. **T-mapping construction.** User-supplied R2RML mappings are compiled with the ontology hierarchy into a new set of mappings called *T-mappings* (triple-pattern mappings). A T-mapping `m` for class `C` absorbs all mappings for every subclass of `C` as a UNION, so querying `C` at runtime does not require consulting the ontology at all. The result is a *saturated* mapping set: it exposes a virtually saturated ABox.
3. **T-mapping optimization.** Redundant UNION branches are pruned using SQL's expressivity and DB integrity constraints. PK/FK constraints on the relational schema allow the optimizer to drop whole branches that are guaranteed duplicates or that produce empty results.

This offline saturation means the ontology reasoning cost is paid once, not per query.

### 2.2 Online: Per-Query Rewriting

4. **SPARQL → IQ (tree-witness + unfolding).** The input SPARQL query is parsed into an *Intermediate Query* (IQ). If tree-witness rewriting is enabled, OWL 2 QL entailment is folded in here (see §4). The IQ is then unfolded against the T-mappings: intensional data nodes (triple patterns) are replaced by the SQL sub-expressions from the matching T-mapping entries. Since Ontop 5.4.0, this unfolding is a two-phase operation — first expanding abstract predicates, then resolving to concrete table references — for better handling of complex SPARQL patterns.
5. **IQ optimization.** The flat unfolded IQ is optimized through a cascade of structural and semantic passes (see §5).
6. **SQL generation.** The optimized IQ is translated to a dialect-specific SQL string via a per-DBMS code generator. The SQL is sent to the JDBC-connected RDBMS.
7. **Result translation.** SQL result rows are converted back to SPARQL variable bindings or RDF triples.

---

## 3. Intermediate Query (IQ): The Internal Representation

The IQ is a DBMS-independent algebraic tree representation that unifies SPARQL algebra and relational algebra with named attributes. Introduced in Ontop 4.x, it replaced the older Datalog-based internal representation. Full formal specification: [dev internals page](https://ontop-vkg.org/dev/internals/iq) and [formal characterization](https://ontop-vkg.org/research/iq-formal.html).

**Leaf nodes:**
- *Intensional data nodes* — placeholders for triple/quad patterns, resolved during unfolding.
- *Extensional data nodes* — concrete DB table references with sparse column mappings.
- *Empty node* — identity for union, absorbing element for join.
- *True node* — zero-ary non-empty relation; identity for natural join.
- *Native nodes* — generated during final SQL translation; hold a raw SQL string.
- *Values nodes* — embedded table literals.

**Non-leaf nodes:** inner join (N-ary natural join + optional boolean conditions), left join (binary, maps to SPARQL OPTIONAL), union, filter, construction (extended projection supporting IRI template expansion), aggregation, distinct, order-by, slice.

The IQ algebra is designed so that SPARQL OPTIONAL maps cleanly to left outer join with null-safe semantics, and SPARQL triple patterns map to data nodes that expand to table scans + filter on the predicate column.

---

## 4. Ontology Entailment: Tree-Witness Rewriting

For OWL 2 QL ontologies, Ontop can rewrite SPARQL queries under the entailment regime so that answers implied by the TBox (not just the ABox) are returned. The mechanism is the **tree-witness rewriting algorithm** from the 2014 ISWC paper [Kontchakov et al.](https://link.springer.com/chapter/10.1007/978-3-319-11964-9_35). It generates a UCQ (union of conjunctive queries) over the original predicates that encodes all entailed answers, using "tree-witness" structures to represent existential derivations from domain/range axioms.

In practice this is **switched off by default** in Ontop. The reason: SPARQL under OWL 2 QL has difficulty expressing existentially quantified variables (unlike plain CQs), so tree-witness patterns are extremely rare in real-world SPARQL queries. The ontology entailment that matters practically — subclass/subproperty hierarchy, rdf:type inference, domain/range for property hierarchy — is already handled entirely by T-mappings without tree-witness rewriting.

Implication for semantic-fabric: **the OWL 2 QL entailment regime is a significant complexity layer that is rarely exercised in practice.** An initial implementation can omit tree-witness rewriting and still cover nearly all real workloads.

---

## 5. Structural and Semantic Optimizations

This is where Ontop delivers most of its practical performance. The IQ optimization cascade (applied after unfolding) includes:

### 5.1 Structural Optimizations

**Self-join elimination.** When the unfolded IQ contains an inner join of a table with itself (the same T-mapping entry referenced twice on the same key variable), and the join condition reduces to a PK equality, the duplicate table reference is eliminated. This is pervasive because unfolding a BGP with multiple triple patterns using the same subject IRI template often produces self-joins.

**Self-left-join elimination with nullable determinants (v5.2.0).** Extended self-join elimination to left joins where the join key may be nullable. When the determinant (join key) is null, the CASE WHEN is used to preserve null semantics. This was made robust to provenance variables and trivial functional dependencies in subsequent releases.

**IRI template mismatch pruning.** If two triple patterns are joined on a variable bound to an IRI, and the IRI templates in the two T-mapping entries use different URI prefixes that can never coincide, the entire join branch is pruned as empty. This is one of Ontop's most impactful optimizations: a SPARQL BGP that joins on rdf:type may unfold to dozens of UNION branches, and template-mismatch pruning eliminates most of them.

**Redundant union elimination.** Union branches that produce identical triples (same template, same table, subsumable WHERE clauses) are merged.

**Distinct removal.** Unnecessary DISTINCT operators below joins or with limit conditions are removed.

**Selection pushdown.** Filter conditions are propagated down the IQ tree (to DNF, then conjunct-by-conjunct), so predicates are applied as early as possible, reducing intermediate result sizes.

### 5.2 Semantic (Constraint-based) Optimizations

From the paper [OBDA Constraints for Effective Query Answering](https://arxiv.org/pdf/1605.04263) and extended in subsequent releases:

**PK-based join elimination.** If a join is on a PK column and the joined table's remaining columns are not projected, the join is eliminated.

**FK-based join elimination.** A join on a FK→PK relationship where the FK side is non-nullable and the PK table's non-key columns are not needed can be simplified by dropping the PK table reference.

**Functional dependency inference (v5.3.0).** Ontop computes transitive closures of functional dependencies declared on the schema (or on "lenses"). This powers stronger self-join detection and distinct removal.

**Constraint sources.** Constraints come from (a) the DB schema via JDBC metadata, or (b) Ontop "lenses" — virtual views where constraints are declared manually in the mapping layer, allowing constraint-based optimization even on complex subqueries or views.

---

## 6. R2RML Support

Ontop is the leading open-source R2RML implementation. The W3C [R2RML specification](https://www.w3.org/TR/r2rml/) and its [100+ test cases](https://www.w3.org/TR/rdb2rdf-test-cases/) covering 25 DB scenarios are the conformance benchmark (PKs, FKs, data types, NULLs, blank nodes, named graphs, RefObjectMaps, SQL queries as logical tables). Ontop reaches near-full compliance as of v4.2.0+, with documented exceptions:

- Base IRIs: not supported.
- Default mapping generation (Direct Mapping): not implemented.
- Binary SQL datatype normalization: not supported.
- Complex SQL logical tables (GROUP BY, subqueries): datatype inference requires enabling `ontop.allowRetrievingBlackBoxViewMetadataFromDB`.
- `rr:inverseExpression` and BNode in the Ontop native mapping format: not supported.

Ontop also ships its own `.obda` mapping format (fully interoperable with R2RML), which is the default in Protégé integration. Both are compiled to the same T-mapping internal representation.

---

## 7. SPARQL 1.1 Compliance

As of Ontop 5.4.0, the [compliance page](https://ontop-vkg.org/guide/compliance) documents:

**Supported:** BGP, FILTER, OPTIONAL, UNION, MINUS, FILTER EXISTS/NOT EXISTS, BIND, VALUES, aggregates (COUNT/SUM/MIN/MAX/AVG/GROUP_CONCAT/SAMPLE), subqueries, FROM/FROM NAMED/GRAPH, ORDER BY, SELECT/DISTINCT/REDUCED/OFFSET/LIMIT, SELECT/CONSTRUCT/ASK/DESCRIBE, string functions, numeric functions, XPath constructors, GeoSPARQL (non-topological + most topological).

**Partially supported:** Property paths (5 of 8 types: PredicatePath, InversePath, SequencePath, AlternativePath, NegatedPropertySet — ZeroOrMore/OneOrMore/ZeroOrOne not supported), date/time functions (8 of 9), RDF term functions (11 of 13).

**Not supported:** SERVICE (federated queries), user-defined functions, full EXISTS (bottom-up evaluation only). Hash functions and REGEX/REPLACE work only on DBMSs that provide them natively.

---

## 8. SQL Dialect Handling

Ontop uses a dialect abstraction layer where each DBMS has a code generator that produces conformant SQL. Supported dialects as of 2025:

**Traditional RDBMS:** PostgreSQL, MySQL, MariaDB, Oracle, SQL Server, DB2, H2.
**Cloud/analytical:** Snowflake, DuckDB, Google BigQuery, AWS Redshift, AWS Athena, Trino, PrestoDB, Databricks, Apache Spark, Dremio, Denodo, Teiid, MonetDB, SAP HANA.
**Emerging:** TDengine (v5.4.0), AWS DynamoDB (via CData connector).

Version 5 included a "scaffolding tool" to reduce the friction of adding new dialects. The main dialect variation points are: string concatenation syntax, IRI casting, regex operators, date/time arithmetic, UUID generation, NULL handling in aggregates, and BOOLEAN representation.

---

## 9. Materialization Mode

Ontop ships an `ontop materialize` CLI command that runs the full mapping against the DB and streams RDF triples to a file (N-Triples, Turtle, JSON-LD since v5.2.0, with compression support). This is a scan-all query: `SELECT * WHERE { ?s ?p ?o }` unfolded to a UNION over all T-mapping entries.

In [KROWN benchmark](https://kg-construct.github.io/KROWN/) evaluations (materialization-focused, used at ESWC KG Construction Challenge 2023/2024), Ontop competes with RMLMapper, Morph-KGC, SDM-RDFizer, and RMLStreamer. Ontop's materialization mode is slower than purpose-built materializers (Morph-KGC in particular) because the SPARQL-to-SQL rewriting path is not optimized for bulk export; it pays the overhead of the full translation pipeline per query.

Ontopic (the commercial spin-off) introduced a hybrid materialization feature: materialize selected portions of the KG into a graph database, leaving the rest virtual.

---

## 10. Architecture Summary and Current Limitations

**Architecture (Java, ~600k+ LoC):**
- `ontop-model` / `ontop-core-model` — ontology and mapping data structures
- `ontop-reformulation-core` — T-mapping construction, IQ optimizer, SPARQL rewriter
- `ontop-sql-core` — SQL dialect layer, schema metadata extraction
- `ontop-protege` — Protégé plugin
- `ontop-cli` / `ontop-endpoint` — REST SPARQL endpoint (Jetty)
- RDF4J (v5.1.0 as of Ontop 5.3.0) for RDF/SPARQL parsing
- OWLAPI (v5.5.1) for OWL 2 QL ontology loading

**Known limitations and pain points:**
- Issue [#800](https://github.com/ontop/ontop/issues/800): certain SPARQL-to-SQL translations for N-column result sets generate N-1 self-joins or left-joins, leading to extremely slow queries. A known class of unoptimized patterns.
- No incremental materialization (full rematerialization only).
- Property path types ZeroOrMore, OneOrMore, ZeroOrOne are not supported — these require recursive SQL (CTEs) which Ontop does not currently generate.
- SERVICE (federated SPARQL) is not supported — Ontop is single-RDBMS per endpoint.
- No support for heterogeneous sources (CSV/JSON/XML) without a federator layer in front.

---

## 11. Benchmark Targets

**GTFS-Madrid-Bench:** [ScienceDirect paper](https://www.sciencedirect.com/science/article/pii/S1570826820300354). 18 complex + 11 simple SPARQL 1.1 queries over Madrid subway GTFS data, scaled at 1×/5×/10×/50×/100×/500×, 1-hour timeout. Ontop is one of the primary reference systems. Query response times in real deployments range from ~100ms for simple BGPs to ~10s for complex joins at scale.

**KROWN:** [Results on Zenodo](https://zenodo.org/records/10973892). Materialization throughput benchmark. Ontop is included; purpose-built materializers (Morph-KGC) tend to outperform it in pure bulk export, but Ontop remains competitive at smaller scales.

**W3C RDB2RDF test suite:** [Specification](https://www.w3.org/TR/rdb2rdf-test-cases/) — 100+ test cases, SQL 2008 RDBMS. Ontop near-full pass. This is the minimum correctness target for semantic-fabric.

---

## 12. What Is Hardest to Reproduce in Rust

Ranked by difficulty and subtlety:

1. **T-mapping saturation with OWL 2 QL reasoning.** Requires a correct DL-Lite classifier (or OWL 2 QL reasoner) to compute the class/property hierarchy, then fold axioms into mappings as UNION expansions. Oxigraph does not currently include a DL reasoner; this would need to be built or the ontology layer deferred (as semantic-fabric intends for the first phase per ADR-0020 scope).

2. **IQ optimization cascade — specifically self-join elimination and its interaction with nullable functional dependencies.** The cascade of passes is order-sensitive: IRI template mismatch pruning must run before self-join elimination; functional dependency inference must precede join elimination. Getting the interaction right across diverse mapping patterns took Ontop multiple major versions.

3. **Tree-witness rewriting for OWL 2 QL entailment.** Computationally expensive and theoretically complex. Practically skippable for v1 (off by default in Ontop too), but required for full SPARQL entailment compliance.

4. **SQL dialect abstraction with correctness parity.** Each of the ~20 supported dialects has quirks in NULL handling, string functions, type casting, and aggregate semantics. The scaffolding approach Ontop uses (dialect-specific code generators) is the right pattern; `sqlparser-rs` in Rust can serve as the AST layer.

5. **Two-phase unfolding correctness.** The separation between intensional (mapping) and extensional (table) phases is subtle for queries with OPTIONAL, UNION, and nested subqueries. Getting null semantics correct across the join types is the hardest single correctness challenge.

6. **R2RML RefObjectMap join conditions.** Parent/child triples maps with `rr:joinCondition` generate SQL JOINs between parent and child logical tables. Combined with IRI templates, this creates join patterns that interact with all the optimization passes.

---

## 13. What to Reuse vs. Reimplement for semantic-fabric (Rust/Oxigraph)

**Architecture to mirror:**
- The offline/online split (T-mapping saturation offline, per-query unfolding online) is the right design and should be preserved.
- The IQ as a tree-based algebra bridging SPARQL and relational algebra is the right internal representation. An Oxigraph-native variant using its SPARQL algebra types as starting point would work.
- The dialect abstraction layer pattern is proven; implement it from the start.

**Rust crates relevant:**
- `oxigraph` — SPARQL parsing, evaluation, RDF types.
- `sqlparser` (Apache-licensed, `sqlparser-rs` crate) — SQL AST for the output SQL generation layer.
- `oxrdf` — RDF terms, used by Oxigraph.
- For R2RML parsing: no production-ready Rust crate exists; implement against the [W3C spec](https://www.w3.org/TR/r2rml/) directly using `oxttl` or `oxrdflib`.

**Reimplement from scratch (do not port Java):**
- The T-mapping construction logic (the Java version is deeply entangled with OWLAPI structures).
- The IQ optimizer passes — use the Ontop SWJ + ISWC 2020 papers as the algorithmic specification.
- The constraint extraction from JDBC metadata — replace with native DB schema introspection per dialect.

**Defer or descope for v1:**
- Tree-witness rewriting (OWL 2 QL entailment) — confirms ADR-0020 scope.
- Property path recursion (ZeroOrMore etc.) — needs recursive SQL (CTEs), non-trivial.
- Full OWL 2 QL reasoning — start with RDFS hierarchy only.

---

## Key Publications

| Year | Venue | Title | URL |
|------|-------|-------|-----|
| 2015 | J. Web Semantics | Efficient SPARQL-to-SQL with R2RML mappings (Rodriguez-Muro & Rezk) | [ScienceDirect](https://www.sciencedirect.com/science/article/abs/pii/S1570826815000153) |
| 2016/2017 | Semantic Web Journal | Ontop: Answering SPARQL Queries over Relational Databases | [SWJ](https://journals.sagepub.com/doi/10.3233/SW-160217) |
| 2017 | ISWC | Cost-Driven Ontology-Based Data Access | [arXiv](https://arxiv.org/pdf/1707.06974) |
| 2018 | ISWC | Efficient Handling of SPARQL OPTIONAL for OBDA | [ResearchGate](https://www.researchgate.net/publication/329326869_Efficient_handling_of_SPARQL_OPTIONAL_for_OBDA) |
| 2020 | ISWC | The Virtual Knowledge Graph System Ontop | [PDF](https://research.bcgl.fr/pdfs/ontop-iswc20.pdf) |
| 2022 | — | W3C R2RML Test Cases | [W3C](https://www.w3.org/TR/rdb2rdf-test-cases/) |
| 2024 | ESWC/Zenodo | KROWN Benchmark Results | [Zenodo](https://zenodo.org/records/10973892) |
