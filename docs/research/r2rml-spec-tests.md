# R2RML Spec & W3C Conformance Suite

**Research key:** `r2rml-spec-tests`  
**Date:** 2026-06-26  
**Scope:** W3C normative basis (R2RML + Direct Mapping Recommendations) and the RDB2RDF conformance test suite — the correctness gate for semantic-fabric.

---

## 1. The W3C Normative Basis

Two Recommendations were published on **27 September 2012** by the [W3C RDB2RDF Working Group](https://www.w3.org/groups/wg/rdb2rdf/publications/).

| Rec | Short name | URL |
|-----|-----------|-----|
| R2RML: RDB to RDF Mapping Language | `r2rml` | https://www.w3.org/TR/r2rml/ |
| A Direct Mapping of Relational Data to RDF | `rdb-direct-mapping` | https://www.w3.org/TR/rdb-direct-mapping/ |

Both are stable W3C Recommendations (not living standards). They have not been revised since 2012; RML 2.0 (a community group successor) extends them but does not supersede them for the RDBMS-only scope.

---

## 2. R2RML — Core Concepts

R2RML is an RDF vocabulary (prefix `rr:`, namespace `https://www.w3.org/ns/r2rml#`) for expressing customised mappings from a relational database to an RDF dataset. A mapping document is itself a Turtle file.

### 2.1 TriplesMap (§6)

The fundamental unit. Each `rr:TriplesMap` has exactly:
- **one** `rr:logicalTable` — the SQL source
- **one** `rr:subjectMap` — generates the subject RDF term
- **zero or more** `rr:predicateObjectMap` — generates predicate–object pairs

Every row in the logical table's result set is processed once per TriplesMap.

### 2.2 LogicalTable — Base Table vs. R2RML View (§5)

```
rr:logicalTable  [
  rr:tableName "EMP"             # base table or SQL view
]
```

An **R2RML view** replaces `rr:tableName` with `rr:sqlQuery`:

```
rr:logicalTable  [
  rr:sqlQuery  "SELECT id, UPPER(name) AS name FROM EMP WHERE active = 1"
]
```

The SQL query is evaluated by the underlying RDBMS. This is the primary hook for data transformation without altering the source schema. The query must produce a single-character-set result with unique column names; implementations must report a data error for `NULL` in a subject template position.

### 2.3 SubjectMap (§6.1)

Generates the RDF term that becomes the subject of every triple produced by the TriplesMap. A `rr:subjectMap` also carries optional `rr:class` values, which inject `rdf:type` triples automatically.

### 2.4 Term Map Types (§6)

All term maps (`rr:subjectMap`, `rr:predicateMap`, `rr:objectMap`, `rr:graphMap`) share three generation mechanisms:

| Type | Property | Semantics |
|------|----------|-----------|
| **Constant** | `rr:constant` | Fixed RDF term, independent of row |
| **Column** | `rr:column` | Value from a named column; literal unless `rr:termType rr:IRI` |
| **Template** | `rr:template` | String with `{ColumnName}` placeholders; column values are percent-encoded for IRI safety |

`rr:termType` controls whether the result is `rr:IRI`, `rr:BlankNode`, or `rr:Literal`. Blank node subjects are scoped to the logical table and row; identical column values within a run produce the same blank node.

### 2.5 PredicateObjectMap (§6.3)

Pairs one or more predicate maps with one or more object maps (or `rr:refObjectMap`). Multiple maps per POM produce a Cartesian product of triples.

### 2.6 RefObjectMap and Join Conditions (§8)

A referencing object map joins two logical tables to set the object of a triple to the _subject_ of another TriplesMap:

```turtle
rr:predicateObjectMap [
  rr:predicate  ex:worksIn ;
  rr:objectMap  [
    rr:parentTriplesMap  :DeptMap ;
    rr:joinCondition [
      rr:child   "deptId" ;
      rr:parent  "id"
    ]
  ]
] .
```

`rr:joinCondition` is optional when the parent and child logical tables are identical (self-join). Multiple join conditions are ANDed. Joins are evaluated as SQL equi-joins under the hood; this is the primary driver of the SQL complexity a semantic-fabric OBDA engine must generate.

### 2.7 Graph Maps (§9)

`rr:graphMap` on a SubjectMap or PredicateObjectMap places triples into a named graph. `rr:defaultGraph` is a constant value directing triples to the unnamed default graph. Templates and column values can generate graph IRIs dynamically.

### 2.8 Natural Mapping of SQL Datatypes to RDF (§10)

The specification defines the **natural mapping** — the canonical conversion of SQL column values to RDF literals. This is normative and is the ground truth for datatype handling in semantic-fabric's materializer.

| SQL datatype(s) | RDF datatype | Canonical lexical form note |
|---|---|---|
| CHARACTER, CHARACTER VARYING, CLOB, NCHAR, NCHAR VARYING, NCLOB | plain literal (no datatype tag) | value as-is |
| BINARY, BINARY VARYING, BINARY LARGE OBJECT | `xsd:hexBinary` | hex-encoded |
| NUMERIC, DECIMAL | `xsd:decimal` | — |
| SMALLINT, INTEGER, BIGINT | `xsd:integer` | — |
| FLOAT, REAL, DOUBLE PRECISION | `xsd:double` | — |
| BOOLEAN | `xsd:boolean` | must be lowercase `true`/`false` |
| DATE | `xsd:date` | — |
| TIME | `xsd:time` | — |
| TIMESTAMP | `xsd:dateTime` | space separator replaced by `T` |
| INTERVAL | _undefined_ | implementations may cast to string |

The specification distinguishes:
- **Natural RDF lexical form** (SHOULD apply XSD canonical mapping)
- **Canonical RDF lexical form** (MUST apply XSD canonical mapping)

Datatype override via `rr:datatype` produces typed literals; ill-typed pairings (value not in the datatype's value space) are data errors. Custom language tags are set via `rr:language`.

Source: [R2RML §10 — Datatype mapping](https://www.w3.org/TR/r2rml/#natural-mapping); full SQL-to-XSD discussion at [W3C RDB2RDF wiki](https://www.w3.org/2001/sw/rdb2rdf/wiki/SQL_to_XSD_Type_Mappings.html).

---

## 3. Direct Mapping — Key Concepts

The [Direct Mapping](https://www.w3.org/TR/rdb-direct-mapping/) is a fully algorithmic, configuration-free transformation. It is the _default mapping_ — a baseline that R2RML is designed to refine.

### 3.1 Algorithm

**Row nodes:**
- Table has a primary key → row node is an IRI: `<base/TableName/PK1=val1;PK2=val2>`
- No primary key → row node is a blank node (unique per row in the run)

**Column values:**
- Non-NULL → literal triple using property IRI `<base/TableName#ColumnName>`
- Each row also gets `rdf:type <base/TableName>`

**Foreign keys:**
- FK columns generate reference triples with predicate `<base/TableName#ref-ColName>` pointing to the referenced row's node
- Establishes RDF links between entities without any mapping file

### 3.2 Relationship to R2RML

The Direct Mapping is a specialised instance of R2RML: any Direct Mapping output can be expressed as an R2RML mapping. The semantic-fabric OBDA engine should implement Direct Mapping as a code path that auto-generates an R2RML document from an inspected DB schema, then runs the normal R2RML pipeline. This keeps the core unified.

---

## 4. W3C RDB2RDF Test Suite — The Conformance Gate

### 4.1 Official Publication

[R2RML and Direct Mapping Test Cases — W3C Group Note, 14 August 2012](https://www.w3.org/TR/rdb2rdf-test-cases/)

The canonical live test artefacts are served from the W3C Mercurial repository:  
`https://dvcs.w3.org/hg/rdb2rdf-tests/`

The community test harness is at [d2rq/rdb2rdf-harness](https://github.com/d2rq/rdb2rdf-harness) (shell scripts + Jena/ARQ comparison utilities, archived January 2021).

### 4.2 Database Scenarios (D000–D025)

The suite is organised around **26 database scenarios** (D000 through D025), progressing from trivial to complex:

| Range | Focus |
|---|---|
| D000–D005 | Single table, no/composite PK, duplicate rows, nulls |
| D006–D008 | Single/composite primary keys |
| D009–D011 | Multi-table, FK, many-to-many |
| D012–D016 | Null values, datatypes (D016: ten SQL types in one table) |
| D017–D020 | Internationalisation, CHAR, IRI values in columns |
| D021–D025 | Complex FK patterns, self-referencing, non-PK FK references |

### 4.3 Per-Scenario File Layout

```
D009-2tables1primarykey1foreignkey/
├── create.sql        # DDL + INSERT — the relational source
├── manifest.ttl      # Test metadata (Test Metadata vocabulary in RDFa)
├── directGraph.ttl   # Expected Direct Mapping output (Turtle)
├── r2rml.ttl         # R2RML mapping document (one or more per scenario)
└── mapped.nq         # Expected R2RML output (N-Quads, for named graph support)
```

Some scenarios carry multiple R2RML mapping variants (`r2rml-a.ttl`, `r2rml-b.ttl`, …) with correspondingly named expected outputs. This is why individual test-case IDs (e.g. `R2RMLTC0002a` through `R2RMLTC0002i`) outnumber the database scenarios.

### 4.4 Test Case Counts

| Suite | Count |
|---|---|
| R2RML test cases | **~37** (positive + error) |
| Direct Mapping test cases | **~12** |
| **Total** | **~49** |

Source: [TCOverview.html](https://www.w3.org/2001/sw/rdb2rdf/test-cases/TCOverview.html). The W3C NOTE itself says "over 60" in some places, counting sub-variants; the canonical manifest list yields ~49 named test IDs.

**Test types present:**
- _Positive_ tests: valid mapping + expected N-Quads/Turtle output
- _Error_ tests: intentionally invalid mapping documents (e.g. `R2RMLTC0002c` — invalid SQL identifier, `R2RMLTC0002e` — undefined table name, `R2RMLTC0002g` — invalid SQL query, `R2RMLTC0012c` — TriplesMap without SubjectMap). The expected result for an error test is that the processor signals a data/mapping error rather than producing output.

### 4.5 Base IRI Requirement

All tests mandate `<http://example.com/base/>` as the base IRI. A conformant implementation must accept this as a parameter and use it to resolve relative IRI references in both Direct Mapping output and R2RML template expansion.

### 4.6 Expected Output Format

- **Direct Mapping tests** → Turtle (`.ttl`)
- **R2RML tests** → N-Quads (`.nq`) — N-Quads is required because R2RML supports named graphs, which Turtle cannot represent in a single file

Graph-isomorphism comparison (blank-node-aware) is required for evaluation; byte-level string comparison is not sufficient.

### 4.7 Running the Suite

The [d2rq/rdb2rdf-harness](https://github.com/d2rq/rdb2rdf-harness) workflow:
1. Load `create.sql` into the target RDBMS (harness supports MySQL, PostgreSQL; CI can substitute SQLite for basic cases)
2. For each test case, invoke the R2RML processor with the mapping file and the DB connection
3. Capture N-Quads output; compare against `mapped.nq` using RDF graph isomorphism
4. Emit an [EARL](https://www.w3.org/TR/EARL10-Schema/) report (`earl-<toolname>-r2rml.ttl`)

For semantic-fabric, the Rust test harness should drive the engine directly (no shell script); the comparison logic should use Oxigraph's dataset APIs.

### 4.8 EARL Report and Implementation Report

Implementors must produce an EARL RDF document (`earl-<toolname>-r2rml.ttl`) linking each test case IRI to a result (`earl:passed`, `earl:failed`, `earl:cantTell`, `earl:untested`). The aggregate results feed the [RDB2RDF Implementation Report](https://www.w3.org/TR/rdb2rdf-implementations/).

The W3C Implementation Report confirms that at least **two independent implementations passed all tests** across both R2RML and Direct Mapping suites (a prerequisite for W3C Recommendation status). Implementations tested include: D2RQ, morph, Ontop (OpenLink Virtuoso), ultrawrap, db2triples, XSPARQL, RDF-RDB2RDF. None are written in Rust — semantic-fabric would be the first.

---

## 5. Implications for semantic-fabric

### 5.1 Correctness Gate

The 37 R2RML test cases are the minimum bar for a conformant implementation. They are small enough to run on every CI push with an embedded RDBMS (e.g. DuckDB or SQLite for test execution). The error-test cases are especially important for the Rust engine: the type system should make invalid mapping documents unrepresentable at parse time, but SQL-level errors (undefined columns, ambiguous names) surface only at execution time.

### 5.2 Datatype Pipeline

The natural mapping table (§2.8 above) maps almost 1:1 to Rust types: `i64` for `INTEGER`/`BIGINT`, `f64` for `DOUBLE PRECISION`, `rust_decimal::Decimal` for `NUMERIC`/`DECIMAL`, `time::Date/DateTime` for temporal types. The lexical form rules (lowercase booleans, `T` separator in TIMESTAMP) must be hardcoded — they are not left to the database driver.

### 5.3 R2RML View as the Virtualisation Seam

In OBDA mode, an R2RML view (`rr:sqlQuery`) is where the SPARQL-to-SQL rewriter injects its translated SQL. The mapping IR must preserve the distinction between base-table logical tables and R2RML views, because the rewriter will wrap the view's SQL as a subquery.

### 5.4 RefObjectMap = Join in SQL

Every `rr:joinCondition` pair becomes an additional `JOIN … ON child_col = parent_col` clause in the generated SQL (OBDA mode) or a nested iteration (materialisation mode). Composite join conditions (multiple `rr:joinCondition` on one RefObjectMap) are ANDed. This is the main source of SQL complexity the engine must handle.

### 5.5 Test Suite Hosting

The W3C Mercurial repository at `dvcs.w3.org` is still accessible but unmaintained. For CI use, snapshot the test artefacts into the semantic-fabric repository under `tests/w3c/rdb2rdf/` (MIT-compatible W3C document licence). This insulates the test run from network availability.

---

## 6. Sources

| Title | URL | Evidence quality |
|---|---|---|
| R2RML: RDB to RDF Mapping Language (W3C Rec) | https://www.w3.org/TR/r2rml/ | High |
| A Direct Mapping of Relational Data to RDF (W3C Rec) | https://www.w3.org/TR/rdb-direct-mapping/ | High |
| R2RML and Direct Mapping Test Cases (W3C Note) | https://www.w3.org/TR/rdb2rdf-test-cases/ | High |
| RDB2RDF Implementation Report (W3C Note) | https://www.w3.org/TR/rdb2rdf-implementations/ | High |
| Test Case Overview (TCOverview.html) | https://www.w3.org/2001/sw/rdb2rdf/test-cases/TCOverview.html | High |
| Live test artefacts (W3C Mercurial) | https://dvcs.w3.org/hg/rdb2rdf-tests/ | High |
| d2rq/rdb2rdf-harness (GitHub) | https://github.com/d2rq/rdb2rdf-harness | Medium |
| R2RML namespace schema | https://www.w3.org/ns/r2rml | High |
| W3C announcement blog post | https://www.w3.org/blog/2012/rdb-to-rdf-mapping-language-r2rml-and-a-direct-mapping-of-relational-data-to-rdf-are-w3c-recommendations/ | High |
| SQL to XSD type mappings (RDB2RDF wiki) | https://www.w3.org/2001/sw/rdb2rdf/wiki/SQL_to_XSD_Type_Mappings.html | Medium |
| R2RML wiki — test cases | https://www.w3.org/2001/sw/rdb2rdf/wiki/R2RML_Test_Cases | Medium |
| Efficient SPARQL-to-SQL with R2RML Mappings (Rodríguez-Muro & Rezk, Web Semantics 2015) | https://www.sciencedirect.com/science/article/abs/pii/S1570826815000153 | High |
