# SOTA application-completion programme

- **Status:** In progress
- **Date:** 2026-08-26
- **Updated:** 2026-08-28
- **Decision record:** [ADR-0038](../adr/ADR-0038-sota-application-completion-programme.md)

**Scope:** Repository source, tests, accepted ADRs, CI, and measured benchmark
evidence. GitHub issues and pull requests are deliberately not programme inputs.

## Execution status

| Slice | Status | Verified evidence |
|---|---|---|
| H0a — frozen replay-policy foundation | Complete | `b40dbc6`; schema-v4 surfaces unchanged; schema-v5 policy fingerprint `11c17544e97c1509456f6efb88081a55bd56c93ac306a9b05c2da7102e5f755b`; 381 tests passed and 2 expected skips |
| H0b — schema-v5 evaluator, scorer and envelope | Complete | `7a1fa24`; accepted golden policy/assessment/envelope `0d5505e4…61bb` / `4f4fe45c…a977` / `fdab0843…65e7`; hardened build; 430 tests passed and 2 expected skips; independent Codex and Claude COMMIT verdicts |
| H0c — trusted-launcher activation | Complete | V6 run `programme_v6_h0c_20260828_02` passed the candidate transaction and every hard gate at 100/100, with seven native-evidence digests, two final native reviews, no retry or repair, a sealed schema-V6 envelope, and provider-free verified replay. V4/V5 remain frozen |
| M0 — architectural truth and deterministic foundation | In progress | Through the current tranche: the sealed mapping authorities remain protected; per-test expected SQLite query/protocol baselines and the default `sf-cli` package dependency closure now have provider-free receipts; controlled performance capture/comparison machinery exists; and commit `ad94cdb` replayed byte-identically in two clean checkouts. Current required-live PostgreSQL evidence, actual binary artifact closure, and a controlled performance baseline remain open; the complete M0 release gate must be repeated in two clean builders after those authorities exist |
| M1–M7 — application completion | Gated | Existing product evidence remains valid, but no later milestone starts merely because an M0 slice lands; each milestone still requires its own executable QA gate |

Runs `_03` and `_04` remain honest historical failures at the final-review
boundary. Fresh run `programme_v5_h0c_20260828_05`, under controller `4b0756f`,
then passed the candidate transaction after one patch-admission repair and
proved the final build, three verifier lanes, generated EARL, four mutation
sentinels, dual native reviews, QE, and protected inputs. Its sealed V5 envelope
was nevertheless `REJECTED` at 85/100 solely because the frozen V1 reliability
law requires build evidence for every prior attempt, including an attempt whose
patch never reached build. Its policy, receipt, acceptance, envelope, and replay
digests are respectively
`c4411178334b54620da099ba7e2c9e029ebcc6873a0bc85e9dfca93bacbfeb79`,
`91bfc845961c84237ff7f9ea58e75bc77c76d21913f042e05e77c68bc7ab98d1`,
`cf7734d88eaa05e91916992c8ad8e1c2dfdc285e7a14fe14858bed3744288930`,
`4c0897695d9a865ff636088be806b4ce5074d3e3552a15038c58435b0017e05f`,
and `ca2bbbb3a909f846b0a443ce21572680a2fc9e930a0cb30d294cf2b4af178cfc`.

The correction is a sibling schema V6, not a reinterpretation of `_05`. Commits
`106da58` through `c8e3f68` add a V2 outer policy and gate contract, full
candidate transaction/native evidence, transition-derived `not-started`,
`failed`, or `passed` prior-build semantics, a trusted launcher, and
provider-free replay. Historical V4/V5 bytes and decisions remain immutable.

The first V6 attempt, `programme_v6_h0c_20260828_01`, passed policy review and
then failed closed with `HARNESS_NATIVE_ORIGIN_POLICY_DENIED` before admitting
native evidence. Its schema-V6 failure envelope and provider-free replay remain
honest historical evidence; they were not edited or reused. Exact subprocess
probes then confirmed that the pinned Claude essential-traffic environment uses
only `api.anthropic.com` and the Codex subscription route uses `chatgpt.com`, both
already declared first-party origins. No origin allow-list was widened and no
provider key or indirect route was introduced.

