# Application-completion programme

- **Status:** In progress — ADR-0055 v1 completion profile active
- **Date:** 2026-08-26
- **Updated:** 2026-09-12
- **Controlling decision:** [ADR-0055](../adr/ADR-0055-v1-product-completion-and-release-profile.md) (accepted v1 profile)
- **Historical programme:** [ADR-0038](../adr/ADR-0038-sota-application-completion-programme.md) (superseded; post-1.0 SOTA backlog retained)
- **Supporting decisions:** [ADR-0037](../adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md), [ADR-0039](../adr/ADR-0039-minimal-production-serving-artifact.md), [ADR-0040](../adr/ADR-0040-bounded-federated-global-operators-and-spill.md), [ADR-0048](../adr/ADR-0048-rust-production-and-node-evidence-runtime-boundary.md), [ADR-0049](../adr/ADR-0049-exact-recursive-property-path-fixed-points.md), [ADR-0050](../adr/ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md), [ADR-0051](../adr/ADR-0051-postgresql-16-public-observed-schema-profile.md), [ADR-0052](../adr/ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md), [ADR-0053](../adr/ADR-0053-grammar-coupled-sparql-parser-governance-and-process-isolation-fallback.md), and [ADR-0054](../adr/ADR-0054-bounded-stable-root-order-windows.md)

**Scope:** Repository source, tests, accepted ADRs, CI, and measured benchmark evidence; GitHub issues and pull requests are deliberately not programme inputs.

**Status note:** This programme now separates v1 product completion from the post-1.0 SOTA and advanced-assurance backlog. Reclassification is explicit; nothing moved to post-1.0 is relabelled implemented, supported, or complete.

## Remaining release gates (2026-09-11)

This is the finite completion ledger under ADR-0055, not a new programme. At the review baseline
`a09e772a66080002574b92804b3dda4b0e64c62a`, sixteen overlapping catalogue
limitations mapped to the six outcomes below; they are not sixteen equal tasks.
All remain open until their missing evidence is attached. The sole main integrator
owns each outcome; native read-only Sol/Sonnet review supports it. Command IDs
resolve to exact commands in [the catalogue](../../tests/capabilities/catalog-v1.json).
Existing passes qualify their recorded source/profile, not a later release candidate.

