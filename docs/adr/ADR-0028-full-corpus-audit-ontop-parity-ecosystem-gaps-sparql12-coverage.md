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
- **RDF-star — genuine parity, not an asymmetry (added 2026-07-16, follow-up
  research)**: mainline Ontop ships **zero** RDF-star support — checked directly
  against Ontop's own release notes (every version 1.5.1-RC1 through 5.5.0) and
  its SPARQL 1.1 compliance page, no mention anywhere. The only related artifact is
  a 2022 MSc thesis ("Extending VKG Systems with RDF-star Support," Sundqvist, Free
  University of Bozen-Bolzano) hosted on Ontop's own publications page, proposing
  an R2RML-star mapping extension — academically affiliated with the Ontop group
  but never merged, never productized, no evidence of a maintained fork. So this
  is **not** "Ontop supports RDF-star and semantic-fabric doesn't" — both engines
  have zero shipped capability here. The honest asymmetry is narrower: Ontop's
  research community has published a design sketch for *how* R2RML could be
  extended to construct quoted triples; semantic-fabric has no equivalent design
  document. Not a functional gap in today's parity comparison.
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
  SPARQL-1.2-readiness claim, not a checklist item to close. **Confirmed this isn't
  a competitive gap either** (follow-up research, 2026-07-16): mainline Ontop 5.5.0
  ships zero RDF-star support too — see §B's RDF-star note for the full citation.
  Both engines are at parity here (neither has shipped capability); Ontop's
  research group has an unmerged 2022 thesis design (R2RML-star) semantic-fabric
  has no equivalent of, which is the only real asymmetry.

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

## E. RDF-star follow-up: RML-STAR status and a plain-RDF encoding path

Two more follow-up agents (`rdfstar-encoding-researcher`, after
`ontop-rdfstar-researcher` in §B) closed out the RDF-star question in full —
whether RML (the actively-developed R2RML superset) supports it, and whether
RDF-star's own semantics reduce to a plain-RDF encoding semantic-fabric could
exploit without adding a genuinely new R2RML term type.

**RML-STAR mechanism (real, spec-grounded)**: RML-STAR does not add a new
`TermMap` kind. It adds `rml:StarMap` (nests inside a Subject/ObjectMap),
`rml:quotedTriplesMap` (links a StarMap to the `TriplesMap` whose output becomes
the quoted triple), and `rml:AssertedTriplesMap`/`rml:NonAssertedTriplesMap`
(controls whether the underlying triples are also emitted as regular asserted
triples). This is a genuine RML-specific extension, not reproducible with vanilla
R2RML/RML term maps alone.