Fresh run `programme_v6_h0c_20260828_02`, under controller `c8e3f68`, sealed
policy fingerprint
`e71107e522342e1b19206c88d861549d00f9df87f27ed4293b0cc36139b2ae34`.
The transaction passed with six bound commands, seven native-evidence digests,
two final native reviews, and zero retries or repairs. Programme acceptance was
100/100 with every hard and diagnostic gate green. Candidate evidence, receipt,
acceptance, envelope, and execution-claim digests are respectively
`a1dc307130c6d3efb42354e0c464fcc66095d016564d5227e6259dc71998ac7f`,
`d9d244ef42c4a914b4b2bec52844b1ddc58a46d1b99759453cde7b34a5940216`,
`1cf36b1d9bcb2c4e6d2c81525baead859b2b62ed9042bfd33991bed8367430f7`,
`02c30ed3bb8f0b0b5a5c10320d64308934ee8438df051626e68e661589939a06`,
and `578799eff72dd84cc3b5601754654d1bce33a7b4b1fa56666271fe6833979c86`.
Provider-free replay independently verified the pass in receipt
`f1bcf0fe0720d2851dff219cf9b27563bcf6ff4da317ac9ec259b1fcd505bf02`.
H0c is therefore complete. M0 is in progress through verified incremental
product slices; M1–M7 remain gated by their own product evidence.

The SPARQL regression receipts bind per-test expected SQLite query and Protocol
outcomes. They are regression baselines only: they do not attest W3C SPARQL
Query/Protocol conformance, runtime provenance, or backend admission. The
default `sf-cli` dependency receipt closes locked package resolution, enabled
features, and normal/build dependency edges only. It does not attest binary
bytes, build-script output, linker or system provenance, an SBOM,
reproducibility, or production admission.

Performance production and comparison machinery now exists, but no controlled
runner profile, baseline, candidate, or measured numbers exist. The clean-
release first baseline must still be captured from clean committed source on
the exact controlled runner/profile.

On 2026-08-28, exact commit `ad94cdb` was cloned twice without local hard links
under the hardened-builder `umask 0022`. Each checkout rebuilt the controller,
passed all 91 harness files (627 tests passed and 8 environment-intentional
skips), replayed the RDB2RDF, query, Protocol, dependency-closure, performance-
scenario, and capability authorities, remained Git-clean, and produced
byte-identical authority and controller digests. The harness correctly rejected
an earlier pair created under `umask 0002` because tracked inputs were group-
writable; no trust check was relaxed. This closes current-tranche checkout
repeatability, not binary reproducibility or the final release proof.

M0 therefore remains open for current required-live PostgreSQL evidence, actual
binary artifact closure, and the controlled performance baseline. Once those
exist, the complete release gate must run again in two clean builders. M1–M7
remain gated.

## Outcome

Complete semantic-fabric as a state-of-the-art, virtualisation-only knowledge
graph over live relational systems of record. Completion means exact-or-fail
semantics, bounded execution, real cross-source query execution, production
security and operability, and independently verifiable release evidence.

The semantic compiler does **not** need a wholesale rewrite. Two substantial,
evolutionary architecture changes are required:

1. lower every advertised global operator to a bounded physical execution path;
2. add source identity, a source registry, and a federated physical plan.

If the second item is removed, the application charter must change from systems
of record/cross-RDBMS federation to one source per deployment. This programme
retains the accepted charter.

## Audited baseline

This is a product-readiness assessment, not a code-quality score. The current
planning baseline is **44/100**; hard-gate failures make the repository not
release-ready regardless of the aggregate.

| Dimension | Weight | Baseline | Evidence-led finding |
|---|---:|---:|---|
| Correctness and standards | 25 | 17 | Strong fixed differential/W3C coverage and sealed mapping inputs/runners; path truncation, one R2RML deviation, incomplete backend receipts, and no generative/fuzz layer |
| Security and governance | 20 | 8 | Bound parameters and partial timeout/pooling exist; no total budget, result/cost cap, TLS, identity, or policy enforcement |
| Federation and architecture | 15 | 3 | Reusable backend abstraction and semi-join cost model; runtime and mapping IR are single-source |
| Performance and boundedness | 15 | 10 | Strong measured simple-streaming and Ontop evidence; global sort/group/dedup retain source-sized state |
| Operability and reliability | 15 | 2 | No production config, telemetry, health/readiness, graceful shutdown, reload, or drift handling |
| Release and product evidence | 10 | 4 | CI/audit/harness exist; ignored app lockfile, broad binary closure, version 0.0.0, no product release proof |
| **Total** | **100** | **44** | **Target ≥98 and every hard gate green** |