| Gate / required outcome | Existing public evidence | Missing closure / dependency / acceptance |
|---|---|---|
| G1 — finite admitted request governance: `l-query-budget`, `l-deadline-cancellation`, `l-sqlite-admission` | Shared deadline/admission, isolated parser, selected compiler controls, public caps, SQLite VM interrupt and native stop/recovery. Projection/UNION integration is at `a09e772`. G1a base/VALUES now has exact/N-1/every-stop and ordinary/bearer phase-refusal, duplicate/UNDEF cold/key-only warm and capacity-recovery evidence; its harness binds the exact resulting commit. G1b's DESCRIBE form now has a checked whole-block envelope, paid fresh-name collisions/output copying and exact ordinary/bearer pre-source/cold-warm recovery evidence; initial RDF-star now has per-invocation controls over recursive amplification, inventories, names, environments and VALUES/output growth, with exact/sticky-stop and ordinary/bearer phase-refusal/cold-warm recovery evidence. RDF-star realization now has paid template/name/recursive binding and guarded SubPlan remap operations, exact/sticky-stop parity and ordinary/bearer SELECT/CONSTRUCT/DISTINCT cold-warm recovery tests. Constant-DESCRIBE parser binders now have deterministic, semantically fresh names before accounting; forced-length, parsed/string cache reuse, isolated-worker exact cold/warm work and ordinary/bearer graph recovery tests cover the correction without changing defaults. G1b logical compiler/cache coverage is verified through cd5e3ec and the combined-profile acceptance below; G1c source/admission and G1d combined request qualification remain open. | Post-compile admission is integrated at `85fc1ef`. The `sqlite-worker-shutdown-20260912` acceptance uses a real held SQLite UDF to check forced cleanup success only after worker exit, timeout failure while held, retained request/connection capacity and exact recovery on the same connection; its exact-commit harness verdict is required. This proves ownership, not arbitrary UDF/VFS/I/O preemption. Finish source/recursive work and terminal cleanup ownership for admitted shapes, then prove ordinary/authenticated cold/warm exactness, early refusal and full capacity recovery at unchanged defaults. Use `cmd-query-budget-http`, `cmd-compiler-input-admission`, `cmd-sqlite-active-vm-identity`, `cmd-sqlite-connection-admission-backend`, `cmd-sqlite-connection-admission-serving`, `cmd-native-query-controls`. Compiler sub-outcomes are below; no research-grade CPU/allocator claim. G1c source preparation (`g1c-source-preparation-opus-20260915`, successor of the paused `controlled-source-preparation-recovery-20260913`, itself succeeded by `-20260915b` (Claude-only continuation, Codex subscriptions unavailable) and `-20260915c` (scope amendment for hook-managed `.claude/helpers/*` files rewritten out of band by the session's own version-drift monitor); receipts `49b9092`, `85fc1ef`, `c3d2ca3`, `02736d5` are reconciled): prepared branches are borrowed with base-first hidden-key views; logical-source, projection, reference, subplan and path metadata, DISTINCT proofs, lexical roles, per-arm layout/merge and reference validation are paid iteratively (1,024-level/128-KiB-stack and exact-N/N−1 checks); parameter binds, nested transfers, IRI/decimal/float/identity/natural-literal constants and decimal normalization use checked admission; numeric constant parses, SQL parser round trips, placeholder rebasing, synthetic/live nested catalogues, UNION/FROM/skeleton assembly, percent encoders, template copies are prepaid, and the reference-atom fallback copy is paid per node, source text and binding key (its owned payload is bounded by prior compiler admission, not paid per byte). Truthful accounting exposed two defects, both fixed without changing emitted values or defaults: parameter vectors were charged and reallocated per bind (quadratic; now one modelled doubling per power-of-two length) and the SQLite percent encoder emitted ~13 KB of SQL per column (~238 KB for one identity query; now the query-local native `__sf_percent_encode_v1`, which also made the public budget suite three times faster). Two further live paths that still reached an uncontrolled `SourceWork::new(None)` are now governed: the SQLite backend's column-metadata recovery (the owned worker's `column_meta` call into the declared-type walk and COLLATE-scan fallback always ran uncontrolled even when the request supplied a real control; the now-dead `recover_collated_decltypes` convenience wrapper is removed) and the reference-atom renderer's lexical-role resolution (`ref_atom::sql`'s decoded/natural/typed-column classification, previously via the raw `literal_roles::resolved`); both raw wrappers are `cfg(test)`-only. A cancellation test asserting zero `SourceWork` for a governed worker was asserting the absence of this very charge, which made it pass unchanged whether or not the fix was present; a sixth independent Opus review (digest 37337d3f, changes-requested) proved this empirically by reverting the fix and rerunning the whole evidence set green. The test is corrected to require a nonzero charge, equal for two equal-length SQL strings whose recursive iteration and callback-firing counts differ tenfold — this bound is proved by the same revert (it now fails without the fix, restored after confirming so). The same review found `literal_roles::resolved_controlled`'s threading at `ref_atom::sql`'s call site had no coverage that would catch a revert there either; `resolved_controlled` itself now has an exact/N−1 boundary test whose charge strictly increases with key count (proving the callee is genuinely governed, not a no-op wrapper), and `column_meta` gained its own direct exact-boundary test matching its sibling `result_columns_with_control`. Disclosed, not claimed: no test yet fails if only the one `ref_atom.rs` line reverts to `SourceWork::new(None)` while `resolved_controlled` stays correct — the call site is directly readable and simple, but not independently pinned by an assertion. Verified at this slice: 1,139 SPARQL, 270 SQL (both re-run after the above, including this slice's two new lib tests) and 374 serving-library tests, every serving integration target including `query_budget` (89/89), the catalogue checks (regenerated) and the serving CLI build. `LIMIT 0` is declared source-independent for every form, Rust-group plans included: it answers empty before any probe (library test: `COUNT(DISTINCT *)` over an absent source; serving refuses source-backed Rust-group plans with 501, so no public path changes). Raw `Plan::emitted` SQLite SQL now needs the query-local percent encoder on the executing connection. Still open in G1c: recursive condition/conjunction rendering (no depth guard exists), execution-side lexical decoding, MySQL's ~24 KB-per-column encoder at unchanged defaults, path-closure SQL assembly (`path_with_prelude`, `path_as_derived_table_sql`, the recursive `hop_sql` text build) which is unmetered although path metadata is paid, public cold/warm/two-source qualification beyond the declared checks, and `emit.rs` (3,600 lines) shedding its rendering into modules with that rewrite. G1d combined public/default-corpus acceptance (`g1d-combined-acceptance-opus-20260916`, base `4ef69eb9`): the admitted SQLite/PostgreSQL/MySQL and two-source paths, run together, are green with no new feature code. Checks: `sf-sql --lib` (270), `sf-sparql --lib` (1,139/4 ignored), a serving-library and thirteen-target integration bundle (`query_budget`, `describe_endpoint`, `lineage`, `endpoint`, `resource_admission`, `mysql_release`, `pg_rls_profile`, `post_body_admission`, `request_admission_config`, `sqlite_active_vm_cancellation`, `order_window_semantic_oracle`, `query_security`, `problem_details`; 374 library plus every integration target), `differential_pg_sqlite`/`differential_mariadb` (graceful no-op skip without a reachable live server, per their own documented CI contract — not evidence of a passing live comparison), the catalogue checks (regenerated), the serving CLI build, `fmt` and `git diff --check`. A `cargo test --workspace --locked` baseline (3,927 log lines, every crate and doc-test) and a `cargo clippy --workspace --all-targets` pass were also run manually as G1b's own acceptance precedent did (not encoded as harness checks): the full suite is failure-free; clippy has 20 pre-existing style warnings (needless-reference-deref, items-after-a-test-module, two functions over the 7-argument lint, one large enum variant) that predate this slice, are not correctness defects, and are not claimed fixed here. G2–G6 remain open. |
| G2 — admitted RDF/query exactness: `l-native-ref-witness-identity`, `l-path-resource`, `l-postgresql-synthetic-row-identity`, `l-describe` | Public SQLite/native numeric, text/CHAR, static-template, path and bounded one-hop DESCRIBE cases have scoped evidence. | Close declared natural/typed-template value construction, arithmetic, mixed-descriptor/pooled and base-resolved IRI cases; resolve the PostgreSQL synthetic/real-rowid boundary for the declared profile. Validate exact bags/NULL/identity and existing unsupported-shape rejection with `cmd-property-path-exact`, `cmd-property-path-key-equality`, `cmd-postgresql-rowid-boundary`, `cmd-describe-compile`, `cmd-describe-sqlite`, `cmd-native-query-profile-live`. Do not add unadvertised DESCRIBE breadth or remove a promised feature. G1 controls must cover these admitted paths. G2 scoping (2026-09-16): of the six named acceptance commands, five pass as declared (`cmd-property-path-exact` 13/13, `cmd-property-path-key-equality` 2/2, `cmd-postgresql-rowid-boundary`, `cmd-describe-compile` 20/20, `cmd-describe-sqlite` 13/13); `cmd-native-query-profile-live` fails: `SELECT ?x WHERE { VALUES ?x { 9007199254740992 9007199254740993 } FILTER(?x > 9007199254740992) }` (the 2^53 float-precision boundary) returns HTTP 429 against the real live-server harness (`sf-cli`'s `source_tls_live` Docker-backed test) with an empty response body, though the identical query passes at the library/in-process level (`sf-sparql`'s `e2e::values_filter_on_const_var_compares_numeric_values` — `query_budget::mixed_identity`'s 19/19 is a related but non-identical column-vs-constant shape, not this VALUES constant-vs-constant one). An independent review found my first pass wrong here: `source_control::numeric_lexical` IS reachable for this exact query, via `emit.rs`'s `render_cond_controlled` calling `literal_cmp::render_controlled` on `SqlCond::LiteralCmp` for a `base_numeric()` operand (`xsd:integer` matches), and `render_controlled` is itself new in the G1c commit — confirmed by an `eprintln!` probe firing on both literal operands for this query. That correction is accepted. But I then ran the decisive experiment the reviewer could not (permission-gated for a read-only review): neutralizing `numeric_lexical` to a no-op and rerunning `cmd-native-query-profile-live` produces the byte-identical 429, at the same line, for the same query. G1c's charge is reachable but not causal; ruling it out required running the test with it disabled, not reasoning about reachability alone. Traced the exact mechanism and confirmed it definitively (2026-09-16): the 429 is `QueryControlError::CompilerWorkExceeded`, not a `SourceWork` charge at all — confirmed via server-subprocess stderr capture (`sf_core::query_control::QueryBudget::consume_with_hook`'s limit-check branch) showing `charge=CompilerWork amount=517120 current=531736 next=1048856 limit=1000000`. The 517,120-unit charge traces through `IqNode::Filter` → `apply_owned_conds` → `lower_owned_iq_cond` → `CompileContext::filter_condition` to `reserve_filter_work` (`crates/sf-sparql/src/compiler_control/filter_work.rs:125-134`), whose documented formula `passes = (expression+1)*(bindings+1)*128; reserve_checked_sum(&[1024, passes])` evaluates to exactly `64*63*128+1024 = 517120` for this query's measured `expression=63, bindings=62` (both individually small and scaling linearly with the literal's digit-length — no runaway or value-dependent blowup in either factor). This is the deliberate, documented worst-case FILTER cost model (module doc: "128 passes plus 1024 fixed logical units are not a physical heap bound") operating exactly as designed; it is DEFINITIVELY NOT a code defect anywhere in the chain. Confirmed separately: `max_compiler_work` has zero configuration pathway anywhere in the codebase (no `--max-compiler-work` CLI flag, no `ServeConfig` setter — unlike `max_source_work`/`max_result_items`, which both have one), so "raise the live-server test's limit" is not a narrow test-fixture change but would require adding new public CLI/config surface for a value that may be deliberately fixed as the one non-operator-tunable algorithmic-complexity envelope. Closing this item requires a product/security decision among three disclosed options, not a unilateral code change: (a) add `--max-compiler-work` CLI/config surface; (b) recalibrate the 128x/1024 constants in `reserve_filter_work` (workspace-wide FILTER cost change, requires updating the module's own documented bound); (c) accept current behavior as correctly-governed and adjust the test/mapping fixture's literal magnitude (reduces this acceptance command's coverage of exact large-integer arithmetic at the 2^53 boundary, which is its declared purpose). No path has been chosen. **Update (2026-09-16, user-authorized path (a), committed `8532606b8a28106b3053ecfdad54b9dcc0926ab6`):** added `--max-compiler-work` as a real CLI flag and TOML/env config option, mirroring `--max-source-work`/`--max-result-items` exactly; this also fixed the actual bug behind the gap (`crates/sf-serve/src/startup.rs`'s `configure()` previously read `config.query_limits.max_compiler_work()` back into itself, a no-op self-assignment that silently discarded any attempt to change it). CLI default is unchanged, so no operator sees any behavior change unless they explicitly pass the new flag. Raising the `query_profile` test group's own limit to 100,000,000 fixed two of the three known 429s in the live Docker-backed acceptance test: the original 2^53-boundary case, and a previously-masked ~400-digit decimal literal case that hit the identical ceiling but was never reported because the test aborts on its first failure. **G2 is still NOT closed**: past those two, the same test now reaches a third, previously-unreachable, unrelated case — a 94-item `VALUES` list combined with `FILTER EXISTS { ... FILTER(?o = 1.00) }` — where the server starts a `200 OK` chunked response that cuts off before the terminating chunk. Confirmed via a diagnostic-only, fully-reverted experiment that this is NOT a `--timeout-secs` issue (fails identically at both 2s and 15s). This turned out to be a genuine, previously-masked architectural inefficiency, not a streaming bug: an adversarial review (2026-09-16, user-directed "do the real fix") traced it to `SourceWork` correctly charging for MySQL's `percent_encode_col_mysql` re-embedding the column-reference SQL expression at ~40 byte-range/length checks (measured `repeats=148` for MySQL vs `2` for PostgreSQL and `1` for SQLite, for the identical semantic check). Fixed by binding the BINARY cast once via a derived table (`FROM (SELECT CAST({col} AS BINARY) AS b) AS pre`) and referencing the short alias everywhere, dropping MySQL's `repeats` to `1`; verified byte-identical encoded output before/after against a live MySQL 8.4 instance across an exhaustive case set, independently reproduced by a second reviewer via its own old-vs-new A/B equivalence check (138 cases, all matched), committed `80fd022e73250677516e14a34aa1486ae3c69fd2`. Combined with `--max-compiler-work` (committed `8532606b`) and `--max-source-work` raised to 20,000,000 for this test group (covering PostgreSQL's own legitimate per-operator scaling, unrelated to the MySQL fix), the full live Docker-backed acceptance test `native_describe_and_recursive_paths_are_exact` passed end-to-end (0 failed) on two separate runs (696.88s and ~700s). **Correction, same day: an independent reviewer reran the identical test against the identical committed state and got a genuine failure** (163.92s, panicking at the same 94-item VALUES + `OPTIONAL{decimal FILTER + marker}` query, the same truncated-chunked-response symptom as before) — two passes and one failure on byte-identical code and environment class is intermittent failure, not a fixed test. **G2 is NOT closed.** The SourceWork-overage explanation and the MySQL percent-encoder fix are still correct and verified (five of six commands, plus the SourceWork-specific failure mode, are solid), but this specific query shape has a separate, unresolved, load/timing-sensitive failure mode once it's allowed to actually execute (rather than fail fast on an undersized budget) that needs its own investigation before `cmd-native-query-profile-live` can be called reliably passing. **Update: 2 additional same-day reruns, both pass** (673.20s, 664.58s), joining the earlier 696.88s and ~700s passes **against the single earlier failure** (163.92s) — tally now 4 pass / 1 fail across 5 runs of byte-identical code. The one failure is an outlier in both directions at once: it's the only failure AND its runtime (163.92s) is roughly a quarter of every passing run's (664-700s), consistent with an early truncation under transient load on this shared, multi-tenant environment (dozens of concurrent unrelated Docker containers observed) rather than a data-dependent cost edge case recurring on this specific query. This shifts the balance of evidence toward shared-environment contention over a genuine intermittent product defect, but does not settle it: a single confirmed failure in 5 runs is still real and unexplained, and a dedicated, unshared CI runner sampling this test many more times (ideally alongside system load metrics at the moment of any failure) remains the fastest way to be sure before treating `cmd-native-query-profile-live` as reliably passing. **Root cause CONFIRMED via direct instrumentation (2026-09-19), not further inference:** the single earlier failure (163.92s) was `crates/sf-cli/tests/source_tls_live/query_profile.rs::start_command`'s own `--timeout-secs 2` — a single absolute per-request wall-clock deadline (`sf-serve`'s `RequestBudget`, `crates/sf-serve/src/budget.rs`) that can fire mid-stream, after HTTP 200 headers are already committed (`crates/sf-serve/src/stream.rs`), truncating the response with no way to send a typed error. Proven, not theorized: an env-gated per-row streaming delay (temporary diagnostic, fully reverted before the fix) was run against the real, unmodified `--timeout-secs 2`; it reproduced the exact same truncated-response symptom, and the server itself logged `reason=DeadlineExceeded` at the moment of truncation — this rules out `SourceWork`/`CompilerWork` or any other governed-resource dimension as the cause of this specific failure, distinct from (and not contradicting) the separately-confirmed MySQL percent-encoder `SourceWork` fix above. **Fixed, committed `62646689ab8a923dc8e7ca21f561134825dcc75d`** (task `g2-query-profile-timeout-fix-opus-20260919`): raised `--timeout-secs` to 15 with a comment citing the confirmed mechanism and the measured ~661-766ms worst case (this suite's heaviest streamed query, the 94-item `VALUES` + `OPTIONAL` policy query in `pg_decimal_text.rs`), giving roughly 20x real headroom rather than leaving an uncommented, unexamined 2s default. Independent review (fresh subagent, distinct executorId, ACCEPT first round) independently re-derived the mechanism from `budget.rs`/`stream.rs`, found an additional corroborating pre-existing unit test (`terminal_body.rs`'s `full_unpolled_channel_finishes_at_deadline_then_drains_prefix_error_and_fuses`, which already covers exactly this post-200 mid-stream-failure shape), independently reran the live acceptance test itself rather than trusting the prior run (134.02s, passed), and confirmed no sibling test module depends on this specific timeout for cancellation/deadline behavior. **`cmd-native-query-profile-live` now passes reliably: 2/2 full `native_describe_and_recursive_paths_are_exact` reruns since the fix** (680.64s, 700.56s — consistent with the pre-fix passing-run baseline of 664-700s, exactly as expected since the fix only changes behavior for requests that would have hit the old 2s ceiling), plus 2 additional targeted reruns of the specific previously-failing query during harness verification and independent review (137.11s, 134.02s). Unlike the earlier withdrawn premature "G2 fully closed" claim in this same cell (which rested on two passing samples with no understood mechanism, and was correctly overturned by an independent reviewer's genuine failure), this closure rests on a confirmed, instrumented causal mechanism plus a fix that targets that exact cause — a materially stronger evidentiary basis, not a repeat of the same mistake. **G2 is now CLOSED**: all six named acceptance commands pass (`cmd-property-path-exact`, `cmd-property-path-key-equality`, `cmd-postgresql-rowid-boundary`, `cmd-describe-compile`, `cmd-describe-sqlite`, `cmd-native-query-profile-live`). A future run showing the identical truncated-response symptom against the new 15s ceiling would be genuinely surprising given this evidence and should reopen this item rather than being dismissed as more of the same intermittency. Separately noted, not yet scoped or decided: the same mid-stream-deadline-after-200-commit behavior is a general `sf-serve` architectural property, not test-specific — once HTTP 200 headers are sent in production, a later failure cannot become a clean error response either — which is a legitimate open question for a future item, not addressed by this fix. |
| G3 — coherent generations/semantic admission: `l-metadata-toctou`, `l-schema-lifecycle`, `l-semantic-admission-scope`, `l-snapshot` | Immutable reload leases and protected PostgreSQL Direct/authored generations are public; invalid generations fence readiness/new requests. | Complete required backend/policy generation and DDL-race guarantees, binding M ⋈ T/source validation and cache identity to the same lease. Run `cmd-runtime-snapshot-state`, `cmd-runtime-snapshot-http`, `cmd-runtime-snapshot-body`, `cmd-public-authored-generation-live`, `cmd-public-direct-lifecycle-live`, `cmd-semantic-admission-runtime`, `cmd-semantic-admission-validation`, `cmd-semantic-admission-mapping`. Existing profile exclusions remain explicit; no new general ABAC or policy-hot-reload programme.  **2026-09-20 scoped acceptance (`g3-admission-reload-20260920`):** `runtime_activation_http_tests::semantic_admission_and_warm_plans_stay_with_the_request_generation` uses real observed SQLite integer/text schemas and exact duplicate typed HTTP bags. It proves shared-plan cache reuse, semantic rejection of an incompatible successor, explicit not-ready refusal, distinct admission/ontology identity for a valid successor, bidirectional cross-binding plan rejection, and old pre-body request completion after new-generation cold/warm requests. This qualifies application snapshot/admission/cache isolation only; readiness fencing here is explicitly invoked, not a backend drift detector. Ordinary SQLite/MySQL and other required backend/policy DDL-generation guarantees remain open. **SQLite implementation (`g3-sqlite-generation-qualified-20260920`):** the opt-in authored file-backed WAL/DELETE path now uses sealed read-only members and one transaction from bounded schema observation through compilation, all response forms and acknowledged cleanup. Backend tests cover queued workers, DDL snapshots, cancellation/poisoning and reuse; serving tests cover exact binding, protected execution despite a poisoned legacy handle, cold/warm typed bags, stale-generation refusal and single/multiple-mapping lineage. `cargo test --locked -p sf-cli --no-default-features --test sqlite_generation` covers authenticated actual-CLI queries, semantic incompatibility, automatic reload/recovery, source-work refusal and shutdown in both journal modes. Final task verification and independent review gate the slice commit. Protected authored portable policies now admit bearer and all-portable registries on SQLite/PostgreSQL; native RLS/mixed registries and Direct row policies remain excluded. Uncached preflight authorizes before shape/source work; authoritative compilation reapplies policy under the lease. Candidate validation rejects missing mapped policy columns without rejecting intentionally uncovered subjects. Actual WAL/DELETE and pinned PostgreSQL 16.9/16.15 TLS CLI cases prove cold/warm A/B isolation, mapping reload, repeated policy-column drift fencing/recovery, zero-source-work or held-lock denial, and PostgreSQL pinned A-only stream/disconnect cleanup. Use `cmd-protected-portable-generation-cli` with the SQLite CLI/serving checks. Ordinary modes, MySQL, native PostgreSQL RLS and federated generations remain open; G3 is not closed. |
| G4 — declared cross-source execution: `l-federation` | Two-source UNION and fixed-cap two-pattern join, actual lineage, native stop/sibling isolation/recovery are implemented. | Qualify source consistency, protected generations and every admitted backend combination for those public shapes; depends on G1–G3. Run `cmd-federated-union-sparql`, `cmd-federated-union-serve`, `cmd-federated-union-cli`, `cmd-federated-join-sparql` and their required owned mixed-source TLS cases. No Bloom/spill/global-algebra research prerequisite. |
| G5 — required native transport/live matrix: `l-transport-security`, `l-live-optional` | Verified remote TLS and native authentication/exactness suites have owned-fixture evidence. | Run the required fail-closed matrix against the immutable candidate after G1–G4; record exact artifact/profile/log digests and resolve candidate-specific failures. Use `cmd-verified-source-tls`, `cmd-verified-source-tls-cli`, `cmd-verified-source-tls-live`, `cmd-native-query-profile-live`. The TLS/authentication selector is not numeric-profile evidence. No new Product Mock/live-database access is implied. |
| G6 — exact candidate/admission verdict: `l-release-artifact`, `l-production-admission` | Rust-only serving boundary and versioned developmental non-root/read-only image smoke exist. | After G1–G5, one immutable candidate passes the full locked workspace and serving-only checks, backend/profile matrix, clean-machine smoke, licence/advisory review, SBOM, checksums, signature/provenance and independent native Codex/Claude exact-delta review. Use ADR-0055 §3 and M7/Definition of done below; record the actual commands/digests. These are aggregate gates, not two more feature programmes. Publication requires separate current approval. |

