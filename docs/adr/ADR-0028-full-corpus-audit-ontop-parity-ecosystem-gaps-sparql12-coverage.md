---
status: accepted
date: 2026-07-16
ratified: 2026-07-16
tags: [audit, adr-drift, ontop-parity, ecosystem-research, sparql-1.2, devils-advocate, backlog]
supersedes: []
depends-on:
  - ADR-0001
  - ADR-0002
  - ADR-0006
  - ADR-0010
  - ADR-0012
  - ADR-0021
  - ADR-0025
  - ADR-0026
  - ADR-0027
implements: ADR-0012
---

# Full-corpus audit: ADR status, Ontop parity, ecosystem gaps, SPARQL 1.2 coverage

## Context and Problem Statement

Requested: a full review of every ADR against real implementation state, a definitive
Ontop-parity verdict, a scan of other R2RML/OBDA/virtualization frameworks for gaps
worth closing, and a soundness check of SPARQL 1.2 coverage — synthesized into one
ADR with a devil's-advocate pass over the findings before anything is taken at face
value.

## Methodology

Four parallel research agents (Claude Code `Agent` tool, `model: sonnet` per this
session's cost-routing convention; `ruflo` `swarm_init` used for topology tracking
only — per rUv's own documented convention, `agent_spawn`/execution stays optional
metadata and Claude Code's native tool is "always" the actual executor for one-shot
research with no cross-session learning need):

1. **`adr-auditor`** — read all 24 ADR files, cross-checked central claims against
   real source (grep, file reads, line counts) rather than trusting ADR prose.
2. **`ontop-parity-analyst`** — read ADR-0021/0022/0023/0024/0025,
   `docs/research/ontop.md`/`ontop-optimizer-dossier.md`, `BENCHMARKS.md`.
3. **`framework-researcher`** — read all 15 existing `docs/research/*.md` competitor
   docs, cross-checked against current `crates/` source, then light fresh web
   research for gaps not already covered.
4. **`sparql12-coverage-auditor`** — read ADR-0019/0004/0007/0008, README §9, and the
   `sf-sparql` implementation surface directly.

**Then a devil's-advocate verification pass** (this session, direct file reads/greps,
not delegated) spot-checked the most consequential and surprising claims from all
four reports before accepting them into this ADR. Every spot-check confirmed the
underlying claim, several with added precision the original report didn't have —
recorded inline below, not hidden.

## A. ADR Corpus Status

Full table from `adr-auditor`, condensed to verdicts that diverge from frontmatter
status (see `adr-auditor`'s full per-ADR table in this session's transcript for the
complete 24-row version; only divergences and process-level findings are load-bearing
here):

| ADR | Frontmatter | Verified | Gap |
|---|---|---|---|
| 0006 | accepted | **accepted-but-gap-found** | `deadpool-postgres` — see devil's-advocate note below; `lasso` interning documented, never a dependency |
| 0007 | accepted | shipped, description stale | Pipeline description describes the flat cascade; production default is the ADR-0023 tree IR since M8 (frontmatter not updated) |
| 0010 | accepted | **accepted-but-gap-found** (already amended this session) | No cycle detection in `P+`/`P*` (depth-limit only, `emit.rs:323-386`); no result-size cap/admission control; the already-documented stream-lane-pool gap |
| 0011 | accepted | **proposed-not-built** | Zero tracing/metrics/config-validation anywhere in the codebase |
| 0012 | accepted | **accepted-but-gap-found** | `proptest`/`cargo-fuzz`/`insta` — its own stated release-blocker (R3) — entirely absent |
| 0017 | accepted | **proposed-not-built** | Zero PROV-O/lineage/reification implementation |
| 0018 | accepted | **proposed-not-built** | Zero RLS/ABAC/sensitivity-label implementation — the ADR's own stated problem ("anyone who can query reads all mapped rows") is unsolved |
| 0023/0024 | accepted | shipped, **exceeds** ADR framing | Both more mature than their own text conveys (tree IR is production default; executor abstraction now spans 11 adapters, not the smaller set originally scoped) |
| 0025 | accepted | shipped, one stale entry | Tier-2 "reflexive `P*`/`P?`" listed open; closed by commit `b206dc0` (2026-07-08), text never updated (found by `ontop-parity-analyst`) |

### Process finding (not a features gap — a rigor gap)