Material gaps found directly in the current tree:

| Priority | Gap | Current evidence | Required disposition |
|---|---|---|---|
| P0 | Silent path truncation | `path.rs` fixes `PATH_MAX_DEPTH = 256`; recursive rows retain depth | Exact cycle-safe closure or explicit limit failure, tested beyond 256 |
| P0 | Bounded-memory invariant is too broad | `exec_core.rs` buffers global order, Rust grouping, solution/triple dedup | Composite SQL or bounded spill/merge; otherwise capability-profile `501` |
| P0 | Cross-source charter is not delivered | One `ServeConfig.backend`; no `SourceId`; semi-join planner has no production caller | Source registry, source-bound mappings, federated plan and coordinator |
| P0 | Reproducibility closure is incomplete | `93ae3c2` tracks `Cargo.lock`; `374ca99` pins actions, service images and selected tools; `31a1164` installs MetaHarness/Darwin only from the committed npm lock; the default `sf-cli` receipt now closes package resolution, features and dependency edges only | Capture actual binary bytes plus build-script, linker and system provenance; produce an SBOM and reproducibility/admission evidence; close the hosted-runner, apt-transitive and release-toolchain residuals; repeat the complete M0 gate in two clean checkouts |
| P0 | Standards evidence is not yet release-complete | `1c9bb61` seals the exact mapping suite and per-ID policy; `a3efb32` runs SQLite/PostgreSQL in sealed order; `33e202b` binds the SQLite mapping baseline; per-test expected SQLite query/protocol baselines now detect product regression without claiming W3C Query/Protocol conformance, runtime provenance or backend admission. Current PostgreSQL evidence still derives from the prior live baseline | Add a durable current required-live PostgreSQL receipt, add MySQL mapping coverage, and keep mapping/query/protocol evidence disjoint |
| P1 | Governance covers only part of a request | compile is unbounded; no result/byte/cost cap or common cancellation budget | One `QueryBudget` from ingress through serialization, every backend |
| P1 | Production secret/transport exposure | PostgreSQL `NoTls`; source secrets accepted in argv; parse errors echo conninfo | `SecretRef`, verified TLS, redacted errors and secret-corpus tests |
| P1 | Accepted runtime ADRs are not delivered | ADR-0011/0017/0018 have no production implementation | Implement or supersede with dated status/evidence |
| P1 | Test strategy is incomplete | no `proptest`, fuzz target, `insta`, durable coverage or mutation gate | Generated, fuzz, snapshot, mutation and LCOV trains |
| P1 | Serving artifact is too broad | `sf-cli` imports conformance/bench; conformance enables REST and SQL Server | Minimal serve artifact; opt-in evidence/developer features |
| P2 | Lifecycle and admission are absent | immutable-in-practice epoch; no reload/drift; `M ⋈ T` not a startup gate | Validated immutable snapshot, atomic swap, drift detection and readiness |
| P2 | Maintainability risk | compiler/executor files exceed the 500-line rule by several multiples | Characterize first, then decompose by stage without semantic rewrite |

`cargo audit` passed on 2026-08-26 with the configured exceptions and three
unmaintained-crate warnings. That is not a clean supply-chain verdict: the
exception register contains six advisories and must become reachable-feature,
owner, expiry, and compensating-control evidence.

## Domain model and target architecture

| Bounded context | Aggregate or port | Current home | Target responsibility |
|---|---|---|---|
| Semantic contract | `RuntimeSnapshot`, T-box, mapping IR, capability profile | `sf-core`, `sf-mapping` | Versioned T/M/schema/source identity and fail-closed validation |
| Query compiler | IQ, optimizer, dialect-neutral physical operators | `sf-sparql` | Exact supported-profile rewrite to single/federated physical plan |
| Source runtime | `SourceRegistry`, `SqlBackend`, backend capability contract | `sf-sql`, `sf-serve` | Source lifecycle, binding, streaming, cancellation, health and admission |
| Federation | `FederatedPlan`, fragment, reducer, global operator | new modules behind existing crates | Per-source fragments, bounded data movement, merge/spill and failure semantics |
| Request governance | `QueryBudget`, `SecurityContext` | `sf-serve` | Total deadline/work/result budget, identity, policy and safe public errors |
| Lineage | query receipt/provenance vector | `sf-sparql`, `sf-serve` | Mapping/source/row-key lineage without persisted A-box state |
| Operations | runtime config and lifecycle | `sf-serve`, `sf-cli` | Secrets, TLS, telemetry, probes, reload, shutdown and drift |
| Evidence and release | immutable evidence bundle | `sf-conformance`, `sf-bench`, CI | Standards, oracle, QE, load, security and exact-artifact proof |
| Engineering control | task contract and receipt | `coding-harness/`, Ruflo | Dual-host proposals/repair/verification; never product authority |