G1's finite accounting sub-outcomes are: **G1a**, base/VALUES controls now have
independent phase-cut and public exactness/recovery evidence; **G1b**, verified logical
compiler/cache coverage through the integrated phase chain and combined acceptance
below; **G1c**, close post-compile admission, source/recursive work and cleanup ownership for
the admitted SQLite/PostgreSQL/MySQL and two-source paths; **G1d**, run their combined
public/default-corpus acceptance and record the profile verdict. Before G1b edits,
enumerate the remaining phase entry/exit boundaries and exact missing proof once.
Group compatible work under that outcome; do not turn each next unmetered helper
into a new completion prerequisite or require `GovernedV1` merely as a label.
This inventory is not proof of closure, and known failures are not scope exclusions.

**G1b RESOLVE slice (2026-09-11, integrated/verified at `308d75b`):** the completed
`controlled-resolve-finalize-20260911` delivery task covers recursive traversal,
bridge/output inventories, D1/D2 and shared-dedup markers. D1/D2 now share one
controlled unique schema map; ADR-0055 records the explicit duplicate-name
validation tightening. Focused ordinary/bearer HTTP tests observe the actual
RESOLVE span and prove pre-source refusal, exact duplicate-preserving cold and
key-only warm bags, and compiler/source capacity recovery. Direct schema tests
cover exact/N-1, comparison/relocation cancellation and deadline, and retained
source-type authority through D1/D2. All ten declared checks and independent native
Sol review passed; catalogue evidence and the exact commit are harness-verified. G1 remains open.