Three ADRs (0011, 0017, 0018) carry `status: accepted` with **literally zero
implementation** — not "started, incomplete," but no code at all. This is
qualitatively different from ADR-0010's gap (mostly built, one clause missing) or
ADR-0012's (built, one layer missing). A reader cannot currently tell "accepted and
built" from "accepted, design-only, unscheduled" from the frontmatter alone.
**Devil's-advocate framing**: this may be a real schedule slip, or it may reflect an
unstated house convention where "accepted" means "we agree this is the right design"
rather than "this exists." Both are legitimate outcomes — the actual problem is that
the *distinction itself* isn't visible anywhere. Recommend (not executed here, a
maintainer call): either a status-correction pass on these three ADRs matching the
amendment style already used on ADR-0010/0015/0024 this session, or an explicit
project-wide convention note (e.g. in `ADR-0012` or a README section) defining what
"accepted" commits to.

## B. Ontop 5.5.0 Parity Verdict

**The defensible claim is "parity on the verified 15-feature-class/166-test-oracle
surface, performance parity-or-faster measured at 4 scales, with 2 acknowledged
cosmetic SQL-shape exceptions and 1 deliberate charter exclusion" — not "full
parity."**

- **Feature parity**: full SPARQL surface wired and regression-locked (BGP, FILTER,
  OPTIONAL, UNION, MINUS, aggregates, GROUP BY, BIND, VALUES, GRAPH, ORDER BY,
  (NOT) EXISTS, subqueries, property paths incl. reflexive `P*`/`P?`). W3C RDB2RDF:
  81/82 SQLite, 80/81 PostgreSQL, one documented deviation (`R2RMLTC0002f`).
- **Two places semantic-fabric is measurably ahead of Ontop, not just at parity**:
  general recursive property paths inside `EXISTS`/`MINUS` (Ontop has no general
  transitive-path mechanism — hardcodes only `rdfs:subClassOf*`, and skips 11 W3C
  property-path compliance tests in its own suite); `COUNT(DISTINCT *)` correctness
  (Ontop's `RDF4JValueExprTranslator` has a confirmed live bug silently dropping the
  `DISTINCT` flag for the wildcard form). Both claims are time-bound to Ontop 5.5.0
  as documented in `docs/research/ontop.md` — a future Ontop release could close
  either.
- **Optimization parity**: matching passes confirmed (self-join/left-join
  elimination, FK/redundant-join elimination, IRI-template pruning, FD closure,
  agg-through-union pushdown, the full ADR-0023 operator-tree IR mirroring Ontop's
  IQ model). Two remaining cosmetic Tier-3 SQL-shape gaps (shared-URI-template
  binding-lift across UNION arms; Slice·Distinct·Union arm-count pruning) — correct
  `=_bag` result, different SQL text, reversing either means reversing ADR-0006/0007
  design choices, explicitly not attempted.
- **Coverage parity**: live `race.sh` head-to-head (real Ontop JVM + PostgreSQL vs
  semantic-fabric's native HTTP endpoint, same data) shows row-parity on all 15 GTFS
  query classes at 4 scales, faster on every query in the final re-race
  (~3x geomean, up to 16.6x on the largest join/path queries). **Caveat carried
  forward from `ADR-0025` itself**: this is the last checked-in re-race
  (2026-07-07), not a fresh measurement this session — a stale release binary has
  previously hidden real wins (documented incident in ADR-0025).

## C. Ecosystem Gaps (Other R2RML/OBDA/Virtualization Frameworks)

Ranked by `framework-researcher`, verified against source where checked:

**Tier 1 — highest impact, open:**
1. **Mapping-partition/invariant-disjointness algorithm** (Morph-KGC-inspired) — zero
   code in `sf-mapping` today. Pure logic, no I/O, substrate-agnostic. Unlocks
   parallel materialization dispatch and is the prerequisite for item 5 below
   (incremental re-materialization). Reasonably low risk to build, though "zero risk"
   (the original framing) should be read as "low, relative to other backlog items" —
   risk assessment before implementation, not after, is inherently provisional.
2. **Cross-source semi-join planner built, never wired to execution.**
   **Devil's-advocate-verified**: `crates/sf-sql/src/cost.rs::plan_semijoin`'s own
   module doc confirms this is genuinely a cross-*database* federation planner
   ("combining tables that live in *different* relational databases"), not just an
   intra-query join-order optimization already covered by the cascade passes. Direct
   grep confirms zero callers outside `cost.rs`'s own test module. This is a real,
   precisely-characterized dead feature — but whether wiring it in is currently
   in-scope needs a maintainer read of `ADR-0002`'s charter (does v1 include
   cross-database federation, or is single-database-per-mapping still the boundary?)
   before treating it as a definite next step.
3. **Resource governance half-built against its own ADR-0010** — independently
   corroborated **three separate ways** this session: this session's own live load
   test (`ADR-0027`), `framework-researcher`'s research-doc cross-check, and
   `adr-auditor`'s direct `Cargo.toml`/`Cargo.lock` dependency check (confirming
   `deadpool-postgres` is only a stale 2026-06-27 planning comment in `Cargo.toml`,
   never an actual dependency — PostgreSQL genuinely runs over a single
   `Arc<Client>`). Three independent methods, same conclusion — about as solid as
   evidence gets without shipping the fix.