```text
HTTP / CLI
   │  SecurityContext + QueryBudget
   ▼
Query session ───────────────► CapabilityProfile (exact or reject)
   │
   ├── RuntimeSnapshot { T, M, schemas, epochs, digests }
   │                    │
   │                    └── SourceRegistry { SourceId → backend/capabilities }
   ▼
semantic compiler → federated physical plan
                         │
            ┌────────────┼────────────┐
            ▼            ▼            ▼
       source fragment  reducer   global bounded operator
            └────────────┴────────────┘
                         ▼
              reconstruction/serialization

Evidence plane: standards + differential + QE + load + release receipts
Engineering plane: Ruflo + native Codex/Claude MetaHarness (no promotion authority)
```

The dependency direction should move relational schema contracts out of
`sf-mapping -> sf-sql` into a neutral domain/port module. `sf-sparql` should be
decomposed by parse/build/normalize/lower/emit/execute only after the generated
characterization suite exists.

## Programme dependency graph

```text
M0 truth/evidence foundation
  └─► M1 exactness + bounded physical execution
        └─► M2 total governance
              ├─► M3 secure observable runtime
              ├─► M4 standards + generative QE
              └─► M5 snapshot lifecycle, policy + lineage
M1 + M2 + M4 ─► M6 cross-source federation
M3 + M4 + M5 + M6 ─► M7 minimal release + SOTA proof
```

M3 and M4 are intentionally parallel after the request-budget contracts settle.
M5 may start its snapshot model during M3, but policy/lineage promotion waits for
the relevant security and generated noninterference tests. M6 does not wait for
all UI/packaging work, but cannot start before exactness and backend contracts.

## Milestones and QA gates

### M0 — Architectural truth and deterministic foundation

Outcomes:

- publish a generated, dated capability/backend/standards matrix;
- distinguish `accepted` decision status from implementation status in every
  touched ADR;
- track `Cargo.lock`; use `--locked`; pin CI actions, installed tools, W3C suite
  inventory, fixtures, expected outcomes, skips, deviations, and spec snapshots;
- split RDB2RDF mapping conformance from SPARQL query/protocol evidence;
- **SPARQL baselines:** freeze per-test expected SQLite query and Protocol
  outcomes as regression receipts, without treating them as W3C conformance,
  runtime provenance, or backend admission;
- freeze controlled heap/RSS and latency receipts, and keep package dependency
  closure distinct from actual binary artifact and supply-chain closure; and
- write subordinate ADRs for the production artifact and the federated global-
  operator/spill choice.

QA gate:

- two clean checkouts resolve the same dependency graph and evidence digests;
- every public claim maps to a test/profile entry or is labelled planned;
- missing/malformed standards inputs fail; no new skip can hide behind a count;
- the programme backlog remains derived solely from the charter, source,
  decisions, standards and executable evidence.

### M1 — Exactness and bounded physical execution

Outcomes:

- replace silent 256-hop truncation with exact cycle-safe closure for each
  admitted dialect, or reject before returning a success response;
- lower global ORDER, GROUP/aggregate, DISTINCT and graph/CONSTRUCT dedup into one
  composite relational plan where sound;
- define the bounded external operator contract for shapes that cannot be pushed
  to one source, with no accidental in-memory fallback;
- close the PostgreSQL delimited-identifier deviation or retain it as an explicit
  release-profile exclusion; and
- characterize then split compiler/executor hotspots into files below 500 lines.

QA gate:

- chains and cycles at 1, 255, 256, 257 and >1,000 hops are exact or explicitly
  rejected, never silently partial;