**G1b finalization slice (2026-09-11, integrated/verified at `87f48a7`):** `controlled-finalization-alias-20260911` covers aggregate wrapper preparation, nested candidate rollback/projection/depth, graph retention and deterministic dedup-scope lifting. Focused ordinary/bearer SELECT, aggregate, UNION and CONSTRUCT tests prove phase refusal before held source admission, exact cold/key-only warm results and capacity recovery. Regression tests exposed and corrected random aggregate-parser work and repeated-alias output loss; ADR-0055 records their semantic boundaries. All ten declared harness checks and independent native Sol review passed; the harness verified the exact integrated commit. At that boundary optimizer internals and acquired-cache operations remained open; neither G1b nor G1 was closed by those focused results.

**G1b combined logical compiler/cache acceptance (2026-09-12):** the phase chain is integrated: RESOLVE `308d75b`, finalization/aggregate parser `87f48a7`, optimizer `edfe39c`, acquired cache `cd5e3ec`, with earlier BUILD/NORMALIZE/LOWER/RDF-star/DESCRIBE controls retained. Independent native Sol inspection of `translate_tree_with_column_type_use` confirms one metered context reaches every admitted form, root/nested cascade and finalization; raw APIs are separate compatibility surfaces, not v1 prerequisites. At `cd5e3ec`, 1,053 compiler, 500 serving/HTTP and 245 differential tests pass, plus serving CLI build, full workspace/all-targets clippy, catalogue, parser and harness checks. Exact/N-1/sticky stops, actual-stage refusal, ordinary/bearer cold/warm exactness and unchanged default-corpus cases cover the combined profile. `g1b-compiler-profile-acceptance-20260912` binds the combined compiler/serving rerun and final review to this reconciliation; its exact-commit verdict is required evidence. G1b logical compiler/cache coverage is closed, not all G1: the then-unmetered post-compile resource scan belongs to G1c, alongside source/cleanup ownership. The `controlled-resource-admission-20260912` slice now carries shared work control through root ORDER checks and a one-pass nested classifier inside the same compiler worker. Public ordinary/bearer admission-tail refusal, duplicate/UNBOUND recovery and cumulative two-fragment rejection are its acceptance checks; raw-profile parity, exact/N-1, every-charge cancellation/deadline and depth checks cover the classifier. Its exact-commit harness verdict remains required. Physical allocation/last-Arc destruction and G1d end-to-end candidate qualification remain open. No `GovernedV1` label is required or activated.

**Execution correction:** after G1a's source-bound verification/commit, take G1b's
one-time missing-phase/proof inventory and the smallest unblocked gate
outcome above. Audit prerequisite tests before freezing each harness scope; preserve
negative tests' intended phase rather than raising defaults or measuring away the
failure. Keep exact prospective controls at fanout, copy and retained-state growth
boundaries. A conservative phase envelope is an option only with checked finite
inputs, a bound covering its entire admitted work, bounded cancellation checkpoints
and unchanged default-corpus acceptance; it is not a substitute for source controls
or a permission to delete existing checks. Update one authoritative gate row and
only ADR decisions/status that actually change, not four implementation diaries.

**Forecast and stop rule:** six recent completed outcome families took 168.3 minutes
first-start-to-finish wall time; recorded checks took 10.6 minutes (6.3%). The remainder
is not all overhead: implementation, review and interruptions were not separately
timed. Nine recent commits touched 177 files, 61 of them ADR/catalogue/generated
evidence; file touches are not effort measurements. No whole-programme ETA is justified
until G1c/G2/G3 remaining proof work and candidate matrix runtimes are measured.
A correction has not worked merely because another helper/receipt is green: compare
the remaining named obligations and actual user-visible acceptance. Once G1–G6 pass
on the candidate, stop completion work and recommend ending the six-hour reviews.

## Historical implementation evidence

The prior queue and detailed H0/M0–M7 implementation diary remain locally in Git:
`git show a09e772:docs/plans/sota-application-completion-programme.md`.
That local commit is not asserted to be published. Historical passing/failing
receipts keep their exact claims; they are not current release gates or estimates.

## Outcome

Complete and release semantic-fabric v1 as a virtualisation-only knowledge graph over live relational systems of record. Completion means exact-or-fail semantics, bounded execution, real cross-source query execution, production security and operability, and the ADR-0055 minimum release evidence. The wider SOTA research and advanced-assurance programme resumes post-1.0.

The semantic compiler does **not** need a wholesale rewrite. Three evolutionary
product changes remain on the v1 critical path:

1. lower every advertised global operator to a bounded physical execution path;
2. finish total request controls, public security, reload/drift and operability;
3. complete source identity, the registry, and an admitted federated plan.

If the federation item is removed, the application charter must change from systems of record/cross-RDBMS federation to one source per deployment. This programme retains the accepted charter.

## Historical SOTA baseline

This frozen 2026-08-26 assessment is retained for audit and has not been
rescored. Its **44/100** and former 98-point target are post-1.0 SOTA diagnostics,
not v1 release authority. ADR-0055's explicit product and release gates control.

| Dimension | Weight | Baseline | Evidence-led finding |
|---|---:|---:|---|
| Correctness and standards | 25 | 17 | Strong fixed differential/W3C coverage and sealed mapping inputs/runners; path truncation, one R2RML deviation, incomplete backend receipts, and no generative/fuzz layer |
| Security and governance | 20 | 8 | Bound parameters and partial timeout/pooling exist; no total budget, result/cost cap, TLS, identity, or policy enforcement |
| Federation and architecture | 15 | 3 | Reusable backend abstraction and semi-join cost model; runtime and mapping IR are single-source |
| Performance and boundedness | 15 | 10 | Strong measured simple-streaming and Ontop evidence; global sort/group/dedup retain source-sized state |
| Operability and reliability | 15 | 2 | No production config, telemetry, health/readiness, graceful shutdown, reload, or drift handling |
| Release and product evidence | 10 | 4 | At the frozen snapshot, CI/audit/harness existed but the app lockfile was ignored; broad binary closure, version 0.0.0, and product release proof were absent |
| **Total** | **100** | **44** | **Historical SOTA target ≥98; not a v1 gate** |

Material gaps found directly in the current tree:

