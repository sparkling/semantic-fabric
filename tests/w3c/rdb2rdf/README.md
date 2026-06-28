# W3C RDB2RDF Test Cases (vendored)

The **correctness gate** for semantic-fabric (ADR-0005) and the non-degradation half
of the Darwin fitness function (ADR-0001 / meta-harness Path-B posture).

## What goes here

A snapshot of the **W3C R2RML and Direct Mapping Test Cases**
(<https://www.w3.org/TR/rdb2rdf-test-cases/>) — ~49–63 named cases across 26 database
scenarios (`D000`–`D025`). Each scenario directory carries:

| File | Role |
|---|---|
| `create.sql` | DDL + INSERT — the relational source |
| `r2rml*.ttl` | the R2RML mapping document(s) |
| `mapped*.nq` | expected R2RML output (N-Quads — named-graph capable) |
| `directGraph.ttl` | expected Direct Mapping output (Turtle) |
| `manifest.ttl` | test metadata |

Positive cases assert the output graph; **error cases** assert the processor signals a
mapping/data error rather than producing output.

## Provenance (what is actually vendored here)

The canonical artefacts lived in the W3C Mercurial repo
(`https://dvcs.w3.org/hg/rdb2rdf-tests/`), which is now **`410 Gone`**. The
`D000`–`D025` scenarios under `cases/` were therefore obtained from a faithful
GitHub mirror of the W3C test suite — **`johardi/jr2rml-test-suite`** (its `res/`
directory), which carries the unmodified W3C artefacts (`create.sql`,
`r2rml*.ttl`, `mapped*.nq`, `directGraph.ttl`, `manifest.ttl`). Only the W3C
test-case data files are vendored (redistribution permitted under the W3C document
/ test licence); the mirror's own GPL test-runner code is **not** included. No
expected output has been altered (ADR-0005 honesty contract).

- 26 scenarios `D000`–`D025`; 63 R2RML mapping documents + 24 `directGraph.ttl`.
- Base IRI fixed at `http://example.com/base/`.

## Harness

`sf_conformance::run_suite` (ADR-0005) loads each `create.sql` into an in-memory
SQLite database, loads the mapping (R2RML parsed by `sf-mapping`; Direct Mapping
auto-generated from `sf-sql` introspection), runs
`CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` through the virtualiser (`sf-sparql`),
and compares the produced triples to the expected graph by **blank-node-aware
graph isomorphism** (`oxrdf` RDFC-1.0, not byte equality), cross-checked through
the in-memory oracle. It writes `earl-semantic-fabric-{r2rml,direct}.ttl`.

Run it: `cargo test -p sf-conformance` (the `w3c_suite` test prints the split and
the per-case pass/fail/skip with reasons, and gates on a non-regression baseline).

### Known non-passing categories (honest, documented — not gamed)

- **Named-graph output** (`rr:graphMap`) → **skipped (501)**: the `?s ?p ?o`
  CONSTRUCT dump emits the default graph only.
- **Error cases** the engine does not reject → **failed**: the virtualiser
  executes well-formed mappings and does not implement R2RML static validation
  (malformed-mapping / undefined-column rejection is a deferred validation layer).
- **`CHAR(n)` padding** → **failed on SQLite**: SQLite does not space-pad
  `CHAR(n)`; these match on PostgreSQL (per-DBMS forked fixtures, ADR-0015).
- **Direct Mapping, no-PK tables with duplicate rows** → **failed**: a virtualiser
  cannot mint a distinct per-row blank node without a key.
- **Relative-IRI `rr:column` base resolution** → **failed** (one case): templates
  are base-resolved; a relative `rr:column` IRI value is not yet.