- every advertised blocking shape has flat heap and RSS under 1×/10×/100× source
  growth; growth from 10× to 100× is within 10% after measurement noise;
- flat/tree/unoptimized/optimized/materialized-oracle results agree;
- unsupported-shape tests assert the exact pre-execution failure class.

### M2 — Total request governance and cancellation

Outcomes:

- introduce `QueryBudget`: absolute deadline, compile work, result rows/bytes,
  source cost, recursion, admission and stream-lifetime limits;
- bound blocking compiler work with admission and cooperative cancellation;
- apply source-native statement/transaction timeouts and capacity shedding to
  SQLite, PostgreSQL, and MySQL;
- propagate disconnect/cancellation to all tasks, streams and connections; and
- return safe RFC 9457-style problem details with correlation IDs, not internal
  driver/schema/credential strings.

QA gate:

- one deadline covers ingress, parse, compile, acquire, execute and serialize;
- timeout/disconnect releases worker and connection capacity within one second;
- overload sheds within the configured wait and never queues without a bound;
- row and byte caps work for every result format without producing valid-looking
  partial success.

### M3 — Secure, observable, operable runtime

Outcomes:

- implement typed layered configuration and redacted `SecretRef` resolution;
- add verified TLS for remote admitted sources and remove credentials from argv;
- implement ADR-0011 request/stage spans, bounded-cardinality metrics, structured
  logs and governance events using a pinned OpenTelemetry conventions version;
- add `/livez`, `/readyz`, service description, graceful shutdown and source
  failure policy; and
- define SLOs for latency, overload, availability, correctness and recovery.

QA gate:

- a seeded secret corpus appears nowhere in argv, logs, traces, metrics or errors;
- telemetry overhead stays below 5% on the controlled benchmark;
- no metric label contains query text, IRIs, source values or other unbounded data;
- readiness fails for an invalid snapshot or unavailable mandatory source;
- SIGTERM drains or cancels within the configured bound without connection leaks.

### M4 — Standards and generated QE train

Outcomes:

- make the exact W3C RDB2RDF inventory fail closed and run equivalent admitted-
  backend coverage for SQLite, PostgreSQL and MySQL;
- add a pinned supported-surface SPARQL manifest and full Protocol/content-
  negotiation tests;
- execute the real upstream-generated `M ⋈ T` closure, retaining synthetic units;
- add valid schema/data/R2RML/SPARQL generators, NoREC, MR1, cross-dialect and
  `spareval` properties with persisted shrunk counterexamples;
- add parser/rewriter/emitter/serializer fuzz corpora, SQL snapshots, mutation,
  workspace LCOV, Agentic-QE gap analysis, fault injection and backend contracts;
- make required live services fail closed in CI while retaining explicit local
  opt-in skips; and
- automate Criterion, concurrent ramp, slow client, disconnect, overload and
  one-hour soak tests on controlled runners.

QA gate:

- exact suite inventory, zero unexpected failures and zero new skips/deviations;
- at least 5,000 deterministic generated cases per PR and 100,000 nightly with no
  divergence, panic, unexpected unsupported result or nontermination;
- bounded fuzz smoke per PR, long persisted-corpus campaigns nightly, zero crash;
- ≥90% mutation score in critical translation/governance modules with no survivor
  that weakens binding, NULL/bag semantics, dedup, recursion or cancellation;
- ≥95% line/region coverage on critical boundaries and ≥90% workspace coverage,
  both ratcheted rather than reset;
- median regression ≤5%, p95 regression ≤10%, and no monotonic RSS growth during
  the soak. Variance above the harness threshold makes the run inconclusive.

### M5 — Snapshot lifecycle, identity, policy and lineage

Outcomes:

- build immutable `RuntimeSnapshot {T, M, schemas, sources, epochs, digests}`;
- validate `M ⋈ T` and source capabilities off-path, then atomically swap; existing
  queries retain their original snapshot;
- fingerprint source schemas, detect drift, invalidate affected plans, roll back
  invalid snapshots and expose readiness state;
- add provider-neutral `SecurityContext`, PostgreSQL transactional `SET LOCAL`
  RLS, portable ABAC/sensitivity enforcement and audit decisions; and
- implement opt-in query-time provenance using mapping/source/row-key and plan/
  policy hashes, never persisted instance data or source values in telemetry.