Former P0 constraint-authorized serving drift is closed by `24a0e20` for the authored-R2RML lane. A typed `CompilerSchema` strips unverified PK, UNIQUE, FK, functional-dependency and NOT-NULL proofs before compiler/cache construction. The 2026-09-02 extension adds non-forgeable `ColumnTypeAuthority::Unverified`: cached serving cannot use mutable startup types to prove positional PostgreSQL pooling across different physical columns, and missing facts fail closed. Poison controls cover stale constraints plus frozen/cached type-authority counterfactuals. This deliberately forgoes key/FD/type optimisations: D1 deduplicates conservatively; each fallback arm captures its full BGP-boundary key, including an active graph variable, before projection, remaps it only through physically key-preserving pure unary wrappers after all rewrites, and overlays it on a private execution clone. Joined/OPTIONAL/path/aggregate, modifier-bearing or multi-branch nested wrappers, key-dropping nested projections, orphaned/multiply owned markers and groups with fewer than two executable arms return `501`; runtime repeats the shape proof before I/O. Fully ground overlapping arms instead use an exact SQL unit-relation pool. Serving rejects the remaining source-sized fallback before I/O.

Live execution now recursively probes base Table/Query sources, overlays those catalogs for base-source references in nested SubPlans, rejects missing, duplicate or ambiguous result columns, allocates fresh aliases above nested IQ/SQL uses, validates and emits every branch before opening any cursor, and maps post-commit SELECT/CONSTRUCT executor failures to one stable body error. Offline/synthetic alias emission and translate-time immediate wrappers retain the bounded, non-SQL-token-aware lexical heuristic; it is never live metadata authority. Conformance `rr:sqlQuery` metadata errors are no longer ignored. Compiler controls preserve the synthetic Direct-Mapping `rowid` name across Table→Query and read PostgreSQL `ctid` only at a base table, but a real `rowid` collision and snapshot-local CTID keep the no-PK path non-authoritative. Shared fail-fast admission now bounds application work before Router/body polling; its permit follows active internal workers instead of completed response bytes. Each physical serving SQLite connection also has a permanent cap-one async identity. The public PostgreSQL PK-backed lifecycle now rebuilds coherent rich observations off-path and fences/heals through one control coordinator; request paths never fence. Raw/foreign SQLite mutex access, general lifecycle, no-PK identity and production admission remain open.

| Priority | Gap | Current evidence | Required disposition |
|---|---|---|---|
| P1 | Recursive-path resource qualification | `5c379f6` computes an exact finite-pair fixed point beyond 256 and rejects unproved dialects; serving counts observable probe/open/pull attempts, not source rows or recursive iterations; owned SQLite interrupts only active VM work | Charge recursive/source work to total `QueryBudget`; qualify a common source-native cancellation contract before backend admission |
| P0 | Bounded global operators are incomplete | `639134d` rejects source-reading fallbacks pre-I/O; ADR-0054 qualifies finite root variable-key ORDER; `43399ce` adds only streaming two-source `UnionAll`, which is non-blocking | Composite SQL or accepted bounded spill/merge for GROUP, DISTINCT, graph dedup, wider ORDER and cross-source blocking shapes; retain `501` until each is proved |
| P0 | Cross-source charter is not delivered | Immutable snapshots now serve exact two-source UNION and a fixed-cap two-pattern join, with conservative key reduction, exact RDF set/bag semantics, pre-200 failure and required SQLite/encrypted PostgreSQL/MySQL public evidence; native guards now protect fragments; the pinned native cancellation matrix now passes; wider backend/profile combinations, protected backend generations and release admission remain open | Extend the physical algebra one exact bounded shape at a time; qualify consistency, failure, cancellation, performance and the full backend matrix without accepting proposed ADR-0040 implicitly |
| P0 | Minimum release closure is incomplete | `Cargo.lock`, pinned inputs and non-authorizing diagnostic closure evidence exist; no exact v1 artifact bundle exists | Produce the minimal Rust artifact, SBOM, licence/advisory disposition, checksum, signature, provenance and clean smoke required by ADR-0055. Complete dynamic closure, witnesses and two-builder identity remain post-1.0 |
| P0 | Standards evidence is not yet release-complete | Backend-aware v5 receipts bind all 87 ordered SQLite, required-live PostgreSQL, and required-live MySQL mapping outcomes. MySQL records 74 pass, one documented deviation and 12 exact typed unsupported outcomes under its conformance-only SQL-2008 profile; provider provenance is unbound and zero production admission remains explicit. Per-test SQLite query/protocol baselines and the exact one-target-expression, one-hop SQLite DESCRIBE endpoint detect regression without claiming full W3C conformance | Add pinned supported-surface SPARQL/Protocol manifests and wider DESCRIBE qualification; keep mapping/query/protocol, provider provenance and backend-admission evidence disjoint |
| P1 | Governance covers only part of a request | One serving identity spans the deadline, fail-fast aggregate active-work admission, and observable source/result/byte work. Its default 64 is finite governance, not capacity evidence; active internal workers retain the permit. SQLite adds cancellable per-member admission and active-VM interruption. Fairness/bounded waiting, raw mutex and submitted/running-work cancellation, busy/UDF/VFS/I/O, compiler CPU, database/recursive work, raw/conformance, full native cancellation/admission qualification and atomic post-`200` responses remain outside total governance | Measure and configure the gate per deployment; extend the same control into total `QueryBudget` and native cancellation for every backend |
| P1 | Production secret/transport exposure | `484a4b4` adds bounded redacted `SourceRef`, exclusive `--source`/`--source-env`, typed driver parsing and pre-I/O inline-password rejection. Typed bounded TOML/environment/CLI layering now carries only secret references through the same redacted startup boundary. Remote PostgreSQL/MySQL certificate/name verification, bounded private roots and same-policy PostgreSQL cancellation have required peer/CLI tests. Required digest-pinned live CLI tests cover both encrypted backends and their mixed UNION. Exact-release-artifact qualification, a direct external secret-store protocol, metrics/OTLP and broader secret-corpus coverage remain open | Exact-artifact TLS qualification, complete telemetry and secret-corpus tests |
| P1 | Accepted runtime ADRs are not fully delivered | ADR-0011 now has typed layered startup configuration, fixed health/readiness probes, bounded signal shutdown and the exact partial structured-tracing slice described above, but still lacks full metrics/OTLP, SLO/overhead qualification, exact-artifact TLS qualification, source-health policy and adapter-internal `sf-sql` spans; ADR-0017/0018 remain incomplete | Implement or supersede the remaining clauses with dated status/evidence |
| P1 | V1 release-profile tests are incomplete | The exact 5,000-case SQLite SELECT train is integrated; the dated advertised/live release matrix remains open | Close the focused v1 matrix; schedule the nightly 100,000, broad NoREC/MR1, long fuzz/shrinking and global coverage/mutation trains post-1.0 |
| P1 | Serving artifact is too broad | `sf-cli` imports conformance/bench; conformance enables REST and SQL Server | Minimal serve artifact; opt-in evidence/developer features |
| P1 | Remaining lifecycle and admission work | Immutable snapshots, request leases, fixed probes, bounded shutdown and authored-R2RML `M ⋈ T` admission exist. The public closed PostgreSQL-16 Direct profile now builds fully validated candidates off-path, compares the rich qualified generation on a distinct max-size-one control pool, fences only completed control failures, retries while not ready, activates by full-state CAS and fails closed on worker loss. Exact-patch public CLI startup and fixed-policy authored reload are qualified; wider lifecycle and production admission remain open | Preserve the public closed-profile evidence; close remaining required compiler/source controls and exact release gates without weakening the sealed boundary |
| P2 | Maintainability risk | `exec_core`, `build`, PostgreSQL introspection and the normalizer are characterized/decomposed; every `iq/normalize` production/test file is below 500 lines. Eighteen product-source files remain above 500 lines: 12 in `sf-sparql` (`iq/lower.rs`, `cascade/mod.rs`, `unfold.rs`, `emit.rs`, `unify.rs`, `lib.rs`, `iq/resolve.rs`, `iq.rs`, `cascade/joinelim.rs`, `path.rs`, `leftjoin.rs`, `cascade/ws_st.rs`), four in `sf-sql` (`backend/rest.rs`, `backend/pg.rs`, `backend/sqlserver.rs`, `backend/monetdb.rs`) and two in `sf-mapping` (`r2rml.rs`, `direct_mapping.rs`). `sf-bench/workload.rs` and multiple test/evidence files are also oversized | Split only lane-blocking stages, preserving behavior, API, test identities and evidence selectors |

The 2026-09-01 graph-scope slice closes the former P0 generated-blank-node
identity gap for the evidenced single-source profile. R2RML mapping nodes are
reconstructed from `(effective target graph, generated identifier)`; default,
constant and row-derived graph maps normalize consistently; direct and
reference objects, class atoms, fixed-graph paths, dump/query execution and
DISTINCT subplan remapping share the same IQ identity; and CONSTRUCT template
nodes retain a disjoint fresh-per-solution label domain. Same-graph equality
between differently shaped identifier recipes remains a sound `501`, as do
dynamic-graph paths and row-dependent rendered-width pooling. PostgreSQL/MySQL
still need direct named-graph execution matrices, and globally bounded graph
dedup remains open.

