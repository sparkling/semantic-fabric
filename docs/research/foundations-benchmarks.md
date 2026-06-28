# Foundations and Benchmarks for semantic-fabric

**Research area:** SPARQL-to-SQL theory, KG-construction engine landscape, GTFS-Madrid-Bench and KROWN benchmark SOTA numbers  
**Date:** 2026-06-26  
**Scope:** R2RML/RML over RDBMS only; heterogeneous sources (CSV/JSON/XML) are noted for context but are out of scope for the initial build.

---

## 1. SPARQL-to-SQL Foundations

### 1.1 Chebotko, Lu & Fotouhi (2009) — Semantics-Preserving Translation

The foundational formal treatment of the translation problem is:

> A. Chebotko, S. Lu, F. Fotouhi. **Semantics preserving SPARQL-to-SQL translation.**  
> *Data & Knowledge Engineering*, 68(10):973–1000, 2009.  
> [ScienceDirect abstract](https://www.sciencedirect.com/science/article/abs/pii/S0169023X09000469)  
> [ResearchGate PDF](https://www.researchgate.net/publication/222694291_Semantics_preserving_SPARQL-to-SQL_translation)

**Key contribution.** The paper formalizes a relational-algebra semantics for SPARQL and proves that it is equivalent to the standard mapping-based semantics of Pérez et al. (2006). From this equivalence it derives a provably sound and complete translation from SPARQL to SQL covering:

- Triple patterns and Basic Graph Patterns (BGPs) via cross-product + selection
- OPTIONAL patterns via outer-joins
- FILTER expressions via SQL WHERE predicates

The paper is the first to offer a soundness and completeness proof rather than empirical validation alone, making it the theoretical bedrock for any R2RML-over-RDBMS engine. A companion short paper at ACM IDEAS 2009 targets efficiency:

> **A complete translation from SPARQL into efficient SQL.**  
> [ACM DL](https://dl.acm.org/doi/10.1145/1620432.1620437)

**Relevance for semantic-fabric.** The relational-algebra correspondence underpins both the materialization path (execute the SQL then serialize RDF) and the virtualization path (rewrite SPARQL to SQL then stream). The outer-join handling for OPTIONAL is the hardest correctness surface — Chebotko et al. provide the proof target.

### 1.2 Ultrawrap — SQL-View-Based Virtualization (Sequeda & Miranker)

> J. Sequeda, D. Miranker. **Ultrawrap: SPARQL Execution on Relational Data.**  
> *Web Semantics: Science, Services and Agents on the World Wide Web*, 2013.  
> [Web Semantics](https://www.websemanticsjournal.org/index_php/ps/article/view/344)  
> [ScienceDirect abstract](https://www.sciencedirect.com/science/article/abs/pii/S1570826813000383)  
> [SSRN preprint](https://www.ssrn.com/abstract=3199073)

**Key technique.** Rather than hand-coding a SPARQL-to-SQL rewriter, Ultrawrap compiles R2RML (or D2RQ) mappings into a set of SQL views that encode an RDF-graph representation of the relational data. A simple syntactic SPARQL-to-SQL translation then operates over those views. The critical insight is that the underlying SQL optimizer — not the semantic-web layer — performs the hard work: self-join elimination, unsatisfiable condition pruning, and view unfolding reduce the naive translation to something that executes at near-native SQL speed.

The follow-on paper introduces bidirectional OBDA and is the first to support transitivity via SQL `WITH RECURSIVE`:

> J. Sequeda, M. Arenas et al. **OBDA: Query Rewriting or Materialization? In Practice, Both!**  
> *ISWC 2014*, Springer, pp. 535–551.  
> [Springer](https://link.springer.com/chapter/10.1007/978-3-319-11964-9_34)  
> [Semantic Scholar](https://www.semanticscholar.org/paper/OBDA:-Query-Rewriting-or-Materialization-In-Both!-Sequeda-Arenas/dd2c7a7da566f5d5e482095156eb5eee685d58c8)

**Relevance for semantic-fabric.** The SQL-view architecture is the cleanest way to share the mapping compilation step between materialization and virtualization — the same compiled view layer is either streamed into a triple writer (materialization) or used as the target for a SPARQL-to-SQL rewrite (virtualization). This is precisely the "one shared core" model semantic-fabric targets.

### 1.3 Ontop — Industrial OBDA System (Calvanese et al.)

> D. Calvanese, B. Cogrel, S. Komla-Ebri, R. Kontchakov, D. Lanti, M. Rezk, M. Rodriguez-Muro, G. Xiao.  
> **Ontop: Answering SPARQL Queries over Relational Databases.**  
> *Semantic Web Journal*, 2016 (submitted Dec 2015). DOI 10.3233/SW-160217.  
> [SWJ page](https://www.semantic-web-journal.net/content/ontop-answering-sparql-queries-over-relational-databases-1)  
> [SWJ PDF (swj1278)](https://www.semantic-web-journal.net/system/files/swj1278.pdf)

**Key technique — five-phase pipeline:**

1. **Tree-witness rewriting.** SPARQL query → Union of Conjunctive Queries (UCQ) by "unfolding" OWL 2 QL ontological entailments. Handles class-hierarchy and role-hierarchy reasoning.
2. **T-mapping compilation.** R2RML mappings are extended with the ontology's entailments to produce T-mappings — an extended set of mappings that subsumes both base mappings and ontological inferences. Redundant T-mappings are removed via inclusion dependency reasoning.
3. **Perfect reformulation.** The UCQ is merged with the T-mappings to produce a single SQL expression (often a large union of SQL queries).
4. **SQL optimization.** Self-join elimination, projection pushdown, and other standard SQL optimizations are applied to the generated SQL.
5. **Execution and result translation.** The SQL is executed against the target RDBMS; result rows are translated back to SPARQL bindings.

**Correctness.** Ontop guarantees sound and complete SPARQL evaluation under the OWL 2 QL profile (first-order rewritable fragment). For R2RML-only scenarios (no ontology), it degenerates to a complete SPARQL-to-SQL rewriter — the same completeness proof as Chebotko et al. applies.

**Database support.** PostgreSQL, MySQL, MariaDB, SQL Server, Oracle, DB2, DuckDB, Snowflake, BigQuery, Redshift — making it the widest-coverage OBDA system in production use.

**Key performance challenge.** The worst-case exponential blow-up of the tree-witness rewriting step (exponential in the number of joins in the query) is the primary scalability bottleneck. Ontop mitigates this with T-mapping redundancy elimination and caching but does not fully solve it — a fundamental open problem.

**Published performance data.** On the Berlin SPARQL Benchmark (BSBM) at 200 million triples, Ontop delivers more than 400,000 queries per minute through a single database engine. This is the key comparison point for virtualization mode: semantic-fabric should target at least this throughput on PostgreSQL.

**Evaluating SPARQL-to-SQL in Ontop (early results):**

> *Evaluating SPARQL-to-SQL Translation in Ontop.*  
> [CEUR Vol-1015/paper_16](https://ceur-ws.org/Vol-1015/paper_16.pdf)

---

## 2. Algebraic Foundation for KGC (2025)

The most recent foundational paper — winner of Best Research Paper at ESWC 2025:

> S. Min Oo, O. Hartig. **An Algebraic Foundation for Knowledge Graph Construction** (Extended Version).  
> *22nd ESWC*, June 2025. arXiv:2503.10385.  
> [arXiv abstract](https://arxiv.org/abs/2503.10385)  
> [Springer](https://link.springer.com/chapter/10.1007/978-3-031-94575-5_1)

**Key contributions:**

- Introduces a **language-agnostic algebra** for KGC mapping definitions (analogous to relational algebra for SQL).
- Provides a translation algorithm (Algorithm 1) from RML into this algebra, giving RML its first formally grounded semantics.
- Proves a set of **algebraic rewriting rules** enabling provably correct optimization of mapping plans (e.g., operator reordering, join elimination).
- Does not contain performance benchmarks — this is a purely theoretical paper.

**Relevance for semantic-fabric.** The algebra gives the correct IR for a dual-mode engine: the same algebraic representation of a mapping can be evaluated eagerly (materialization) or lazily with query folding (virtualization). The proven rewriting rules can be applied in a query planner. The implementation should define an IR that is isomorphic to this algebra.

---

## 3. KG-Construction Engine Landscape

### 3.1 Overview Survey

> J. Arenas-Guerrero et al. **Knowledge Graph Construction with R2RML and RML: An ETL System-based Overview.**  
> *KGCW Workshop, ESWC 2021.* CEUR Vol-2873.  
> [CEUR PDF](https://ceur-ws.org/Vol-2873/paper11.pdf)  
> [Semantic Scholar](https://www.semanticscholar.org/paper/Knowledge-Graph-Construction-with-R2RML-and-RML:-An-Arenas-Guerrero-Iglesias-Molina/7747127c5cfbdd746c6bd1414b955dc989ae7d22)

The main materialization engines as of 2021–2024:

| Engine | Language | Approach | R2RML | RML | SPARQL rewriting |
|---|---|---|---|---|---|
| **RMLMapper** | Java | Reference, in-memory | Yes | Yes | No |
| **SDM-RDFizer** | Python | Streaming with partitioned reads | Yes | Yes | No |
| **Morph-KGC** | Python | Mapping partitions + pandas | Yes | Yes | No |
| **RMLStreamer** | Scala/Spark | Distributed streaming | No | Yes | No |
| **Ontop** | Java | Virtual OBDA (+ materialization mode) | Yes | No (R2RML only natively) | Yes |
| **FlexRML** | C++ | Flexible, multi-threaded | Yes | Partial | No |

**No Rust-native R2RML/RML engine exists in the public literature.** semantic-fabric fills this gap entirely.

A live index of available tools is maintained at:  
[https://kg-construct.github.io/awesome-kgc-tools/](https://kg-construct.github.io/awesome-kgc-tools/)

### 3.2 Morph-KGC — Current Materialization SOTA

> J. Arenas-Guerrero, D. Chaves-Fraga, J. Toledo, M. S. Pérez, O. Corcho.  
> **Morph-KGC: Scalable knowledge graph materialization with mapping partitions.**  
> *Semantic Web*, 15(1):1–20, 2024. DOI 10.3233/SW-223135.  
> [SWJ page](https://www.semantic-web-journal.net/content/morph-kgc-scalable-knowledge-graph-materialization-mapping-partitions-1)  
> [ResearchGate](https://www.researchgate.net/publication/362982745_Morph-KGC_Scalable_knowledge_graph_materialization_with_mapping_partitions)

**Core technique.** Mapping partitions are groups of TriplesMap rules that generate *disjoint* subsets of the output KG. Because the subsets do not overlap, each partition can be materialized independently and memory freed between partitions. Combined with parallel execution (multiple partitions in parallel), this is the key innovation that lets Morph-KGC scale beyond the memory ceiling of engines that hold the entire KG in RAM to deduplicate.

**Benchmarks used.** GTFS-Madrid-Bench (RDB + tabular), SDM-Genomic-Datasets, NPD (Norwegian Petroleum Directorate dataset).

**Result summary.** Morph-KGC with parallel partitions outperforms all other engines in execution time for most GTFS-Madrid-Bench scenarios. At GTFS scale factor 100 (~35 million triples), Morph-KGC without partitions runs out of memory; with partitions it completes where all other engines either time out or OOM. PostgreSQL connectivity gives better performance than MySQL at large scale factors.

---

## 4. Benchmarks

### 4.1 W3C R2RML Conformance Test Suite

> B. Villazón-Terrazas, M. Hausenblas. **R2RML and Direct Mapping Test Cases.**  
> W3C Note, 14 August 2012.  
> [https://www.w3.org/TR/2012/NOTE-rdb2rdf-test-cases-20120814/](https://www.w3.org/TR/2012/NOTE-rdb2rdf-test-cases-20120814/)

**Structure.** 63 test cases across 26 database scenarios (D000–D025). Each case specifies:
- SQL schema + seed data
- R2RML mapping document (Turtle)
- Expected output (N-Quads with base IRI `http://example.com/base/`)
- Reference to the R2RML spec section being tested

**Features covered.** URI template expansion; blank node subject maps; literal generation (plain, typed, language-tagged); SQL data type coercion (integer, decimal, float, date, timestamp, boolean, binary/hexBinary); physical tables vs. logical tables (SQL query sources); primary key and composite key handling; foreign-key join conditions via referencing object maps; named graphs and the default graph; malformed mapping validation (negative cases).

**Conformance target for semantic-fabric.** 63/63 pass (100%) is the correctness bar. As of 2012, no engine was validated against all cases; subsequent R2RML implementations (Ontop, R2RML-F, RMLMapper) claim high but not always complete conformance. Achieving 63/63 plus the RML conformance test cases is the correctness gate before any benchmark run.

**RML conformance suite:**

> P. Heyvaert et al. **Conformance Test Cases for the RDF Mapping Language (RML).**  
> *ESWC 2019*, Springer LNCS 11503, pp. 203–217.  
> [Springer](https://link.springer.com/chapter/10.1007/978-3-030-21395-4_12)

### 4.2 GTFS-Madrid-Bench

> D. Chaves-Fraga, F. Priyatna, A. Cimmino, J. Toledo, E. Ruckhaus, O. Corcho.  
> **GTFS-Madrid-Bench: A benchmark for virtual knowledge graph access in the transport domain.**  
> *Journal of Web Semantics*, 65:100596, 2020. DOI 10.1016/j.websem.2020.100596.  
> [ScienceDirect](https://www.sciencedirect.com/science/article/pii/S1570826820300354)  
> [Semantic Scholar](https://www.semanticscholar.org/paper/9ba87b5231b167949a22127ad2de21709b2304b4)  
> [GitHub](https://github.com/oeg-upm/gtfs-bench)

**What it measures.** End-to-end knowledge graph construction and virtual query answering over the Madrid subway GTFS dataset (stops, routes, trips, stop times, shapes, calendars, agencies, etc.).

**Scaling knobs.**
- *Vertical scale*: number of records per table — 10K, 100K, 1M, 10M rows. Scale factor 100 produces roughly **35 million triples** from the RDB source.
- *Horizontal scale*: data format — CSV, JSON, XML, SQL (RDB), MongoDB
- *Query workload*: 15 SPARQL queries reflecting real user questions (simple lookup → multi-hop, aggregation)
- *Mapping languages*: R2RML (RDB), RML (CSV/JSON/XML/RDB), xR2RML (MongoDB), CSVW

**Metrics collected.** Total execution time (seconds); peak memory consumption (GB); initial delay (time to first result for streaming); dief@k and dief@t (throughput diefficiency metrics for continuous/streaming evaluation).

**SOTA results (materialization, RDB track).**  
Absolute triples/second figures are reported in PDF figures not available in plain-text form, but the ordering is well established in the literature:

| Engine | Speed rank | Memory rank | Notes |
|---|---|---|---|
| Morph-KGC (parallel+partitions) | 1st (fastest) | 3rd (higher peak) | Scales to 10M rows; OOM without partitions |
| SDM-RDFizer | 2nd | 1st (lowest) | Slower but lowest peak memory |
| Ontop (materialization mode) | 2nd–3rd | 2nd | Stable memory; best for virtualization |
| RMLMapper | Last | Last | Times out at 1M+ rows; reference implementation only |

**SOTA target for semantic-fabric (RDB materialization).** Beat Morph-KGC's execution time at scale factor ≥100 (35M+ triples) while staying within 2× its peak memory footprint.

**SOTA target for semantic-fabric (virtualization).** Beat Ontop's SPARQL query latency (the 400K+ queries/minute figure on BSBM-200M) on the GTFS SPARQL workload.

### 4.3 KROWN

> D. Van Assche, B. De Meester, M. Heyvaert, A. Dimou et al.  
> **KROWN: A Benchmark for RDF Graph Materialisation.**  
> *ISWC 2024*, Springer LNCS 15194.  
> [Springer](https://link.springer.com/chapter/10.1007/978-3-031-77847-6_2)  
> [Zenodo (results)](https://zenodo.org/records/10973892)  
> [GitHub](https://github.com/kg-construct/KROWN)

**What it measures.** Orthogonal impact of four independent dimensions on materialization systems. GTFS-Madrid-Bench mixes all dimensions simultaneously; KROWN varies one at a time to isolate root causes of degradation.

**Four scaling dimensions:**

| Dimension | Parameters |
|---|---|
| **Data size** | Rows: 10K–10M; Columns: 1–30; Cell size: 500B–10KB |
| **Data quality** | Duplicates: 0–100 %; Empty values: 0–100 % |
| **Mapping complexity** | TriplesMap count: 1–30; Predicate-Object Maps: 1–10; Named Graph Maps: 1–15 |
| **Join complexity** | Relation type: 1:N, N:1, N:M; Join conditions: 1–15; Join duplicates: 0–15 |

**Hardware.** Ubuntu 22.04 LTS, Intel Xeon E5-2650 v2 @ 2.60GHz, 48 GB RAM, 2 GB swap. Each scenario run 5 times; median reported.

**Engines.** RMLMapper, RMLStreamer, Morph-KGC, SDM-RDFizer, OntopM (Ontop materialization mode).

**Key findings:**

- **Data size scaling.** Morph-KGC is fastest across all data-size scenarios. OntopM shows the most stable memory growth (does not load data into JVM heap the same way); RMLMapper and SDM-RDFizer OOM at large cell sizes (10KB) and high column counts.
- **Data quality.** Morph-KGC is fastest for high duplicate/empty-value scenarios. RMLMapper times out.
- **Named graph scaling.** SDM-RDFizer fails (error) with multiple named graph maps. RMLMapper times out. All systems fail or time out at 15 named graphs in the dynamic case.
- **Join complexity.** RMLMapper times out at 5, 10, and 15 join conditions. RMLStreamer does not support multiple join conditions. SDM-RDFizer degrades significantly on N:M joins with high duplicate counts.

**SOTA target for semantic-fabric (KROWN).** Complete all KROWN scenarios without timeout or OOM on the same hardware class. No existing engine does; this is the correctness+completeness ceiling.

---

## 5. Gaps and Implications for semantic-fabric

### No Rust engine exists

The literature search found zero Rust-native R2RML or RML materialization/virtualization engines. The closest is [Oxigraph](https://github.com/oxigraph/oxigraph) (Rust SPARQL graph database on RocksDB), which can serve as the SPARQL evaluation substrate for the virtual query path. For materialization output, Oxigraph's N-Triples/Turtle serializers can be used directly.

Oxigraph's self-reported performance characteristics: point queries from warm cache in ~0.8 µs; complex SPARQL in ~0.5 ms; 100M triples at ~2.1 GB RAM. Query evaluation is explicitly noted as "not yet optimized," meaning semantic-fabric's generated SQL will hit the RDBMS directly — bypassing Oxigraph's SPARQL engine for the virtualization path — which is the architecturally correct approach.

### Worst-case exponential blow-up

The fundamental theoretical challenge for all SPARQL-to-SQL rewriters (Ontop, Ultrawrap, semantic-fabric's virtual path) is the worst-case exponential blow-up from tree-witness rewriting when the SPARQL query has many joins. Chebotko et al. noted this; Ontop mitigates it with T-mapping redundancy elimination. For semantic-fabric (R2RML only, no OWL reasoning), the rewriting is simpler — it is a direct R2RML-to-SQL unfolding without ontology expansion — so the blow-up is bounded by the number of TriplesMap references in the query, not by OWL entailment depth.

### Shared IR is the design key

The Oo & Hartig (2025) algebra paper confirms that materialization and virtualization can share a single IR rooted in the same algebra. The semantic-fabric design should define an `MappingPlan` type in Rust that is isomorphic to this algebra and can be either:
- evaluated eagerly (SQL executed → rows → RDF terms → Turtle/N-Triples output) — materialization
- folded into a SPARQL query (SPARQL → MappingPlan → SQL) — virtualization

### Benchmark target summary

| Benchmark | Mode | Current SOTA | Target |
|---|---|---|---|
| W3C R2RML test suite (63 cases) | Materialization | ~58–62/63 (Ontop/RMLMapper) | 63/63 |
| GTFS scale 100 (~35M triples) | Materialization | Morph-KGC (minutes, exact tps in PDF figures) | Beat Morph-KGC wall-clock |
| GTFS 15 SPARQL queries | Virtualization | Ontop (400K+ qpm on BSBM-200M) | Match or beat on GTFS workload |
| KROWN (all 4 dimensions) | Materialization | No engine completes all | Complete all without OOM/timeout |

---

## Sources

- [Semantics preserving SPARQL-to-SQL translation (Chebotko et al. 2009)](https://www.sciencedirect.com/science/article/abs/pii/S0169023X09000469)
- [A complete translation from SPARQL into efficient SQL (ACM IDEAS 2009)](https://dl.acm.org/doi/10.1145/1620432.1620437)
- [Ultrawrap: SPARQL Execution on Relational Data (Web Semantics 2013)](https://www.sciencedirect.com/science/article/abs/pii/S1570826813000383)
- [OBDA: Query Rewriting or Materialization? In Practice, Both! (ISWC 2014)](https://link.springer.com/chapter/10.1007/978-3-319-11964-9_34)
- [Ontop: Answering SPARQL Queries over Relational Databases (SWJ 2016)](https://www.semantic-web-journal.net/content/ontop-answering-sparql-queries-over-relational-databases-1)
- [An Algebraic Foundation for Knowledge Graph Construction (ESWC 2025)](https://arxiv.org/abs/2503.10385)
- [Knowledge Graph Construction with R2RML and RML: An ETL System-based Overview (KGCW 2021)](https://ceur-ws.org/Vol-2873/paper11.pdf)
- [Morph-KGC: Scalable knowledge graph materialization with mapping partitions (SWJ 2024)](https://doi.org/10.3233/SW-223135)
- [Conformance Test Cases for the RDF Mapping Language (RML) (ESWC 2019)](https://link.springer.com/chapter/10.1007/978-3-030-21395-4_12)
- [R2RML and Direct Mapping Test Cases (W3C Note 2012)](https://www.w3.org/TR/2012/NOTE-rdb2rdf-test-cases-20120814/)
- [GTFS-Madrid-Bench (JWS 2020)](https://www.sciencedirect.com/science/article/pii/S1570826820300354)
- [GTFS-Madrid-Bench GitHub](https://github.com/oeg-upm/gtfs-bench)
- [KROWN: A Benchmark for RDF Graph Materialisation (ISWC 2024)](https://link.springer.com/chapter/10.1007/978-3-031-77847-6_2)
- [KROWN GitHub](https://github.com/kg-construct/KROWN)
- [KROWN results Zenodo](https://zenodo.org/records/10973892)
- [Evaluating SPARQL-to-SQL Translation in Ontop](https://ceur-ws.org/Vol-1015/paper_16.pdf)
- [awesome-kgc-tools](https://kg-construct.github.io/awesome-kgc-tools/)
- [Oxigraph GitHub](https://github.com/oxigraph/oxigraph)
- [Scaling Up KGC to Large and Heterogeneous Data Sources (arXiv 2022)](https://arxiv.org/abs/2201.09694)