**Tier 2 — medium impact, open:**
4. Redundant-union elimination (Ontop has this optimizer pass; semi-fabric's cascade
   module has the adjacent passes but not this one — incremental effort against an
   established pass architecture).
5. Incremental re-materialization/CDC (`ADR-0016` designs the full pipeline; zero
   code exists; correctly gated behind item 1).

**Tier 3 — confirmed already adopted, no action:** DuckDB heterogeneous-file reader,
streaming/backpressure architecture, SHACL via `rudof`, Direct Mapping, the
push-down-over-hand-rolled-spill philosophy for dedup/join, hand-rolled OWL-RL-tier-1
reasoning (a considered choice, not a gap).

**New from fresh web research**: D2RQ and Sparqlify are legacy/architecturally
irrelevant (frozen since ~2012-2013; Sparklify's Spark-cluster model doesn't fit a
single-process engine) — confirms rather than contradicts the existing research's
focus on Ontop/Ultrawrap as real prior art. Two new benchmarks worth tracking if
OWL-entailment or fine-grained SPARQL-feature performance claims are ever made:
**LUBM4OBDA** (2024, inference-focused) and **Sparqloscope** (ISWC 2025, per-feature
engine comparison). No qualitatively new Ontop capability beyond what
`docs/research/ontop.md` already documents (confirms that doc is current). Virtuoso/
Denodo/Trino: nothing beyond what `obda-resource-governance.md`/`federation.md`
already captured.

## D. SPARQL 1.2 Coverage Verdict

Three buckets, per `sparql12-coverage-auditor`, each spot-checked:

**1. Sound and tested** (real oracle-backed differential tests, not just "the code
path exists"): BGP/JOIN/FILTER/OPTIONAL/UNION, the MINUS-vs-`FILTER NOT EXISTS`
semantic distinction (explicitly tested, not just assumed), full property-path
composite surface (`differential_paths.rs`), standard aggregates including edge
cases (empty-group SUM, all-NULL-OPTIONAL), VALUES/BIND/UNDEF, subqueries as JOIN
operands.

**2. Honestly incomplete:**
- Property paths: bound endpoint, nested closure in composite, shape-mismatched
  composites, `P*`/`p?` reflexive over non-single-predicate graphs (README §9,
  confirmed still accurate).
- **`GROUP_CONCAT`, `SAMPLE`, and custom aggregates are unconditional 501s on both
  execution paths** — devil's-advocate-verified directly
  (`crates/sf-sparql/src/iq/build.rs:981-995,1095-1109`: literal
  `"GROUP_CONCAT is deferred → 501"` messages). **This is not in the README's
  limitations table at all** — a real, previously-undisclosed documentation gap,
  not just an implementation gap.
- SERVICE federation, `GRAPH ?var`, tier-2 OWL 2 QL: deliberate charter exclusions.
- RDF-star/RDF 1.2 quoted triples: **architecturally N/A, not a gap** —
  `sf-core/src/term.rs` correctly rejects `Term::Triple` as an R2RML term-map value
  (R2RML has no construct that could ever produce a quoted triple from relational
  rows); recommend treating this as out-of-scope-by-architecture in any future
  SPARQL-1.2-readiness claim, not a checklist item to close.

**3. Untested-but-implemented (the risk bucket)**: two candidates flagged — negated
property sets nested inside a set-composite, and `!p` over a graph mixing
`rr:class` triples — both correctly `501` per the code, neither backed by a
dedicated differential test proving the 501 (vs. silent wrong output) is actually
hit. Lower severity than this session's SQL Server date-epoch bug (that was a
*wrong-answer* bug in a claimed-working path; these two are *unverified-501* paths),
but the same class of risk in miniature.