`cargo audit` passes with six configured advisory exceptions and three
unmaintained crates: `paste 1.0.15` (`RUSTSEC-2024-0436`),
`proc-macro-error2 2.0.1` (`RUSTSEC-2026-0173`), and `rustls-pemfile 1.0.4`
(`RUSTSEC-2025-0134`). This is not clean supply-chain closure: every exception
still needs reachable-feature, owner, expiry, and compensating-control evidence.

## Domain model and target architecture

| Bounded context | Aggregate or port | Current home | Target responsibility |
|---|---|---|---|
| Semantic contract | `RuntimeSnapshot`, T-box, mapping IR, capability profile | `sf-core`, `sf-mapping` | Versioned T/M/schema/source/constraint-policy identity and fail-closed validation |
| Query compiler | IQ, optimizer, dialect-neutral physical operators | `sf-sparql` | Exact supported-profile rewrite to single/federated physical plan |
| Source runtime | `SourceRegistry`, `SqlBackend`, backend capability contract | `sf-sql`, `sf-serve` | Source lifecycle, binding, streaming, cancellation, health and admission |
| Federation | `FederatedPlan`, fragment, reducer, global operator | sealed `sf-sparql`/`sf-serve` two-source `UnionAll`; wider nodes planned | Per-source fragments, bounded data movement, merge/spill and failure semantics |
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
   ├── RuntimeSnapshot { T, M, schemas, constraint authorities, epochs, digests }
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

The shared-contract gate now includes opaque `SourceId`/`SourceMapping`, neutral
schema DTOs, atomic request accounting with fail-fast aggregate admission, an
immutable source registry/snapshot with request-lifetime generation leases, and
the sealed two-source `UnionAll` vertical. Next add validated candidate
construction, drift/reload and each remaining bounded federated node;
decomposition targets only modules blocking those lanes.

## Programme dependency graph

```text
M0A typed seams + release-profile truth
  ├─► M1 bounded physical execution
  ├─► M2 total-governance contract
  ├─► M3 secure observable runtime
  ├─► M4 v1 standards/live matrix
  └─► M5 snapshot lifecycle, source identity, policy + lineage
M1 operator seam + M5 SourceId ─► M6 contracts/fixtures; M2 governance + M4 QE ─► M6 promotion
M1 + M2 + M3 + M4 + M5 + M6 ───► M7 minimal v1 release
M0E + advanced M4 + SOTA proof + harness evolution ───► post-1.0
```

M1–M5 product work uses one integration writer with independent read-only support
once M0A supplies typed seams. M6 follows operator/source-identity dependencies. M7 waits for
all ADR-0055 product and minimum-evidence gates, not the post-1.0 branch.

## Milestones and QA gates

These sections explain G1–G6; they are not a second backlog. Only accepted
ADR-0055 profile guarantees are prerequisites. Proposed ADRs describe options.

### M0 — Architectural truth and deterministic foundation

Outcomes:

- publish a generated, dated capability/backend/standards matrix;
- distinguish `accepted` decision status from implementation status in every
  touched ADR;
- track `Cargo.lock`; use `--locked`; pin CI actions, installed tools, W3C suite
  inventory, fixtures, expected outcomes, skips, deviations, and spec snapshots;
- split RDB2RDF mapping conformance from SPARQL query/protocol evidence;
- freeze backend-aware SQLite and required-live PostgreSQL receipts over every
  ordered mapping outcome without promoting either backend;
- **SPARQL baselines:** freeze per-test expected SQLite query and Protocol
  outcomes as regression receipts, without treating them as W3C conformance,
  runtime provenance, or backend admission;
- retain focused boundedness evidence while keeping diagnostic records distinct
  from artifact and release authority; and
- retain the existing production-artifact decision; broader global-operator/spill
  design remains post-1.0, not a prerequisite to the admitted federation shapes.

QA gate:

- one clean release checkout resolves the locked dependency graph and exact
  candidate; independent byte-identical builder proof remains post-1.0;
- every public claim maps to a test/profile entry or is labelled planned;
- missing/malformed standards inputs fail; no new skip can hide behind a count;
- the programme backlog remains derived solely from the charter, source,
  decisions, standards and executable evidence.

### M1 — Exactness and bounded physical execution

Outcomes:

- replace silent 256-hop truncation with exact cycle-safe closure for each
  admitted dialect, or reject before returning a success response;
- preserve exact bounded execution of admitted ORDER, GROUP/aggregate, DISTINCT
  and graph/CONSTRUCT dedup, with no accidental in-memory fallback; broader
  global/external operator development remains post-1.0;
- close the PostgreSQL delimited-identifier deviation or retain it as an explicit
  release-profile exclusion; and
- keep changed files manageable; decompose a compiler/executor hotspot only when needed for the current required fix, not as an independent completion gate.

QA gate:

- chains and cycles at 1, 255, 256, 257 and >1,000 hops are exact or explicitly
  rejected, never silently partial;
- every advertised blocking shape respects its configured retained-state caps
  and has targeted source-growth evidence; formal comparative RSS qualification
  remains post-1.0;
- flat/tree/unoptimized/optimized/materialized-oracle results agree;
- unsupported-shape tests assert the exact pre-execution failure class.

### M2 — Admitted request governance and cancellation (ADR-0055)

Outcomes:

- preserve one linearizable identity for active application work from the absolute Tower `Service::call` deadline through admission, compile/acquire/execute, observable source work, semantic results and serializer bytes; representable expired handoffs are public `504` while internal first-cause accounting remains sticky; fixed health and query-less discovery metadata intentionally bypass query-work accounting;
- preserve strict media-specific request admission and its raw `n`/checked form `3n+16` wire/decoded `n` caps; it is a subset, not full Protocol conformance;
- preserve the shared fail-fast active-work gate: finite default 64, startup range `1..=Semaphore::MAX_PERMITS` before I/O, shedding in `call` before Router/body polling, deadline/control precedence, stable `503 service-overloaded` plus `Retry-After: 1`, and closed-gate `500`;
- retain its permit through active producer/compiler/backend clones but not completed-byte draining; keep terminal state out-of-band so a full channel finishes at its deadline and a streamed failure yields buffered prefix, one stable `result stream failed` error and fused EOF without `Content-Length`;
- retain the 2026-09-08 compiler-input floor: decoded UTF-8 bytes are precharged at every public compilation entry, including lineage, preflight and authenticated cache hits; exact-bound HTTP and cumulative two-pass tests pass. Public tree compilation also now meters normalization/lowering/nested-cascade clones and checked tree inner-join candidate products/left-branch copies plus atom-resolution candidates/logical-source copies and path mapping-search/complement work with exact term/source copies and mapping-wide graph inventory/reflexive checks, with cancellation, cache isolation and exact-result tests; cache hits avoid clone replay. Finish required admitted-profile compiler/catalog-growth and source-work controls without equating fuel to exact CPU; raw/conformance extensions are not silently v1 prerequisites under ADR-0055;
- keep lexical/direct-IRI scanners diagnostic. The reproduced server parser abort
  now requires the Rust isolated runtime under ADR-0055: public ordinary/lineage,
  preflight and federation use bounded QueryV1, request cancellation and exact reap;
  cap-one survival/exact recovery pass. Full ELF/syscall attestation stays post-1.0;
- admit post-parse algebra before bounded canonical rendering, and prove finite compiler/catalog growth and cancellation for every admitted phase using the shared request identity. Retain checked fanout, copy and retained-state admission; ordinary traversal may use a proved conservative phase envelope with bounded checkpoints. Proposed [ADR-0052](../adr/ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md) is not independent scope authority; logical work is not an exact CPU/allocator measurement;
- retain exact cache identity/partitioning and shared `Arc<Plan>` ownership; bound acquired-lock operations, capacity/eviction and owned cleanup for the admitted profile. Stable cross-process alpha-equivalence expansion remains post-1.0. Do not claim physical allocator/drop isolation from logical accounting alone;
- close cooperative cancellation/work bounds for the cap-four compiler profile; activate `GovernedV1` only if its declared gates pass. Its name is not a product requirement, and no default-admitted query may be silently removed to obtain a green gate;
- retain per-physical-connection cap-one admission and active-VM cancellation for owned SQLite; then cover raw mutex/submitted-work/busy/UDF/VFS/I/O gaps and retain the implemented native PostgreSQL/MySQL guards and the pinned mixed-source timeout/disconnect/SIGTERM matrix; finish wider backend/profile qualification without claiming fairness or bounded waiting;
- propagate disconnect/cancellation to all tasks, streams and connections; and
- retain generated RFC 9457/trace correlation, bounded payload-free logs/metrics and seeded-secret rejection; add adapter visibility only to close a demonstrated admitted operability defect, without driver/schema/credential strings.

