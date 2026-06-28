---
status: accepted
date: 2026-06-27
tags: [testing, unit, integration, property-based, fuzzing, snapshot, ci, security]
supersedes: []
depends-on:
  - ADR-0005
  - ADR-0007
  - ADR-0010
implements:
  - ADR-0001
---

# Test strategy — the inner layers below conformance

## Context and Problem Statement

ADR-0005 specifies the **outer** test layers — W3C RDB2RDF conformance (via CONSTRUCT), the GTFS-Madrid OBDA benchmark, the native in-memory Oxigraph differential oracle, EARL. The **inner** layers (unit / integration / property / fuzz) were unspecified. For a query *translator* that turns untrusted SPARQL into executed SQL, those inner layers are load-bearing for both **correctness** (the optimizer cascade is order-sensitive — the hardest surface, ADR-0007) and **security** (the injection/DoS surface, ADR-0010). This ADR fixes them.

## Decision Outcome — the test pyramid

1. **Unit.** `sf-core` IR + term generation + the R2RML §10 datatype table (table-tests); `sf-mapping` R2RML parser; **each `sf-sparql` cascade pass in isolation** on hand-built IQ inputs (IRI-template pruning, self-join elimination, FD inference — the order-sensitive ones); `sf-sql` dialect emission per dialect.
2. **Integration.** End-to-end SPARQL → SQL → bindings against embedded **SQLite** and a **PostgreSQL** fixture (ADR-0005): load `create.sql` → run SPARQL → assert bindings; run a CONSTRUCT → assert triples.
3. **Property-based (`proptest`).** Generators for valid R2RML + relational data + SPARQL; invariants:
   * **Rewriter correctness vs the in-memory oracle** — the virtualiser's live-SQL answer equals evaluating the same SPARQL over the expected graph loaded into an in-memory store (`spareval`, ADR-0005). *The core property.*
   * **Datatype fidelity** — SQL value → RDF literal per §10 round-trips (ADR-0015).
   * **NULL / OPTIONAL safety** — generated `OPTIONAL` queries never leak the two NULL regimes (ADR-0007 R1–R5).
4. **Differential & metamorphic — the rewriter-correctness core (ADR-0007).**
   * **Native-oracle differential** — virtualiser answer vs the in-memory Oxigraph oracle (ADR-0005); the check that catches a bug in the *base translation* itself.
   * **NoREC-style internal differential** — execute the *unoptimized* IQ (the ISWC-2018 base translation) vs the *optimized* IQ against the same source; any diff **pinpoints the offending cascade rule**, no oracle needed.
   * **MR1 constraint-toggling metamorphic test** — run with integrity constraints declared vs withheld; results must be identical (constraints may only *enable* optimizations) → detects unsound FD/key/join-elimination.
   * **Per-rule bounded equivalence (VeriEQL)** — each cascade rule discharged as a SQL-equivalence obligation under declared PK/FK/NOT-NULL.
   * **Per-DBMS datatype fixtures** — golden output per `(SQL type × dialect)` with cross-dialect byte-identity (ADR-0015).
5. **Fuzzing (`cargo-fuzz` / libFuzzer).** Target the **SPARQL→SQL rewriter** with arbitrary/adversarial SPARQL; assert **no panic, no injection** (generated SQL always parameterised, identifiers always from the mapping — ADR-0010 R1/R2), **no unbounded recursion** (ADR-0010 R3), and **termination under governance** (ADR-0010 R4). Also fuzz the R2RML/Turtle parser for panic-safety. *This is the ADR-0010 Confirmation in executable form.*
6. **Snapshot (`insta`).** Pin generated SQL per dialect (PostgreSQL / SQLite) so rewriter changes surface as reviewable diffs.

Atop these sit the ADR-0005 outer layers: conformance (W3C via CONSTRUCT + EARL), the GTFS-Madrid OBDA benchmark (`criterion`), the differential oracle.

### CI gating (extends ADR-0006)
`fmt` + `clippy -D warnings` + unit/integration/property + a **bounded fuzz smoke** per push (**long fuzz nightly** from a persisted corpus) + the W3C conformance run + `criterion` regression thresholds + a **constant-memory check** on the OBDA benchmark (the streaming invariant, ADR-0006).

### Consequences
* Good — correctness + security net; the rewriter-correctness property is *guaranteed* by generation; fuzz catches the translator's real failure modes; SQL changes are reviewable.
* Bad — `proptest` generators for *valid* R2RML + SPARQL are non-trivial to author; fuzzing needs a corpus + time (nightly, not per-push).
* Neutral — meaningful test-code volume (expected for a translator).

### Confirmation
* The pyramid runs in CI; the rewriter-vs-oracle property holds over generated cases; fuzzing finds no panic/injection over the corpus; snapshots gate SQL changes.

## Rules
* **R1** — every optimizer-cascade pass has isolated unit tests (the order-sensitive surface, ADR-0007).
* **R2** — rewriter-vs-in-memory-oracle is a **property** test, not just fixed cases.
* **R3** — the SPARQL→SQL rewriter is **continuously fuzzed**; a fuzz finding (panic / injection / unbounded recursion) is a **release blocker**.
* **R4** — generated SQL is snapshot-pinned per dialect.

## More Information
* **Outer layers this extends:** ADR-0005. **Fuzz target + the security controls it verifies:** ADR-0007, ADR-0010. **CI baseline:** ADR-0006. **Datatype fixtures:** ADR-0015.
</content>