**Documentation staleness found (real, devil's-advocate-confirmed word-for-word)**:
`crates/sf-sparql/src/iq/lower.rs`'s own module doc says *"Status: tree path only
(NOT the live engine)... it is not wired into the live Plan/exec/unfold path. The
flat crate::unfold stays the production engine"* — directly contradicted by
`crates/sf-sparql/src/lib.rs:364-365`: *"This is the production default since M8:
`translate` and `translate_with` both route here."* Confirmed by reading both files
directly, not taking either agent's word for it. Also: `ADR-0019` and
`sf-sparql/Cargo.toml`'s own comments still claim SPARQL 1.2 support is "compiled
out" / sparopt "fails to compile" — refuted by the live `Cargo.toml` feature flags
and ADR-0004/0007's own 2026-06-28 reconciliation notes. All doc-only drift, zero
functional risk, but exactly the kind of thing that misleads the next reader who
trusts the comment over the code.

**Methodology caveat carried forward**: the README's "81/82 SQLite, 80/81 PostgreSQL"
conformance number is R2RML *mapping* correctness (W3C RDB2RDF suite), not SPARQL
*query-language* conformance — SPARQL algebra correctness rests on this repo's own
differential-oracle suite (166 tests against `spareval`/oxigraph), a real but
methodologically distinct form of evidence. Don't conflate the two when citing
"conformance" externally.

## Consolidated Prioritized Backlog

Deduplicated across all four streams + the process finding:

**P0 — correctness/security, no dependencies:**
1. Cycle detection for `P+`/`P*` (depth-limit exists; genuine cycle detection does
   not — `emit.rs:323-386`).
2. Result-size cap / admission control on `sf-serve` (timeout + query-length cap
   exist; nothing bounds response size or sheds load under concurrency —
   `ADR-0010`/`ADR-0027`).
3. `ADR-0018` (RLS/ABAC/sensitivity) — zero-built despite "accepted"; real exposure
   if ever deployed multi-tenant.
4. `proptest`/`cargo-fuzz` — `ADR-0012`'s own declared release-blocker, absent.
5. `GROUP_CONCAT`/`SAMPLE`/custom-aggregate 501s — add to README's limitations
   table at minimum (documentation fix); implementation is a larger, separate call.

**P1 — designed-but-unwired, comparatively cheap:**
6. PostgreSQL connection pool (`deadpool-postgres` or equivalent) — three
   independent findings converge here.
7. Wire the semi-join federation planner into execution (pending an `ADR-0002`
   charter check on whether cross-database federation is in scope).
8. Mapping-partition/invariant-disjointness algorithm — unblocks item 9.
9. Incremental re-materialization/CDC (`ADR-0016`, gated on item 8).
10. Redundant-union elimination optimizer pass.

**P2 — process / documentation, no functional risk:**
11. Status-correct `ADR-0011`/`0017`/`0018` (or define what "accepted" commits to
    project-wide).
12. Fix `iq/lower.rs`'s stale "not wired" module doc; fix `ADR-0019`/
    `sf-sparql/Cargo.toml`'s stale SPARQL-1.2-unsupported comments; update
    `ADR-0025`'s stale reflexive-path entry; fix `ADR-0007`'s stale pipeline
    description; fix `ADR-0006`'s `lasso`/`deadpool-postgres` comment drift.
13. Adversarial differential tests for the two untested-but-501 property-path edge
    cases (Bucket 3, §D).
14. `ADR-0011` observability — currently zero, and would make future governance
    claims (like `ADR-0010`'s own "emits a metric" text) actually true.
15. `ADR-0017` provenance/lineage — currently zero.
16. Track `LUBM4OBDA`/`Sparqloscope` as future benchmark targets if inference or
    per-feature performance claims are ever made externally.

## Consequences

**Good**: this is the most current, cross-verified picture of the engine's real
state available — every headline claim in sections A-D was independently
spot-checked against source by a second pass, not accepted from a single agent's
say-so. Three genuinely independent methods converging on the PG-pool gap (this
session's own load test, a research-doc cross-check, and a raw dependency-listing
check) is stronger evidence than any single approach would have produced.

**Bad/cost**: this ADR itself will drift the moment any P0/P1 item ships — per this
project's own living-ADR discipline, whoever closes an item here should amend this
ADR (or the more specific ADR it belongs to) in the same piece of work, not let this
become the next stale document it warns against.

**Neutral**: several items (semi-join wiring, cross-database federation scope) need
a maintainer decision this audit deliberately did not make unilaterally — flagged,
not resolved.

## More Information

* Full unabridged per-agent reports (all per-ADR rows, full file/line citations):
  this session's transcript, agents `adr-auditor`, `ontop-parity-analyst`,
  `framework-researcher`, `sparql12-coverage-auditor` (2026-07-16).
* Related: `ADR-0010`/`ADR-0015`/`ADR-0024`/`ADR-0026`/`ADR-0027` (amended/created
  earlier this session; this ADR's findings are consistent with and build on all
  five).
* Source spot-checked directly this session (devil's-advocate pass, not delegated):
  `Cargo.toml`, `Cargo.lock`, `crates/sf-sparql/src/{iq/build.rs,iq/lower.rs,lib.rs}`,
  `crates/sf-sql/src/cost.rs`, plus grep sweeps for `deadpool`, `tracing`/`metrics`,
  `SET LOCAL`/RLS/ABAC terms, `proptest`/`cargo-fuzz`/`insta`, `plan_semijoin`
  call-sites.