QA gate:

- for active application work, one deadline covers Tower `Service::call` after request-target parsing but before Axum route/method dispatch, then admission, parse, compile, acquire, execute and serialize; fixed health and query-less discovery metadata remain available without entering that work path;
- timeout/disconnect releases worker and connection capacity within the declared
  bound for every advertised path; focused parser cancellation/reap is required
  defect-repair evidence, not the deferred full containment-attestation programme;
- exact `0`, `N` and `N+1` parser/algebra/build/work/cache tests plus focused
  release-profile differential and malformed-input proofs pass; long corpus
  fuzzing and cross-process alpha-equivalence expansion remain post-1.0;
- aggregate overload sheds immediately without Router/body polling or an internal queue; provider pools retain their separately configured bounds;
- exact result/byte caps and finite recursive/source work hold for every admitted format/path. A failed bounded prefix is followed by terminal failure and never labelled a complete answer; atomic/no-prefix streaming is explicitly post-1.0 under ADR-0055.

### M3 — Secure, observable, operable runtime

Outcomes:

- retain the implemented bounded typed startup layers and environment-only
  secret injection; direct external secret-store transport remains separate work;
- retain implemented verified remote-source TLS and credential-free argv; retain required live PostgreSQL/MySQL TLS/UNION checks and rerun on the exact release artifact;
- retain ADR-0011's request/compiler spans, Serve-only JSON logs, governance events
  and three-family default-off Prometheus profile with bounded volume/redaction;
  full OTLP/SLO expansion is post-1.0, not an admitted operability prerequisite;
- retain implemented `/livez`, snapshot-state `/readyz`, and three-phase bounded SIGTERM/Ctrl-C shutdown (drain preserves admitted work; forced expiry cancels survivors and allows three seconds for owned cleanup, failing on exhaustion); add automatic source-health/failure policy and complete cross-backend cleanup qualification; and
- publish finite operational limits and alert thresholds; research-grade SLO
  calibration remains post-1.0.

QA gate:

- a seeded secret corpus appears nowhere in argv, logs, traces, metrics or errors;
- telemetry has bounded event/label volume and passes a release smoke; formal
  controlled overhead qualification remains post-1.0;
- no metric label contains query text, IRIs, source values or other unbounded data;
- readiness already fails for explicit invalid/not-ready runtime state; automatic unavailable-mandatory-source detection remains open;
- existing admitted work can complete during drain, newly minted budgets reject, exact forced expiry cancels survivors, and owned PostgreSQL/MySQL TLS CLI tests observe stopped native work, clean exit and closed ingress after forced ASK/SELECT/CONSTRUCT and mixed UNION/join SIGTERM; server-side lock/session witnesses, separate same-credential CLI sibling safety and full cap-one recovery pass; wider backend/profile and release admission remain open.

### M4 — V1 standards and live-release profile

V1 outcomes:

- retain the exact fail-closed RDB2RDF inventory and run the required live
  matrix for each admitted SQLite, PostgreSQL, and MySQL profile;
- publish one dated supported-surface SPARQL/Protocol/result-format manifest and
  prove every advertised cell or its exact pre-I/O rejection;
- retain the static Product Mock `M ⋈ T`, DESCRIBE, mapping receipts, and exact
  5,000-case generated SQLite train as directly applicable regression evidence;
- run focused generators, malformed-input cases, mutants, and fault injection
  for the critical boundaries changed in the v1 lane; and
- make every required release service fail closed when unavailable.

V1 QA requires the exact inventory, no unexpected failure/skip/deviation, and
the advertised live/capability matrix on the release candidate. The nightly
100,000 train, broader NoREC/MR1 and cross-backend generation, long fuzz and
shrinking campaigns, global coverage/mutation ratchets, Agentic-QE expansion,
and one-hour controlled soak are labelled post-1.0 work.

### M5 — Snapshot lifecycle, identity, policy and lineage ([ADR-0050](../adr/ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md))

Current bounded slice (2026-09-08): `PgDirectLifecycleV1` now has public Rust/CLI startup and exact 16.9/16.15 owned-TLS lifecycle proof; its types stay sealed and production admission remains separate. The public Rust/CLI bearer service-principal profile now defaults closed, validates an environment-referenced credential before query body/source work, retains the provider-neutral `SecurityContext` through execution, partitions the single-source cache and compiles protected UNION uncached. Real allow/deny traces emit. The default bearer profile grants read access to all mapped data. Its optional PostgreSQL source-RLS profile binds explicitly configured claims in read-only transactions, guards every mapped public table and role, and rolls back or discards connections. Public SELECT/ASK/CONSTRUCT and both UNION fragments pass disposable PostgreSQL 16.15 row-isolation, concurrency, failure, deadline and pool-cleanup tests. The schema-version-2 provisioned registry additionally supplies a narrow portable equality-row profile for exact source/table/column rules; values remain bound parameters, uncovered or complex plans reject before source I/O, and public SQLite queries plus the two-source UNION isolate callers. General end-user identity, general ABAC/sensitivity, live cross-backend portable-policy qualification, policy-aware hot reload and access-decision metrics remain open. Rotation requires a new server. ADR-0018 remains incomplete and no backend gains production admission. The opt-in ADR-0017 constant-mapping/source SELECT and CONSTRUCT profiles now emit final-solution PROV-O and native RDF 1.2 graph reification with snapshot/logical-plan/policy IDs through the normal authenticated request/stream path. CONSTRUCT frames one response-wide N-Quads dataset with product/default graph separated from named provenance bundles, shared mapped nodes and fresh template nodes, and occurrence counts rather than unique-graph claims. Required owned pinned PostgreSQL 16.15/MySQL 8.4.11 serving-only CLI tests now parse actual SELECT PROV-O and graph reification, preserve empty/allowed portable-row isolation and reject unsupported lineage while a native source-table lock remains held. These cases do not establish all native lineage lifecycle/operator combinations or release admission. Required SQLite tests cover native reification, empty/invalid templates, nested/directional terms, escaping, bags, dedup/slice, UNION, saturation, portable policy, pinned reload, exact bytes and failure; wider multi-origin operators, authorized row keys, wider federation and wider native-profile/exact-release qualification remain open. The additional bounded multi-mapping profile carries only actual origins through RDF-matched positive BGP/JOIN/UNION and root projection/DISTINCT/slice; overlapping witnesses merge origins before LIMIT while explicit UNION preserves bags. It caps each relation at 1,024 witnesses and fails on overflow. Constant-predicate/graph-map and other precise restrictions are in ADR-0017; its mappingCatalog is not provenance. Ordinary query optimization remains separate from the lineage recipe. Twelve required native multi-map SELECT/CONSTRUCT cases now observe the exact encrypted target stop under deadline/disconnect/forced SIGTERM, preserve a separately locked sibling, and verify cap-one lineage recovery or bounded clean process exit. This does not qualify lineage UNION/JOIN, portable/source-RLS cancellation or reload. The separate bounded two-source UNION lineage profile now carries actual source-keyed mapping origins through the public path, preserving bags and blank-node scope with one snapshot/security/budget. Required HTTP checks cover reversed/unbound arms, entailed affinity, portable callers, pinned activation, witness and exact byte/result limits, and cap-one failure recovery. Six additional pinned native TLS UNION cases cover exact-target deadline/disconnect/forced-shutdown stop, unaffected siblings and recovery. Federated lineage CONSTRUCT and wider qualification remain open. The separate bounded federated join lineage profile now seals the actual map/source pair per mandatory arm, follows cost-side swapping, and emits both contributors only for each final matched bag occurrence. It reuses the 128-build/4096-probe executor and capped pre-200 serializer, with explicit map-to-source links and no hidden keys. Required public checks cover exact/projected bags, policy, activation and limits; the pinned TLS CLI aggregate adds twelve both-order join-lineage deadline/disconnect/forced-shutdown cases with exact encrypted target, held-lock, sibling and cap-one recovery witnesses. The recorded source-RLS qualification follows below; exact-release qualification remains open. Required owned TLS lineage reload now verifies changed/restored actual mapping documents and exact results for constant/multi-map SELECT/CONSTRUCT, mixed UNION and nonempty both-order joins. Native-held multi-map SELECT on both providers plus mixed UNION/forward join on PostgreSQL completes with pre-invalid results while readiness and new queries fail closed; repair restores readiness. These are authored-generation checks, not policy/configuration reload or every held graph/operator/order. Required owned PostgreSQL16.15 public-router source-RLS evidence now parses actual constant/overlapping-map SELECT/CONSTRUCT and two-source UNION/join lineage for A/B/A callers: exact authorized products/bags, actual source/map origins, no denied values or raw identities, empty results, concurrent constant SELECT and clean cap-one PID reuse after each complete response. Constant-lineage SELECT/CONSTRUCT body-drop, policy-error and deadline cases fail terminally and recover with isolated caller state. This is the recorded RLS profile, not every failure permutation, remote TLS, policy installation/configuration reload or exact-artifact admission. Under ADR-0055 the declared lineage-profile gate is now qualified and l-lineage is non-blocking. Full historical ADR-0017 remains incomplete; separate runtime-budget, backend-admission and exact-release gates stay blocking.