QA gate:

- invalid reloads never activate; valid reloads are zero-downtime and invalidate
  stale plans exactly once;
- drift is detected within the declared interval and blocks affected readiness;
- tenant noninterference and pooled-context cleanup pass across concurrency,
  cancellation and failure;
- provenance identifies expected mapping/source/row keys and remains bounded by
  returned results and the request budget.

### M6 — Charter-complete cross-source federation

Outcomes:

- add `SourceId` to mapping/source affinity and a capability-aware
  `SourceRegistry`;
- partition the physical plan into per-source SQL fragments and typed global
  operators;
- wire the existing semi-join cost model to IN/temp-table/Bloom reducers with
  skip-if-unselective behavior, streaming merge and bounded spill;
- define global bag/NULL/order/group/OPTIONAL/MINUS semantics, consistency model,
  partial-failure classification, shared cancellation and provenance vector; and
- admit cross-source shapes one by one through the capability profile.

QA gate:

- two-source and three-source differential results equal a trusted materialized
  reference for every admitted shape and failure schedule;
- coordinator heap/RSS is independent of source cardinality within the M1 bound;
- selective fixtures reduce transferred bytes by at least 80%; unselective
  fixtures correctly skip reduction and stay within declared overhead;
- cancellation reaches every source and spill artifact; no partial result is
  labeled successful.

### M7 — Minimal release and SOTA proof

Outcomes:

- split the production server from conformance, benchmark and experimental
  connector closures; give the application a non-zero semantic version;
- define supported platforms/backends, upgrade/rollback/runbooks and a non-root,
  read-only OCI reference deployment;
- gate licences, sources, duplicates, semver/API compatibility, Rust/npm audit,
  time-bounded advisory waivers, SBOM, checksums, signatures and SLSA provenance;
- reproduce the exact release from two clean builders and smoke the packed
  artifact against all admitted backends;
- re-run the fair Ontop 5.5-or-current benchmark, publish wins and losses, and add
  only profile-proven optimizer changes; and
- generalize `coding-harness/` from its single-task driver to reusable task contracts.
  Enable Darwin/GEPA only after 5+5 discriminating/sealed tasks and held-out gain.

QA gate:

- production dependency tree contains only admitted functionality;
- zero unwaived reachable critical/high vulnerability and every waiver has owner,
  dependency path, feature/target reachability, controls and expiry;
- signatures, SBOM and provenance verify independently; clean-machine smoke uses
  the exact packed digest;
- all hard gates pass, product readiness ≥98/100, and independent Codex and Claude
  verification agree. A harness diagnostic or model score contributes no points.

## Ruflo and MetaHarness execution model

For M0–M7 code changes, use the ADR-0037 control plane proportionally:

1. **Recall and route.** Search repository and user Ruflo memory; freeze task,
   standards, evaluator and route snapshots. Native subscription hosts only.
2. **Specify.** Researcher and domain architect write executable acceptance law.
   Security/performance specialists join only when their boundary is touched.
3. **Design duel.** Native Codex and Claude propose independently, cross-critique,
   synthesize once, and independently verify the design. Missing host fails the
   architecture gate rather than triggering an indirect-provider fallback.
4. **Parallel implementation.** One writer per isolated branch/worktree; tester
   owns protected evaluators; no writer may change sealed oracle inputs.
5. **Verify and repair.** Build the patched candidate, run applicable public,
   independent, live, mutation, security, performance and regression gates; allow
   bounded verifier-directed repair with full rerun, retry/breaker/cancellation.
6. **Integrate.** One integration owner applies accepted patches in dependency
   order and produces digest-chained evidence. Ruflo state proves coordination,
   not product correctness.
7. **Learn carefully.** Persist verified patterns/outcomes. Diagnostics remain
   read-only signals. This does not opt into the retrieval-policy flywheel. It
   stays off unless separately and explicitly activated after every gate passes;
   passing gates alone does not activate it. No evolution starts before the 5+5
   holdout and reward-hack controls pass.

