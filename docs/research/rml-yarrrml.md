# RML Ecosystem: Forward-Compat Mapping IR Design

**Research key:** `rml-yarrrml`
**Date:** 2026-06-26
**Scope:** Modular RML spec (W3C KGC CG), YARRRML surface syntax, kg-construct test cases, and the IR design constraints that let semantic-fabric's relational engine extend cleanly to heterogeneous sources.

---

## 1. Why RML-Readiness Matters for semantic-fabric

semantic-fabric targets R2RML over RDBMS only. But the mapping IR should stay RML-ready so that heterogeneous sources (CSV, JSON, XML) can be bolted on later without redesigning the core. The key insight is: **R2RML is a strict subset of RML-Core**. Any IR that faithfully models RML-Core (LogicalSource + pluggable reference formulation + explicit iterator) can represent R2RML as a special case — not the reverse.

---

## 2. The Modular RML Spec (W3C KGC CG)

### 2.1 Background and Status

The W3C [Knowledge Graph Construction Community Group](https://www.w3.org/groups/cg/kg-construct/) redesigned RML into a set of six composable modules. The definitive paper is [The RML Ontology: A Community-Driven Modular Redesign After a Decade of Experience in Mapping Heterogeneous Data to RDF](https://link.springer.com/chapter/10.1007/978-3-031-47243-5_9) (ISWC 2023). As of this writing all modules are published as **Draft Community Group Reports dated 16 March 2026**; RML-Core and RML-STAR have also been adopted as Final Community Group Reports. The shared namespace is `http://w3id.org/rml/` (prefix `rml:`). Resources live under the [kg-construct GitHub organisation](https://github.com/kg-construct).

| Module | What it adds | Spec URL |
|--------|-------------|----------|
| **RML-Core** | Canonical data model: TriplesMap, LogicalSource, TermMap hierarchy, Join | https://kg-construct.github.io/rml-core/spec/docs/ |
| **RML-IO** | Concrete source/target types, reference formulations, encoding, compression, named targets | https://kg-construct.github.io/rml-io/spec/docs/ |
| **RML-CC** | Collections and containers (rdf:List/Bag/Seq/Alt) via GatherMap | https://kg-construct.github.io/rml-cc/spec/docs/ |
| **RML-FNML** | Declarative data-transformation functions via FnO | https://kg-construct.github.io/rml-fnml/spec/docs/ |
| **RML-STAR** | RDF-star graph generation | https://kg-construct.github.io/rml-star/spec/docs/ |
| **RML-LV** | Logical Views — format-agnostic pre-processing layer with cross-source joins | https://kg-construct.github.io/rml-lv/spec/docs/ |

A seventh module, **RML-IO-Registry**, was introduced in the 2025 KGCW Challenge to standardise how new source/target drivers are registered.

### 2.2 RML-Core: The Canonical Data Model

The RML-Core spec defines the abstract classes and properties that all other modules build on. The namespace IRI is `http://w3id.org/rml/`.

**Class hierarchy:**

```
rml:AbstractLogicalSource
  └── rml:LogicalSource

rml:TermMap
  ├── rml:SubjectMap
  ├── rml:PredicateMap
  ├── rml:ObjectMap
  │     └── rml:RefObjectMap   (join to another TriplesMap)
  └── rml:GraphMap

rml:TriplesMap
rml:PredicateObjectMap
rml:Join                        (join condition)
```

**Key properties:**

| Property | Domain → Range | Purpose |
|----------|---------------|---------|
| `rml:logicalSource` | TriplesMap → LogicalSource | Binds a map to its data source |
| `rml:source` | LogicalSource → IRI | URI of the actual data source |
| `rml:referenceFormulation` | LogicalSource → ReferenceFormulation | Grammar for extracting values |
| `rml:iterator` | LogicalSource → literal | XPath/JSONPath/SQL expression selecting iterable elements |
| `rml:reference` | TermMap → literal | Expression evaluated against the reference formulation |
| `rml:subjectMap` | TriplesMap → SubjectMap | Subject generation |
| `rml:predicateObjectMap` | TriplesMap → PredicateObjectMap | PO-pair |
| `rml:object/objectMap` | PredicateObjectMap → ObjectMap | Object generation |
| `rml:joinCondition` | RefObjectMap → Join | Equi-join condition |
| `rml:childMap` / `rml:parentMap` | Join | Child and parent expression maps |
| `rml:graphMap` | SubjectMap or POM → GraphMap | Named graph assignment |
| `rml:termType` | TermMap | IRI, BlankNode, Literal |
| `rml:datatype` / `rml:language` | ObjectMap | Literal typing |

Three mechanisms generate a term in any TermMap: `rr:constant` (unchanged literal/IRI), `rr:template` (string template with `{reference}` slots), or `rml:reference` (direct expression evaluation). These three are inherited unchanged from R2RML.

### 2.3 How RML Generalises R2RML

RML is explicitly **a superset of R2RML** ([rml.io comparison](https://rml.io/docs/rml/rmlvsr2rml/)):

| R2RML concept | RML-Core generalisation | Notes |
|---------------|------------------------|-------|
| `rr:logicalTable` (implicit row iteration) | `rml:logicalSource` + `rml:iterator` (explicit) | In SQL, iterator = optional; defaults to row-level |
| `rr:tableName` / `rr:sqlQuery` | `rml:source` (IRI) + reference formulation `rml:SQL2008` | rr:tableName and rr:sqlQuery remain valid inside the SQL-specific sub-class |
| `rr:column` | `rml:reference` | Column name is a valid SQL2008 reference expression |
| Implicit CSV/JSON/XML support | `rml:referenceFormulation` selects `rml:JSONPath`, `rml:XPath`, `rml:CSV` | Pluggable grammar |

Any R2RML document is a valid RML-Core document — the processor can read it unchanged. An IR that stores an abstract `LogicalSource { source: IRI, referenceFormulation: ReferenceFormulation, iterator: Option<Expr> }` can represent an R2RML SQL source by setting `referenceFormulation = rml:SQL2008` and populating the SQL-specific sub-fields.

A formal algebraic treatment of RML semantics was published as [An Algebraic Foundation for Knowledge Graph Construction (extended version)](https://arxiv.org/abs/2503.10385) (Oo and Hartig, ESWC 2025). They show that a language-agnostic algebra of composable operators covers the full RML expressivity and enables provably correct optimisation rewrites — a useful foundation for a Rust query planner.

### 2.4 RML-IO: Concrete Sources and Targets

RML-IO ([spec](https://kg-construct.github.io/rml-io/spec/docs/)) fills in the abstract `rml:LogicalSource` and adds `rml:LogicalTarget`. For semantic-fabric's relational mode the relevant reference formulation class is `rml:SQL2008`. RML-IO also introduces:

- **Encoding**: `rml:encoding` (UTF-8, UTF-16) on sources
- **Compression**: gzip / zip / tarxz on targets
- **Null handling**: `rml:null` property listing values treated as SQL NULL
- **Term-level targeting**: each TermMap can declare its own `rml:logicalTarget`, enabling per-component output routing

The per-term target is the most IR-impactful feature: it transforms the execution model from single-output to multi-output, requiring triple routing logic in the materializer.

### 2.5 RML-CC: Collections and Containers

[RML-CC](https://kg-construct.github.io/rml-cc/spec/docs/) introduces `rml:GatherMap`, a TermMap subclass that collects results from multiple child TermMaps into an RDF collection or container. Key properties:

- `rml:gather` → ordered list of TermMaps (possibly nested GatherMaps)
- `rml:gatherAs` → target type (rdf:List, rdf:Bag, rdf:Seq, rdf:Alt)
- `rml:strategy` → `rml:append` (default) or `rml:cartesianProduct`
- `rml:allowEmptyListAndContainer` → boolean gate on empty output

An IR needs a `GatherMap` node type with a list-of-TermMap children and a collection-type tag.

### 2.6 RML-FNML: Functions via FnO

[RML-FNML](https://kg-construct.github.io/rml-fnml/spec/docs/) integrates the [Function Ontology (FnO)](https://fno.io/rml/) for declarative, implementation-independent data transformations. The key classes:

- `rml:FunctionMap` — a TermMap that calls an FnO function
- `rml:FunctionExecution` — binds concrete input values to function parameters
- `rml:Input` / `rml:ParameterMap` — connects source data (via TermMaps) to FnO parameters
- `rml:ReturnMap` — selects which function output to use when a function returns multiple values

An RML-FNML module for Python UDFs in [Morph-KGC](https://github.com/morph-kgc/morph-kgc) was published in [ScienceDirect 2024](https://www.sciencedirect.com/science/article/pii/S2352711024000803), demonstrating YARRRML-to-RML-FNML translation on the fly. For semantic-fabric the IR needs a `FunctionExecution` node with a function IRI, an ordered list of (parameter IRI, TermMap) pairs, and a return-map selector — even if FNML execution is deferred to a later milestone.

### 2.7 RML-LV: Logical Views

[RML-LV](https://kg-construct.github.io/rml-lv/spec/docs/) addresses three limitations of RML-Core for nested data: Cartesian-product blowup on hierarchical sources, single-formulation-per-source constraints, and join placement restrictions. It introduces:

- `rml:LogicalView` — a format-agnostic tabular abstraction over one or more LogicalSources
- `rml:Field` (subclasses: `rml:ExpressionField`, `rml:IterableField`) — named columns in the view
- `rml:LogicalViewJoin` — left and inner joins between LogicalViews
- Structural annotations (Primary Key, Foreign Key, Unique, NotNull) enabling optimiser hints

LogicalViews expose a key-value row model; TriplesMap `rml:reference` expressions address named fields rather than raw source paths. This is the module that would make semantic-fabric's join push-down/SQL rewriting work uniformly across formats — important for OBDA virtualisation mode.

---

## 3. YARRRML — Human-Readable Surface Syntax

[YARRRML](https://rml.io/yarrrml/spec/) is a YAML dialect that serialises RML mappings in a human-friendly form. The reference parser is [@rmlio/yarrrml-parser v1.12.2](https://github.com/RMLio/yarrrml-parser) (MIT, October 2025), converting YARRRML → RML (default) or R2RML (`-f R2RML`). The YARRRML repo is maintained by the [kg-construct organisation](https://github.com/kg-construct/yarrrml).

### 3.1 YAML Structure

```yaml
prefixes:
  ex: "http://example.com/"

sources:
  db-source:
    type: postgresql
    access: "localhost:5432/mydb"
    referenceFormulation: sql2008
    queryFormulation: "SELECT * FROM person"

mappings:
  persons:
    sources: [db-source]
    subjects: ex:person/$(id)
    predicateobjects:
      - [ex:name, $(name)~xsd:string]
      - [ex:age, $(age)~xsd:integer]
```

Top-level keys: `base`, `prefixes`, `sources`, `targets`, `mappings`. Template values use `$(ref)` syntax. Language/datatype suffixes are appended with `~`. Functions are nested objects under `mapping`. Conditions gate triple generation. Named graphs appear as a `graphs:` key on a mapping.

### 3.2 Profile System

YARRRML supports multiple semantic profiles: R2RML (SQL), RML (heterogeneous), RMLT (adds targets), and extension vocabularies (FnO, CSVW, D2RQ, DCAT, VoID). semantic-fabric can accept YARRRML as an authoring surface by running @rmlio/yarrrml-parser in a pre-processing step to produce Turtle/RML-Core for internal ingestion.

### 3.3 Tooling Ecosystem

| Tool | Purpose | URL |
|------|---------|-----|
| yarrrml-parser | YARRRML → RML/R2RML CLI + library | https://github.com/RMLio/yarrrml-parser |
| Matey | Browser-based YARRRML editor with live preview | https://rml.io/yarrrml/matey/ |
| GRAPE | Projectional editor for RML authoring | https://ceur-ws.org/Vol-3999/paper3.pdf |
| linkml YARRRML generator | Generate YARRRML from LinkML schemas | https://linkml.io/linkml/generators/yarrrml.html |

---

## 4. kg-construct Test Cases

### 4.1 Repository Status

The monolithic [kg-construct/rml-test-cases](https://github.com/kg-construct/rml-test-cases) repo is **deprecated and archived as of March 2026**. Test cases are now published per module under each module's own repository (e.g., `kg-construct/rml-core/test-cases/`). The canonical index is at [https://w3id.org/rml/portal](https://kg-construct.github.io/rml-resources/portal/).

### 4.2 Module Coverage

| Module | Test case URL pattern | Notes |
|--------|----------------------|-------|
| RML-Core | `http://w3id.org/rml/core/test-cases` | Covers TriplesMap, TermMap types, joins, named graphs, blank nodes, multiple sources |
| RML-IO | `http://w3id.org/rml/io/test-cases` | CSV, JSONPath, XPath, SQL2008 reference formulations; encoding; targets |
| RML-IO-Registry | (new in 2025) | Driver registration for custom source types |
| RML-CC | `http://w3id.org/rml/cc/test-cases` | GatherMap, collection types, strategies |
| RML-FNML | `http://w3id.org/rml/fnml/test-cases` | FunctionExecution patterns |
| RML-STAR | `http://w3id.org/rml/star/test-cases` | RDF-star quoted triples |
| RML-LV | `http://w3id.org/rml/lv/test-cases` | LogicalView, Field, join patterns (new in 2025) |

The KGCW 2024 and [KGCW 2025 Challenges](https://kg-construct.github.io/workshop/2025/challenge.html) (both at ESWC) ran conformance and performance tracks against these test cases. For the 2024 track, published pass rates include: **RMLMapper: 98.70% RML-Core, 50.75% RML-IO, 73.70% overall** ([KGCW 2024 dataset](https://zenodo.org/records/11577087)). Morph-KGC and others were also evaluated.

### 4.3 Test Case Structure

Each test case folder contains:
- A data source file (`input.*` — SQL dump, CSV, JSON, XML)
- `mapping.ttl` — the RML mapping in Turtle
- `output.nq` or `output.ttl` — expected RDF (absent if an error is expected)

For SQL-backed tests the input is a set of SQL `CREATE TABLE` + `INSERT` statements. semantic-fabric must load these into a live SQLite/PostgreSQL/MySQL instance (following the R2RML test harness model) to evaluate RML-IO+Core SQL test cases.

---

## 5. What the Mapping IR Must Accommodate

Designing the IR now for RML-Core readiness costs almost nothing for the relational engine but prevents a rewrite when heterogeneous sources arrive.

### 5.1 Mandatory IR Fields (relational engine, today)

These are all required today to pass the W3C R2RML test suite and can be represented directly in RML-Core terms:

```
TriplesMap {
  id: IRI,
  logical_source: LogicalSource,
  subject_map: TermMap,
  predicate_object_maps: Vec<PredicateObjectMap>,
  graph_maps: Vec<TermMap>,          // rml:graphMap on TriplesMap or POM
}

LogicalSource {
  source: SourceDescriptor,          // SQL2008 sub-type today
  reference_formulation: IRI,        // rml:SQL2008 | rml:JSONPath | rml:XPath | rml:CSV
  iterator: Option<String>,          // None = implicit row iteration (SQL)
}

SourceDescriptor::Sql {             // rml:SQL2008 concretisation
  table_name: Option<String>,
  sql_query: Option<String>,
  sql_version: SqlVersion,          // SQL2008
}

TermMap {
  kind: TermMapKind,                // Constant | Template | Reference
  value: String,
  term_type: TermType,              // IRI | BlankNode | Literal
  datatype: Option<IRI>,
  language: Option<LangTag>,
}

RefObjectMap {
  parent_triples_map: IRI,
  join_conditions: Vec<Join>,
}

Join {
  child: TermMap,                   // child (current source) expression
  parent: TermMap,                  // parent (referenced source) expression
}
```

### 5.2 Extensibility Slots Needed for RML-Forward-Compat

Add these as tagged enum variants or trait objects — they can be stubs initially, but must be REPRESENTABLE in the IR without schema changes:

| Slot | RML module | What to reserve |
|------|-----------|-----------------|
| `LogicalSource::View(LogicalView)` | RML-LV | LogicalView node with Fields and join list |
| `TermMap::Function(FunctionExecution)` | RML-FNML | function IRI + Vec<(param, TermMap)> + return selector |
| `ObjectMap::Gather(GatherMap)` | RML-CC | collect: Vec<TermMap>, gather_as: CollectionType, strategy: Strategy |
| `LogicalTarget` | RML-IO | per-TriplesMap or per-TermMap output routing |
| `referenceFormulation` | RML-IO | NOT hardcoded to SQL — store as IRI, dispatch on value |

### 5.3 Reference Formulation as a First-Class Plugin Point

The single most important IR decision is that `reference_formulation` must be an **open IRI** (not an enum with hardcoded variants). This is how RML-IO is designed: processors iterate over elements using the expression grammar specified by the formulation. For semantic-fabric:

- Today: `rml:SQL2008` → SQL column reference, implicit row iteration
- Tomorrow: `rml:JSONPath` → `$.items[*]` iterator, `$.name` reference
- Tomorrow: `rml:XPath` → `/persons/person` iterator, `@name` reference
- Tomorrow: `rml:CSV` → row iterator, column-name reference

Representing this as a Rust trait (`trait ReferenceFormulation { fn evaluate(&self, data, expr) -> Values }`) with a registry keyed on IRI achieves the right abstraction.

### 5.4 Iterator Explicitness

R2RML has implicit row-level iteration; RML-Core makes iteration explicit via `rml:iterator`. For SQL, the iterator is either absent (whole-table) or a SQL `WHERE`/subquery. Store `iterator: Option<String>` now; for non-relational sources the iterator will carry a JSONPath or XPath expression selecting the top-level elements.

### 5.5 Source as URI, Not Table Name

In R2RML, `rr:tableName "EMP"` is a string. In RML-Core, `rml:source <http://example.com/db/EMP>` is an IRI. The IR should store the source as a URI (possibly a local opaque one for SQL tables), not as a bare string in the TriplesMap. This keeps joins across heterogeneous sources (RML-LV) structurally representable later.

---

## 6. YARRRML Integration Path for semantic-fabric

semantic-fabric does not need to implement a YARRRML parser in Rust. The recommended path:

1. Users author mappings in YARRRML (optional convenience layer)
2. @rmlio/yarrrml-parser (Node.js) transpiles to Turtle/RML-Core at build/load time
3. semantic-fabric ingests Turtle and parses it into its internal IR via the [oxttl](https://docs.rs/oxttl/) crate (already in Oxigraph's dependency tree)
4. The IR is then compiled to a query plan for SQL rewriting or materialisation

This matches how Morph-KGC handles YARRRML: translate on-the-fly before execution.

---

## 7. Open Questions

1. **RML-LV and OBDA virtualisation**: LogicalViews expose a tabular abstraction that is almost identical to the "virtual" relational view needed for SPARQL-to-SQL rewriting. It may be possible to unify the OBDA query planner and the RML-LV evaluation engine. Worth investigating whether the RML-LV algebra (field, left-join, inner-join) maps cleanly onto Oxigraph's SPARQL algebra.

2. **FNML execution in Rust**: FnO functions are currently bound to Python (Morph-KGC) or JVM (RMLMapper). A Rust FNML implementation would need either WASM UDFs or FFI. No Rust FNML engine was found in this survey.

3. **RML-IO-Registry spec maturity**: The Registry module (driver registration) was introduced in the KGCW 2025 challenge but has no published spec as of this writing. Monitor `kg-construct/rml-io-registry` for stabilisation before implementing the source-plugin interface.

4. **Algebraic optimiser**: The Oo/Hartig algebra (arXiv 2503.10385) covers RML v1.1.2; it is unclear whether it also covers the full modular RML-Core v0.2.0 (March 2026). A Rust query planner should validate against the newer spec.

5. **Pass-rate targets**: The KGCW 2024 results give RMLMapper at 98.70% on RML-Core as the current SOTA. semantic-fabric should target 100% on RML-Core and the W3C R2RML suite before claiming SOTA.

---

## 8. Sources

| Title | URL | Evidence quality |
|-------|-----|-----------------|
| RML-Core spec (Draft CG Report, March 2026) | https://kg-construct.github.io/rml-core/spec/docs/ | High |
| RML-IO spec | https://kg-construct.github.io/rml-io/spec/docs/ | High |
| RML-CC spec | https://kg-construct.github.io/rml-cc/spec/docs/ | High |
| RML-FNML spec | https://kg-construct.github.io/rml-fnml/spec/docs/ | High |
| RML-LV spec | https://kg-construct.github.io/rml-lv/spec/docs/ | High |
| rml.io original spec | https://rml.io/specs/rml/ | High |
| RML vs R2RML comparison | https://rml.io/docs/rml/rmlvsr2rml/ | High |
| YARRRML spec | https://rml.io/yarrrml/spec/ | High |
| yarrrml-parser GitHub | https://github.com/RMLio/yarrrml-parser | High |
| kg-construct/yarrrml | https://github.com/kg-construct/yarrrml | Medium |
| kg-construct/rml-test-cases (deprecated) | https://github.com/kg-construct/rml-test-cases | High |
| RML portal (per-module test cases) | https://kg-construct.github.io/rml-resources/portal/ | High |
| KGCW 2025 challenge | https://kg-construct.github.io/workshop/2025/challenge.html | High |
| KGCW 2024 dataset (Zenodo) | https://zenodo.org/records/11577087 | Medium |
| The RML Ontology: ISWC 2023 (Springer) | https://link.springer.com/chapter/10.1007/978-3-031-47243-5_9 | High |
| Algebraic Foundation for KGC (arXiv 2503.10385) | https://arxiv.org/abs/2503.10385 | Medium |
| Morph-KGC RML-FNML Python module (ScienceDirect) | https://www.sciencedirect.com/science/article/pii/S2352711024000803 | Medium |
| W3C KGC Community Group | https://www.w3.org/groups/cg/kg-construct/ | High |
| kg-construct GitHub org | https://github.com/kg-construct | High |
| awesome-kgc-tools | https://kg-construct.github.io/awesome-kgc-tools/ | Medium |