**A real status discrepancy — confirmed, not just flagged (independently re-verified
twice)**: this repo's own `docs/research/rml-yarrrml.md` (dated 2026-06-26) states
RML-Core *and* RML-STAR "have also been adopted as Final Community Group Reports,"
citing a 16 March 2026 draft date. Two independent direct fetches of the live
RML-STAR spec page — one by `rdfstar-encoding-researcher`, one done separately by
the main session itself as a doubt-check — both show **"Draft Community Group
Report, dated 10 May 2023"**, and the GitHub API confirms `kg-construct/rml-star`
has exactly one release, `v0.1.0`, published 2023-05-10 ("Initial draft of the new
RML ontology"). Two agreeing fetches, done independently, is solid confirmation:
**RML-STAR specifically is still Draft status, not Final** — this repo's own
research doc's "Final" claim is wrong for RML-STAR (it most likely conflated
RML-Core's progression with RML-STAR's, since the two are versioned/reported
together in that doc's prose). `docs/research/rml-yarrrml.md` should be corrected
in a future pass. **Do not cite RML-STAR as finalized** — the mechanism description
above is solid
regardless of exact CG-report status.

**Corroborating check (main session, direct)**: `rml.io` itself (the canonical RML
portal — tools, editor, playground) doesn't cover spec/CG status at all, so it adds
nothing on the status question either way. But it pointed at the actual reference
implementation, `RMLio/rmlmapper-java`, which is worth checking independently of
spec status: **zero code-level references to `StarMap`/RML-star anywhere in the
repo, and its README (latest release v8.1.0, 2025-12-23 — actively maintained) makes
no mention of RDF-star support at all.** Consistent with, and independent
reinforcement of, the Draft-not-Final finding above: even the primary reference
implementation hasn't implemented RML-STAR.

**Org-wide sweep (main session, direct — all ~60 `RMLio` GitHub repos listed and the
most relevant ones checked)**: the null result holds across the whole ecosystem, not
just `rmlmapper-java`. `Algebraic-Mapping-Operators` (a newer "mapping algebra"
operator library, a different lineage from the direct-execution tools) states
outright "further operators will be implemented as they're defined" — no
star/quoted-triple operator exists yet. `mappingloom-rs` (Rust, updated the same day
as this research) has **zero code-level hits for "star"**; its README lists
RML-STAR only as one of the six modules in the *official conformance test corpus it
runs against*, with no claim of actually passing or implementing those specific
cases — the same "covered by the test suite ≠ fully passing" distinction RMLMapper's
own published conformance numbers already illustrate (98.70% RML-Core, 50.75%
RML-IO — partial, not binary). One correction made in the course of this check:
`RML-Model` first looked like it might be a newer successor worth checking, but its
own README clarifies it's the **old**, deprecated predecessor — "rebuilt from the
ground up on another repository (rmlmapper-java); all future development will now
happen there."

**Correction (same day, user follow-up) — RML-STAR is real and partially
implemented, in RMLStreamer's actual alpha successor.** The above checked
`Algebraic-Mapping-Operators` and `mappingloom-rs` (the operator/algebra libraries)
but not the application that consumes them. Confirmed directly from an `RMLio`
maintainer's own comment (GitHub issue `RMLStreamer#63`, 2025-10-23): *"This project
[RMLStreamer] is on maintenance mode with very little activities. A successor
project is here `RMLio/MappingWeaver-java` which would eventually replace
RMLStreamer. That project is still in alpha mode and being worked on actively."*
`MappingWeaver-java` is genuinely alpha and genuinely active — commits as recent as
2026-07-15/16 (this ADR's own edit date), latest tagged release `v0.2.0`
(2026-05-27). It embeds Flink internally to execute algebraic mapping plans compiled
via `mappingloom-rs`/`Algebraic-Mapping-Operators` — architecturally still a
materialization/ETL engine like RMLStreamer, **not** an OBDA/virtualization engine,
so still not directly comparable to semantic-fabric's own architecture.

**Second correction, same day (user asked specifically about nested star patterns)
— the "11% passing" figure was numerically right but substantively misleading.**
Read the actual JUnit test file directly
(`src/test/java/.../rml_kgc/RMLSTARTest.java`) rather than trust a secondhand
summary of it. Its own code comment: *"RDF-star plan generation (quoted / asserted
triples) is not supported, so translation throws before any output is produced."*
All 16 **positive** (should-succeed) test cases — `RMLSTARTC001a` through `008b`,
including `RMLSTARTC008a`/`008b` ("two-level non-asserted quoted triple... both
triples have in turn non-asserted quoted triples," i.e. the nested case this
question was actually about) — are `@Disabled("Not running known failing test
cases in CI")`. The only 2 cases that pass, `RMLSTARTC009`/`010`, are **negative**
tests (quoted triples as a predicate map, or as an object without an object map —
both correctly expected to error) that pass by correctly *rejecting* invalid
input, not by producing real RDF-star output. So the honest figure is **0% of the
"does RDF-star generation actually work" cases pass — nested or otherwise**;
there is currently no meaningful difference between MappingWeaver-java's
non-nested and nested handling because neither path produces output yet. What is
real and valuable: the full 18-case official test corpus (correct expected
outputs already transcribed from the spec) is built and wired into the harness,
disabled-but-ready — normal test-first scaffolding ahead of the feature landing,
not evidence the feature itself exists.

**Twice-revised verdict**: RML-STAR is Draft-status and unimplemented in every
*mature/legacy* tool checked (`rmlmapper-java`, `RML-Model`), the standalone
operator libraries have no dedicated star handling, and even the ecosystem's
active alpha successor (`MappingWeaver-java`) has zero working RDF-star
generation today — single-level or nested. The one genuinely real thing that
exists anywhere in the `RMLio` organization is a complete, spec-accurate test
corpus sitting ready for whenever the feature itself gets built. Do not cite any
percentage of "RML-STAR support" for `MappingWeaver-java` without checking
whether the underlying JUnit tests are `@Disabled` — a naive test-pass-rate
number reads as partial support when the truth is zero.

**The plain-RDF encoding question — precise answer, W3C-spec-grounded**:

- RDF 1.2 Concepts (§1.5, "Triple Terms and Reification") treats a triple term as
  its own RDF-term kind, distinct from IRI/blank-node/literal, and is explicit that
  asserting a triple *containing* a triple term does not assert the embedded
  proposition. It does **not** itself define a plain-RDF compatibility encoding.
- That encoding lives in a separate document, **RDF 1.2 Interoperability**: the
  **"basic encoding"** replaces each triple term with a fresh blank node `b` and
  adds four plain triples — `(b rdf:type rdf:PropositionForm)`,
  `(b rdf:propositionFormSubject s)`, `(b rdf:propositionFormPredicate p)`,
  `(b rdf:propositionFormObject o)` — with **basic decoding** as the exact reverse.
  The spec deliberately mints new predicate names rather than reusing classic
  `rdf:Statement`/`rdf:subject`/`rdf:predicate`/`rdf:object`, specifically because
  that legacy vocabulary already appears in real datasets (Uniprot cited by name)
  and reusing it risked silent corruption on graph merge.
- **Real prior art exists, using the classic vocabulary instead**: RDF4J ships
  `Models.convertRDFStarToReification`/`convertReificiationtoRDFStar` doing
  structurally the same shape with `rdf:Statement` etc. Jena's docs independently
  confirm the load-bearing invariant: round-tripping requires "only one
  reification for each unique quoted triple term" — i.e. the identifier must be
  *stable*, not merely fresh-per-run.
- **Oxigraph itself — semantic-fabric's own RDF/SPARQL substrate (ADR-0004) — does
  not do this decomposition internally.** It stores a triple term as a genuine
  native term (`Term::Triple(Box<Triple>)` in `oxrdf`), confirmed by direct source
  fetch. The "hash to a deterministic blank node" idea is **only a proposed,
  unimplemented option** in open Oxigraph issue #1286 ("Migration from RDF-star to
  RDF 1.2") — do not cite it as current Oxigraph behavior.

**Concrete exploitability verdict**: yes, structurally, for the **single-level
(non-nested)** case, using only ordinary R2RML/RML constructs — no RDF-star-specific
mapping feature needed:
1. A `rr:template`-generated identifier (IRI or blank node) as the synthetic stand-in
   for the quoted triple, plus plain constant-predicate `PredicateObjectMap`s for
   the four linking triples, is expressible in vanilla R2RML today.
2. The one place vanilla R2RML must go beyond the spec's own default: the
   identifier must be **deterministically** derived from the same relational
   columns producing s/p/o (satisfying the Jena-documented one-reification-per-term
   invariant needed for repeated query execution over the same rows) — solvable via
   plain templating when s/p/o are simple column values; nested triple-term
   components would need SQL-view precomputation or RML-FNML functions, so this
   is **not** unconditionally solvable for arbitrary nesting depth, only the common
   single-level case.
3. **The reverse leg — a SPARQL-star engine rewriting `<<?s ?p ?o>>` query patterns
   into the corresponding join against this plain-triple data at query time — does
   not exist anywhere as prior art.** Oxigraph has zero built-in support for
   interpreting externally-authored basic-encoded triples as native quoted triples.
   Semantic-fabric would have to build this translation layer itself (most
   naturally as a SPARQL-star algebra rewrite pass alongside the existing R2RML-to-
   SQL unfolding).

**Bottom line**: the target plain-RDF shape is real and W3C-specified ("basic
encoding") with independent precedent (RDF4J's converter, using different
vocabulary). But "author an R2RML/RML mapping that emits this shape, keyed
deterministically off relational columns, for a downstream SPARQL-star rewrite
layer to reconstruct" appears nowhere in the RML-STAR spec, the KGCW literature, or
any engine's documentation checked this session. **This would be a genuinely novel
pattern for semantic-fabric to pursue and document, not established practice to
cite** — a real, non-trivial build (add to the backlog below as an explicitly
research-stage item, not a scoped feature) if RDF-star support is ever prioritized,
offering a path that avoids extending the R2RML term-map model itself.

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
17. **RDF-star via plain-RDF encoding (§E)** — research-stage only, not a scoped
    feature: prototype whether a `rr:template`-keyed "basic encoding" R2RML
    mapping plus a new SPARQL-star algebra rewrite pass could offer RDF-star
    query support without extending the R2RML term-map model. Genuinely novel
    (no prior art combines these two halves); scope to the single-level
    non-nested case first per §E's own caveat.

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