Native Codex/ChatGPT and Claude subscription invocations have no project-imposed
provider-dollar spend ceiling. `subscriptionCostUsd: 0` records zero marginal
provider-API charge in the receipt/routing ledger; it is neither a budget or cap
nor a claim of unlimited subscription capacity. Task, turn, wall-clock, output,
concurrency, first-party rate-limit backoff, retry/repair, resource, and receipt
limits remain operational safety controls. Do not set or consume provider API
keys and do not route through OpenRouter. Native-provider exhaustion or
unavailability fails the affected gate closed; it never authorizes an indirect
fallback. References elsewhere in this programme to query “cost” or
`QueryBudget` govern source work and result resources, not model-provider spend.

Small documentation/status-only corrections use the same truth and review rules
without paying for an irrelevant full harness transaction. Product behavior,
security boundaries, evaluator law, or release controls always use the full lane.

The H0c V6 pass also exposed an engineering-efficiency target: its isolated Rust
verifier outputs peaked at about 35 GiB and the controller spent material time
rehashing them before sealing, although cleanup completed and all resource gates
held. A subsequent harness slice should benchmark and introduce immutable,
content-addressed Rust closure/build reuse plus per-lane digest manifests. It
must retain separate writable targets, verifier independence, exact input
bindings, fail-closed cache validation, and replayable receipts; speed or disk
savings cannot weaken those controls.

The 2026-08-28 generic MetaHarness diagnostic scores the repository root 75 and
`ready`, while scoring the private nested package 67 and `needs-work`. Source
inspection shows the nested delta is a monorepo/private-package classifier
artifact, not a product or harness gate: it misses root CI and treats deliberate
non-publication as a deficit. A regression test keeps `harnessFit` diagnostic-
only; no README keyword or dummy workflow is added to game the score.

## Non-goals unless the charter changes

- A-box materialization, ETL, persistent triple storage or CDC materialization;
- non-relational/file ingestion, RML/FNML/YARRRML, or external SPARQL `SERVICE`;
- admitting cloud/REST/ODBC/Oracle/HANA adapters from mocked happy paths;
- in-engine general result caching, a Kubernetes operator, or bespoke SDKs;
- ML/LLM query planning, GeoSPARQL, FTS, full OWL 2 QL, or other breadth without a
  named product need and differential/benchmark oracle;
- multi-node coordination inside the engine before replica-based deployment and
  the federated single-node coordinator demonstrate an actual scaling limit; and
- changing the semantic compiler merely to reduce file size. Decomposition
  follows characterization and preserves the proven algebra.

## Risk register

| Risk | Consequence | Control |
|---|---|---|
| Draft SPARQL 1.2 changes | Moving conformance target | Pin dated snapshot; publish delta; separate stable R2RML claims |
| Path/global-operator repair changes answers | New correctness regressions | Generated oracle, mutation and >256/cycle corpus before refactor |
| Federation becomes a rewrite | Schedule and semantic drift | Preserve compiler; introduce SourceId/registry/physical plan behind ports |
| Spill substrate conflicts with ADR-0006 | Hidden architecture reversal | Separate design-lock ADR and benchmark both implementation choices |
| Backend behavior diverges | One green dialect masks another | Shared backend contract plus fail-closed live matrix |
| Policy taxonomy is unavailable | Platform coupling or stalled delivery | Provider-neutral SecurityContext/Policy port and reference fixtures |
| High gates become flaky | Teams bypass evidence | Controlled runners, variance classification, quarantine with owner/expiry |
| Supply-chain exceptions become permanent | Known reachable exposure | Reachability proof, owner, controls, expiry and release review |
| Harness optimizes its evaluator | False programme progress | Protected inputs, independent product gates, sealed holdouts, reward-hack scan |
| Documentation drifts again | Misleading claims and planning | Generate capability/status tables from receipts; update ADR with each slice |

## Definition of done

The programme closes only when:

- every accepted ADR is implemented with current executable evidence or is
  explicitly superseded;
- every advertised query/profile/backend cell is exact, and every unsupported
  cell fails before a valid-looking response;
- bounded heap/RSS, total budgets, cancellation and overload gates pass for every
  admitted operator and backend;
- identity/policy/provenance, snapshot lifecycle and operability gates pass;
- cross-source differential, boundedness, reduction, cancellation and failure
  semantics pass;
- the minimal immutable artifact and its supply-chain/release evidence verify on
  clean machines; and
- the score is at least 98/100 with no hard-gate failure or inconclusive evidence.

Anything less may be a useful preview or interim single-source production
profile, but it is not the completed application described by the charter.
