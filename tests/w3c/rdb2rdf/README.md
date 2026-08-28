# W3C RDB2RDF Test Cases (vendored)

The **correctness gate** for semantic-fabric (ADR-0005) and the non-degradation half
of the Darwin fitness function (ADR-0001 / meta-harness Path-B posture).

## What goes here

A snapshot of the **W3C R2RML and Direct Mapping Test Cases**
(<https://www.w3.org/TR/rdb2rdf-test-cases/>) — exactly 87 named cases across 26
database scenarios (`D000`–`D025`). Each scenario directory carries:

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

- 1 suite manifest, 26 scenarios `D000`–`D025`, 63 R2RML cases, 24 Direct
  Mapping cases, and 189 case-tree files.
- Base IRI fixed at `http://example.com/base/`.

`inventory.tsv` is the canonical generated seal. It records every case identity,
kind, scenario, expected-error flag, SQLite/PostgreSQL allowed outcome, required
file, and SHA-256 digest. It is mapping-fixture and allowed-policy evidence, not
a SPARQL query/Protocol result and not an execution receipt.

## Harness

`sf_conformance::run_suite` (ADR-0005) first checks the full seal, then executes
all 87 cases in canonical inventory order. It loads each `create.sql` into an
in-memory SQLite database, loads the mapping (R2RML parsed by `sf-mapping`;
Direct Mapping auto-generated from `sf-sql` introspection), runs
`CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` through the virtualiser (`sf-sparql`),
and compares the produced triples to the expected graph by **blank-node-aware
graph isomorphism** (`oxrdf` RDFC-1.0, not byte equality), cross-checked through
the in-memory oracle. Missing, unreadable, or malformed sealed inputs are fatal;
new skips, deviations, identity substitutions, or order changes cannot hide
behind unchanged aggregate counts. It writes
`earl-semantic-fabric-{r2rml,direct}.ttl`.

PostgreSQL uses the same sealed order and per-ID policy. A local run without a
provider may return typed `untested` evidence, but receipt generation and replay
always select explicit required-live mode; provider absence is fatal regardless
of ambient `CI`.

Run the primary checks:

```bash
cargo run --locked -p sf-conformance --bin rdb2rdf-inventory -- --check
cargo run --locked -p sf-conformance --bin rdb2rdf-execution-receipt -- --check
cargo run --locked -p sf-conformance --bin rdb2rdf-execution-receipt -- --backend postgresql --check
cargo test --locked -p sf-conformance --test rdb2rdf_runner_seal
cargo test --locked -p sf-conformance --test w3c_suite -- --nocapture
cargo test --locked -p sf-conformance --test w3c_pg_suite -- --nocapture
```

Backend-aware v3 receipts bind every ordered case identity, kind, status and
typed cause to this inventory: SQLite records 81 passes, one declared deviation
(`R2RMLTC0002f`), and five exact skips; required-live PostgreSQL records 80
passes, the same deviation, and six exact skips. They attest mapping inputs and
outcomes only—not runner/toolchain/host/provider provenance, SPARQL Query or
Protocol conformance, release readiness, or production admission.

### Known non-passing outcomes (honest, per-ID and fail-closed)

- **`R2RMLTC0002f`** is the one declared deviation; ADR-0015 records the
  identifier-case semantics and the remaining PostgreSQL parity item.
- **SQLite `DirectGraphTC0021`–`DirectGraphTC0025`** are declared fixture skips
  because their W3C DDL is not accepted by SQLite.
- PostgreSQL has its own six exact, named allowed skips in `inventory.tsv`; no
  category-level allowance can admit a newly skipped identity.