Outcomes:

- build immutable `RuntimeSnapshot {T, M, schemas, sources, epochs, digests}`;
- validate `M ⋈ T` and source capabilities off-path, then atomically swap; existing
  queries retain their original snapshot;
- fingerprint source schemas, detect drift, invalidate affected plans, roll back
  invalid snapshots and expose readiness state;
- retain the public query-admission context, isolated cache, PostgreSQL transactional source-RLS and portable equality-row profiles; qualify their required generation/concurrency/cleanup behavior. General ABAC/sensitivity, external end-user issuers and policy-hot-reload expansion remain post-1.0 under ADR-0055;
- retain bounded allow/deny traces and protect their redaction; extra decision telemetry is not an independent v1 gate; and
- retain the qualified opt-in lineage profiles tying actual mapping/source, snapshot,
  plan and policy identifiers to results; wider row-key/operator lineage remains outside the admitted profile. Never persist instance data or source values in telemetry.

QA gate:

- invalid reloads never activate; valid reloads are zero-downtime and invalidate
  stale plans exactly once;
- drift is detected within the declared interval and blocks affected readiness;
- tenant noninterference and pooled-context cleanup pass across concurrency,
  cancellation and failure;
- admitted provenance identifies the expected actual origins and pinned identifiers,
  bounded by its declared retained-state/result/request controls; no wider row-key claim.

### M6 — Charter-complete cross-source federation

Outcomes:

- retain the implemented `SourceId`, mapping/source affinity and immutable source registry, then complete capability and backend-generation contracts;
- retain the implemented bounded two-pattern join and one-triple-per-arm `UnionAll`;
  finish native cancellation and exact-release qualification with no silent source omission;
- preserve exact bag/NULL semantics, source consistency, partial-failure handling,
  shared cancellation and provenance for every advertised join/operator;
- use the simplest proven bounded strategy; broader operators, temp-table/Bloom
  reducers and research spill optimizations stay post-1.0 unless required by an
  advertised feature; admit source/backend/shape cells through the public profile.

QA gate:

- every released source-count/shape differential equals a trusted materialized
  reference under its admitted failure schedules;
- coordinator retained state obeys the M1 caps independently of source size;
- any implemented reducers prove exactness and safe bypass; comparative
  transfer-efficiency qualification remains post-1.0;
- cancellation reaches every source and spill artifact; no partial result is
  labeled successful.

### M7 — Minimal v1 release

Outcomes:

- split the production server from conformance, benchmark and experimental
  connector closures; give the application a non-zero semantic version;
- define supported platforms/backends, upgrade/rollback/runbooks and a non-root,
  read-only OCI reference deployment;
- gate licences, semver/API compatibility, the Rust product audit, time-bounded
  advisory waivers, SBOM, checksums, signature and exact-artifact provenance;
- build once in a clean controlled environment and smoke the exact packed digest
  against every admitted backend and federated profile; and
- retain release commands and outputs as the minimum replayable evidence bundle.

QA gate:

- production dependency tree contains only admitted functionality;
- zero unwaived reachable critical/high vulnerability and every waiver has owner,
  dependency path, feature/target reachability, controls and expiry;
- signatures, SBOM and provenance verify independently; clean-machine smoke uses
  the exact packed digest;
- all ADR-0055 hard gates pass and independent native Codex and Claude release-
  delta reviews agree. A harness diagnostic or model score contributes no points.

Two-builder byte identity, transparency/witness publication, exhaustive runtime-
closure proof, comparative Ontop publication and harness evolution are not
renamed complete; they remain post-1.0.

## Ruflo and MetaHarness execution model

ADR-0037 remains available as the engineering control plane, but ADR-0055
governs its v1 use:

1. One integration owner writes and commits directly on `main`; no new branches
   or worktrees. Preserve unrelated changes and historical recovery refs.
2. Every building task uses the [main-only delivery harness](../../coding-harness/README.md#mandatory-delivery-path): task → native model/effort handoff → checks → verification → scoped commit → exact-commit result. Native agents edit; parallelize read-only investigation/review, not shared writes. Closed experiments remain optional. Honor explicit user review holds before any continuation.
3. Each commit runs affected tests/builds; shared contracts, dependencies,
   security, unknown impact or failed selection escalate to integrated gates.
4. Meaningful public-feature integration boundaries run the full locked workspace
   plus relevant live/security/cache/federation checks, not every micro-commit.
5. The immutable release candidate runs all ADR-0055 gates and receives
   independent native Codex and Claude exact-delta review.

Ruflo records coordination and evidence identity; deterministic Rust tests and
release checks remain product authority. Native subscription transport only is
permitted; OpenRouter and provider API-key fallback remain prohibited. No subscription spend/token/request/invocation/quota ceiling is permitted.
Native subscription/model unavailability pauses execution with its exact error.
**Model correction (2026-09-10):** repeated Ultra follow-up is no longer the default. Ordinary tools handle Git/build/test/polling; Luna low/Haiku handle bounded mechanical tasks, Terra medium established patterns, and Sol medium/Sonnet normal implementation and review. Sol high is appropriate for the current bounded identity proof. Escalate to Astra high/Opus only for a named unresolved cross-component problem; max/Fable needs a bounded exceptional task, and Ultra a specific user request. Retain the selected main model and explicit requested effort. Once a hard question is answered, route subsequent tasks afresh rather than keeping its strongest reviewer. The native Ultra reviewer was stopped and Sol assigned; current evidence does not support a numerical speedup or cost claim. ADR-0055 records the policy and qualification boundary.

Historical harness receipts remain valid for their exact claims. MetaHarness V7,
Darwin/GEPA, AVO, generic score improvement and retrieval-policy evolution are
post-1.0. The flywheel remains off, and no bulk memory import may bypass owning
stores or synchronization.

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
| External scanner or raw cross-parse equality diverges from the pinned parser | False admission, rejection or cache miss classification before source I/O | No scanner authority; ADR-0055 parser-lifetime repair uses isolated parsing, bounded QueryV1 and focused release-profile differentials; full ELF/syscall attestation stays post-1.0 |
| Federation becomes a rewrite | Schedule and semantic drift | Preserve compiler; introduce SourceId/registry/physical plan behind ports |
| Spill substrate conflicts with ADR-0006 | Hidden architecture reversal | Separate design-lock ADR and benchmark both implementation choices |
| Backend behavior diverges | One green dialect masks another | Shared backend contract plus fail-closed live matrix |
| Policy taxonomy is unavailable | Platform coupling or stalled delivery | Provider-neutral SecurityContext/Policy port and reference fixtures |
| High gates become flaky | Teams bypass evidence | Controlled runners, variance classification, quarantine with owner/expiry |
| Supply-chain exceptions become permanent | Known reachable exposure | Reachability proof, owner, controls, expiry and release review |
| Harness optimizes its evaluator | False programme progress | Protected inputs, independent product gates, sealed holdouts, reward-hack scan |
| Documentation drifts again | Misleading claims and planning | Maintain G1–G6 from source-bound public acceptance and exact missing evidence; update only affected ADR decisions/status and required catalogue hashes |

## Definition of done

The v1 programme closes only when one immutable candidate satisfies ADR-0055:

- every applicable accepted product/runtime ADR is implemented with current
  executable evidence or explicitly superseded;
- every advertised query/profile/backend cell is exact, and every unsupported cell fails before a valid-looking response;
- bounded state, total budgets, cancellation and overload gates pass for every
  admitted operator and backend;
- identity/policy/provenance, snapshot lifecycle and operability gates pass;
- cross-source differential, boundedness, reduction, cancellation and failure semantics pass;
- the minimal immutable artifact, live smoke, SBOM, licence/advisory disposition,
  checksums, signature and provenance verify from one clean controlled build; and
- independent native Codex and Claude review the exact release delta without
  replacing any deterministic gate.

Post-1.0 research and advanced assurance remain explicitly incomplete backlog;
they do not change the v1 verdict. Anything short of the bullets above is not a
completed v1 application and must be reported with its exact failed gate.
