---
status: final-transaction-pending
date: 2026-08-25
updated: 2026-08-25
owners: [integration-owner, query-semantics, dependency-governance, harness-control-plane]
decisions: [ADR-0010, ADR-0024, ADR-0035, ADR-0036, ADR-0037]
---

# Open-issue remediation with a dual-host Ruflo MetaHarness

## Outcome

Deliver the wrong-result fixes in issues #8 and #9, the dependency unblock in
#10, and evidence-backed dispositions for #7 and #6. Build the versioned
`coding-harness/` into the full dual-host engineering control plane specified by
ADR-0037, prove it on #8, and accept it only when every hard gate passes and its
project-owned seven-dimension evidence score is at least 98.

The plan itself grants no issue-state, push, merge, release, or publication
authority. The user separately authorized its local execution.

## Execution outcome (2026-08-25)

The product and framework slices were executed and committed locally. Direct
product verification passes, the trusted native runtime is implemented, and the
programme is awaiting its final dual-host issue-#8 transaction. Run 16 passed
the frozen dependency-closure stage, then failed closed when its first Codex
architecture call exhausted a terminal 20-minute deadline. Native subscription
health remained available. Deadline exhaustion is now a classified transient
process failure with exactly one fresh same-host retry; two 10-minute attempts
preserve the previous 20-minute worst-case lane budget. Run 17 proved that retry
path but both full-source Codex architecture attempts exhausted their bounds.
Architecture context is therefore reduced to the task contract plus a digest-
bound file manifest; full declared source remains available to implementation,
and admitted source plus the exact staged diff remain available to repair and
review.

| Scope | Outcome | Evidence |
|---|---|---|
| #8 checked binds | implemented | `10dedd4`; exact red/green target plus flat/tree/oracle and workspace gates |
| #9 graph union | implemented | `5218874`; graph, path, RDF-star, materialization, and conformance gates |
| #10 SQLite link | implemented with advisory baseline | `5b8415c`; one `rusqlite 0.40.2` workspace version; `RUSTSEC-2026-0235` remains unignored |
| #7 serving admission | locked, providers deferred | `9d709dd`; only SQLite, PostgreSQL, and MySQL are admitted |
| #6 materialization API | deferred | no consumer-red Nova reproducer, so no speculative API was added |
| Harness framework | implemented; final transaction pending | incremental harness commits through `936f738`; bounded timeout-retry reconciliation in progress |

Final direct gates passed: locked/offline workspace format, clippy with warnings
denied, all-target build, workspace tests, and W3C conformance (82 adjudicated,
zero unexpected failures, one documented deviation). The harness builds and
passes 298 tests across 45 files before the timeout-retry reconciliation.

The final transaction starts from a packed private controller commit and uses
native ChatGPT and Claude subscriptions, a loopback CONNECT endpoint backed by
the controller's exact-origin Unix-socket broker, sealed mount namespaces,
copied credential capabilities, systemd cgroup-v2 quotas,
frozen offline Rust inputs, task-bound LCOV/SAST, independent review, and a
digest-chained receipt plus a receipt-bound programme envelope. Dirty user-owned
`.mcp.json` and untracked `coding-harness/.claude/` state remain unstaged and
masked from model sessions.
OIA/MCP visibility remains honestly `INCONCLUSIVE`.

The envelope derives diagnostic status from the parsed native Ruflo score
snapshot in `coding-harness/config/metaharness-diagnostics.json`; the snapshot
records the owning implementation hashes and its exact blob digest must equal
the protected-input digest in the candidate receipt.

The upstream repository/harness `harnessFit` values remain 71/67. Exact source
audit of the owning Ruflo wrapper and its active `metaharness@0.3.2` cache proved
these are shallow archetype-classification diagnostics that do not inspect this
harness or run tests. The scorer computes `round(plan.confidence * 100)`; the
selected Rust archetype tops out near `0.8003 → 0.80 → 80`, and the global
archetype ceiling is `0.967 → 0.97 → 97`. ADR-0037 therefore explicitly replaces
the former upstream 98 criterion with the project-owned rubric; failed upstream
hard constraints or degraded execution still fail closed.

Darwin/GEPA and AVO remain ineligible. No issue was closed, no PR was merged,
and nothing was pushed, published, deployed, or promoted.

## Frozen planning baseline

- Repository: `main` at `6f0841afa63864784103b256cc4728a762e96f17`.
- Open issues checked on 2026-08-25: #6–#10.
- Open PR #12: base `6f0841a`, head
  `ea16ece21f1d313614c39ca7689315d8f73db961`; it touches five manifests that
  overlap dependency work.
- Existing user/concurrent changes in `.claude*`, `.mcp.json`, `.claude-flow/`,
  `agentdb.rvf.idmap.json`, and `coding-harness/.claude/` are outside scope and
  must remain unstaged.
- `coding-harness/` is the only versioned harness candidate. The ignored
  `semantic-fabric-harness/` and untracked host files are non-authoritative and
  are never copied or merged implicitly.
- Current diagnostics are baselines only: repository score 71, tracked harness
  score 67 and genome `needs-work`; OIA/MCP scans are inconclusive because they
  do not see the configured MCP surface.
- Public-registry versions verified on 2026-08-25 are
  `@metaharness/harness@0.2.0`, `@metaharness/router@0.4.0`, and both native host
  adapters at `0.1.2`; implementation re-verifies exports before pinning them.

Re-freeze issue bodies, `main`, PR #12, tool versions, and the dirty-tree digest
at execution kickoff. If any changed, route an ADR-drift review before writing.

## Issue decisions

| Issue | Summary | Action |
|---|---|---|
| #8 | Unchecked incompatible S/P/O/class/GRAPH bindings can retain impossible branches. | Implement first as a complete checked-bind invariant. |
| #9 | Query, path, and RDF-star graph semantics disagree with R2RML subject/POM union and default-graph handling. | Implement second in the same writer lane. |
| #10 | Five crates pin `rusqlite 0.32`, blocking downstream `0.40.2`. | Implement in parallel after PR #12 and fresh-resolution baselines. |
| #7 | REST prototypes are buffered, text-bind parameters, omit protocol state/cancellation, and are not served; “Athena” is Presto. | Re-scope by provider; expose none yet. |
| #6 | Cross-project umbrella mixes architecture and optional seams. | Extract only a consumer-red fallible/early-exit sink task; do not implement the umbrella. |

## Operating model

### Ruflo ledger

Use native Ruflo MCP tools for coordination; Codex and Claude execute work:

1. Search project `patterns` and user `user-patterns` memory, including the
   active MetaHarness phase-gating pattern.
2. Call `hooks_route` and `hooks_model_route` for each task before assignment.
3. Initialize a hierarchical, specialized swarm, initially four active workers
   and no more than eight, with `consensusMechanism: raft`; initialize the
   hierarchical hive with Raft for decision consensus.
4. Register named agents and dependency-aware tasks with `agent_spawn`,
   `task_create`, and `task_update`. Do not call a metered provider executor.
5. Monitor `swarm_health` and `observe_trace`; bind swarm, task, route, and trace
   IDs into the harness receipt.
6. After direct verification, record `hooks_model_outcome`, complete the task,
   and store only validated reusable patterns. Repository facts stay in local
   memory; no secrets or project content enter user memory.

The integration owner is the sole Raft leader for writes. Read-only research,
evaluator design, security review, and cross-vendor critique may run in
parallel.

### Native model roles

Every hard task uses native first-party subscriptions only:

| Stage | Codex/ChatGPT | Claude | Rule |
|---|---|---|---|
| Architecture | Independent proposal | Independent proposal | Bounded critique; integration owner records the decision. |
| Implementation | Routed writer or shadow patch | Routed writer or shadow patch | One admitted writer patch; never two writers in one worktree. |
| Verification repair | Eligible for verifier-directed repair | Eligible for verifier-directed repair | Route by verified quality/reliability, with one transient retry. |
| Review | Independent review when Claude wrote | Independent review when Codex wrote | Both vendors review hard-tail/cross-cutting changes. |
| Reflection | GEPA participant only after eligibility | GEPA participant only after eligibility | Frozen routes/models and sealed holdouts. |

Preflight must prove ChatGPT subscription auth for Codex and first-party
`claude.ai` auth for Claude. Strip provider keys, base URLs, and ambient proxy
variables; the controller may inject only a loopback CONNECT endpoint backed by
its exact-origin Unix-socket broker. If either native host is unavailable,
cross-vendor gates fail closed; there is no OpenRouter, gateway, provider-API,
or same-vendor substitute.

### Worktree and writer map

| Lane | Branch/worktree | Mutable scope | Serialization |
|---|---|---|---|
| H-A contracts | `harness/contracts` | harness schemas, policy, process, receipts | Freeze shared contracts before H-B/H-C integration. |
| H-B models | `harness/native-models` | Codex/Claude adapters, router, pool | Parallel with H-C after contracts. |
| H-C verifier | `harness/verifier` | workspaces, build/verifier, task config, tests | Parallel with H-B after contracts. |
| Q semantics | `fix/query-correctness` | #8 then #9 source/tests | One writer; #8 commit before #9 work. |
| D dependencies | `chore/rusqlite-040` | #10 manifests/tests only | Parallel with Q after PR #12 freeze. |
| C connectors | `design/cloud-providers` | read-only evaluator/design first | Product writes wait for Q/D gates and a provider ADR. |
| M materialization | `design/nova-sink` | consumer reproducer/spec first | No API write until a focused red test exists. |

Use detached candidate and verifier worktrees pinned to exact commits. Evaluator
inputs are protected and overlaid only into the verifier transaction. A single
integration owner stages scoped paths and preserves every unrelated change.

## SPARC delivery phases

Each implementation task follows Specification → Pseudocode → Architecture →
Refinement → Completion. The phase artifact is stored in the Ruflo task/receipt,
not added as ad hoc root documentation.

### Phase 0 — authority, factory, and evaluator baselines

**Specification**

- Freeze issue/PR revisions, mutable/protected paths, direct commands, expected
  failures, time/output ceilings, auth class, and publication authority.
- Define the Query Semantics, Dependency Governance, Connector Runtime,
  Materialization API, and Engineering Control Plane context boundaries from
  ADR-0036/0037.

**Architecture and evidence**

- Capture the tracked harness's HTTP/private-registry lock entries as a failing
  supply-chain test; do not run `npm ci` from that lock.
- Query every required package and verify exports against the approved HTTPS
  registry in a disposable directory. Generate a candidate lock there and fail
  on HTTP/private-address origins or missing integrity hashes. If a required
  package is unavailable, stop for a separate runtime/vendoring decision.
- Only after that preflight, run the current Ruflo MetaHarness factory into
  separate disposable directories for `codex` and `claude-code`; run
  doctor/genome/score, inspect manifests, and verify every generated command
  exists. Copy nothing into the repository.
- Create a red evaluator for #8 in a protected verifier branch. Prove it fails
  on the frozen baseline and that materialized RDF gives the expected answer.

**Exit gate:** baseline receipt contains clean candidate/verifier identities,
protected-input digests, red-evaluator proof, both native auth classes, factory
diagnostics, and explicit `development-only-no-promotion` authority.

### Phase 1 — minimum operational engineering harness

Build only the control-plane slice required to execute #8 before expanding
features. Product progress remains the programme objective.

**H-A: contracts, policy, receipts**

- Set `coding-harness` to `"private": true`; remove `publishConfig`, the
  publish-oriented `files` list, and every evolution script/dependency/suite
  while the 5+5 gate is unmet; add a failing `prepublishOnly` guard.
- Regenerate the lock from an approved HTTPS registry, keep integrity hashes,
  require `npm ci`, and fail tests on HTTP/private registry origins.
- Add strict task/config/receipt schemas, structured process execution, explicit
  environment allow-lists, all five policy gates, exact path allowlists,
  protected inputs, cancellation, and chained digest receipts.
- Compute protected-input digests from an explicit tracked-path list rather than
  the stale factory manifest; any listed untracked path fails the task.

**H-B: native routing and workers**

- Add native Codex and Claude adapters, auth/env preflight, real
  `@metaharness/router`, and a persistent run-scoped `AgentPool`.
- Cold-start least-observed capable hosts; update quality only after deterministic
  verification; freeze route snapshots and record honest `$0` subscription cost.
- Add bounded critique, distinct-host review, transient retry, circuit breaker,
  cancellation, and verifier-directed repair orchestration.

**H-C: workspaces and verifier**

- Build on `@metaharness/harness` with `HarnessKernel`, `PolicyGate`,
  `VerifierRegistry`, `AlgorithmRouter`, and the persistent pool.
- Implement clean candidate/verifier worktrees and the invariant:
  **apply patch → build patched candidate → run public/independent/regression
  gates → cross-vendor review → receipt**.
- Reject stale/pre-patch artifacts; every repair repeats admission, build, and
  all verifiers.
- Accept externally supplied Ruflo/QE evidence only after schema validation;
  do not create a replacement MCP server.

All harness files stay under 500 lines. CI tests fake native executables and has
no provider or arbitrary-network access.

**Exit gate:** `npm ci`, build, unit/integration tests, tamper/cancel/breaker/
repair tests, HTTPS-lock test, route snapshot test, two-host fake review, and
receipt-chain verification pass. No real product patch is admitted yet.

### Phase 2 — issue #8 as the first end-to-end proof

**Specification/pseudocode**

- Enumerate every `bind_position`/`bind` call in ordinary atoms and `rr:class`
  atoms. Pseudocode has one rule: a false/incompatible bind immediately prunes
  that candidate branch, regardless of S/P/O/GRAPH position or inverse swap.

**Refinement**

- A tester owns a new integration test file under 500 lines. Cover incompatible
  constant subjects, `?x ?x ?o`, `?x ?p ?p`, `rr:class`, and
  inverse-predicate swapping through flat and tree planners. Audit the GRAPH bind
  call sites now, but defer graph-set-dependent expected rows to #9.
- The routed model writer changes the smallest invariant surface. The other
  vendor reviews independently; mutation-lite removes/negates every prune check.
- Run task-bound Agentic-QE gap analysis and SAST only as advisory evidence with
  provider variables stripped.

**Completion gates**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --all-targets
cargo test -p sf-conformance --test differential_oracle
cargo test -p sf-conformance --test differential_tree
cargo test --workspace
cargo run -p sf-cli -- conformance
```

Add the exact new targeted test command and mutation commands to the task before
execution. Flat/tree/materialized-oracle rows must match and unsupported-query
counts must not rise.

**Exit gate:** harness receipt proves red baseline, admitted patch, post-patch
build, all direct gates, killed mutations, and independent Codex plus Claude
reviews. Integrate one coherent verified `fix(sf-sparql)` commit.

### Phase 3 — #9 and #10 in parallel

#### Q lane: issue #9 graph semantics

- Start only from the integrated #8 commit because both touch `unfold.rs`.
- One semantic owner implements the normalized distinct
  subject-map/POM graph union across BGP unfolding, path filtering/enumeration,
  and RDF-star description maps. Reuse or extract one canonical helper rather
  than repeating precedence logic.
- Exclude `rr:defaultGraph` from `GRAPH ?g`; preserve default BGP membership;
  deduplicate identical destinations. A pinned constant against a dynamic graph
  map gets a runtime equality constraint or honest `Unsupported`, never false
  empty.
- Cover a graph variable reused as S/P/O, including
  `GRAPH ?s { ?s :p ?o }`, only after the normalized graph set is active.
- Add new under-500-line graph/path/star integration files for every ADR-0035
  cell, including RDF-star object position and override/default mutations.

Run targeted `differential_graphs`, `differential_paths`, `differential_star`,
flat/tree tests, workspace gates, and `cargo run -p sf-cli -- conformance`.

#### D lane: issue #10 dependency compatibility

- Re-fetch PR #12 and freeze its base/head/touched paths. Rebase or cherry-pick
  only after the integration owner chooses a non-overlapping base.
- In a disposable isolated resolution, generate a fresh lock and record:

```bash
cargo tree -i rusqlite
cargo tree -i mysql_async
cargo tree -i lru
cargo audit
```

First record a pre-change downstream reproducer containing the exact
`rusqlite 0.32`/`0.40.2` `links = "sqlite3"` failure.

- Centralize `rusqlite = 0.40.2` in workspace dependencies only if fresh
  resolution and PR #12 reconciliation support it. Preserve `bundled` and
  `column_decltype` at crate use sites. Inspect `mysql_async`/`lru` because they
  affect advisory evidence, but change neither without a separate scoped issue.
- Run default/no-default and selective optional-backend builds, SQLite
  execution/introspection, conformance, benchmark tests, the workspace CI
  commands, and live MySQL tests. Do not add a new advisory ignore.
- Raise the ignored-versus-tracked `Cargo.lock` policy as a separate decision;
  do not smuggle it into the version bump.

**Parallel exit gate:** Q and D each have independent patched-build receipts and
coherent commits. Rebase D on the integrated Q/main tip, rerun impacted workspace
gates, then integrate. Neither lane absorbs PR #12 wholesale.

### Phase 4 — full harness acceptance and combined product gate

- Complete outcome memory, Ruflo swarm/task/hive/consensus binding, the issue-#8
  LCOV/SAST profiles, dual-vendor repair/review, cancellation, circuit-breaker,
  and receipt evidence. Keep hook/trace arrays empty when Ruflo emits no
  authoritative identifier; never fabricate one.
- Fix diagnostic visibility so OIA/MCP scans inspect the canonical manifest and
  actual `.mcp.json`; otherwise retain `INCONCLUSIVE`.
- Run doctor, genome, score, OIA, threat model, MCP scan, and drift with provider
  variables removed. Treat them only as control-plane diagnostics.
- Keep score/genome/similarity, fitness weights, tool/command policy, and oracle
  law outside evolution fitness. Require a reward-hack scan in each evolution
  receipt.
- Re-run #8+#9+#10 combined Cargo, standards, differential, live-source,
  advisory, mutation, and receipt-chain gates from a clean integration worktree.

**Exit gate:** every ADR-0037 hard gate passes and the project-owned
seven-dimension evidence score is ≥98/100. Any failed product, security,
standards, authentication, or supply-chain gate fails the programme regardless
of aggregate score. Upstream `harnessFit` is diagnostic context only.

### Phase 5 — re-scope #7 and extract #6

**Connector Runtime (#7)**

- Lock `sf-serve` admission to SQLite, PostgreSQL, and MySQL until any other
  adapter passes its own gates; this includes DuckDB, HANA, MonetDB, ODBC,
  Oracle, Redshift, SQL Server, and the REST family.
- Create a small umbrella and provider-specific specifications for Trino/Presto,
  AWS Athena, Databricks, Snowflake, and BigQuery. Split `rest.rs` by provider
  before adding behavior.
- Split the current 1,183-line `rest.rs` into one module per provider plus shared
  Presto protocol code, each under 500 lines, before expansion.
- For each provider, freeze official protocol/auth references and build negative
  evaluators for async lifecycle, 200-with-error, paging/chunks, cancellation,
  page/row/byte/time ceilings, typed server-side parameters, natural datatypes,
  origin-pinned pagination/redirects, HTTPS, and credential redaction.
- Choose the first implementation only after evaluator feasibility and explicit
  priority review. `sf-serve` exposure waits for mock/emulator coverage, an
  opt-in live canary, schema/source parsing, admission control, and every
  security/streaming gate. Never label the Presto adapter AWS Athena.

**Materialization API (#6)**

- Ask the Nova consumer lane for a minimal red test against a concrete backend.
  If reproduced, extract one focused fallible/early-exit sink issue.
- Specify complete, clean break, engine error, and sink error outcomes; immediate
  callback stop, cursor release, and bounded memory. Do not expose raw internal
  plans, force object safety, or publish an always-ready-only sync bridge.

These are design/evaluator tasks until their gates exist; do not delay completed
#8/#9/#10 delivery for them.

### Phase 6 — conditional Darwin/GEPA and AVO

- Build a corpus of at least five discriminating training tasks and five sealed
  holdouts before any Darwin/GEPA run. Use opaque IDs and keep evaluator law,
  thresholds, protected truth, and authority outside the genome.
- Freeze models and route snapshots. Candidate genes may affect planner,
  context-builder, reviewer, retry, tool, memory, and scoring policy only.
- Promote only a held-out improvement with no safety/regression loss; otherwise
  retain the seed and record an honest null.
- Do not restore evolution scripts, the Darwin dependency, or a suite until this
  eligibility gate passes. Diagnostic scores and tool/score policy are never
  mutation surfaces or reward signals.
- Consider AVO only for a named #7 hard-tail after ordinary repair cannot choose
  between plausible strategies and a reliable independent evaluator exists.
  Use copied workspaces, bounded actions/time/invocations, protected inputs, and
  stop at a verified winner or null. #8, #9, #10, and the #6 sink are ineligible.

## Commit and integration policy

Commit each verified slice immediately with conventional messages, for example:

1. `feat(harness): establish secure execution contracts`
2. `feat(harness): add native dual-host orchestration`
3. `feat(harness): enforce patched-candidate verification`
4. `fix(sf-sparql): prune incompatible atom bindings`
5. `fix(r2rml): unify graph-map query semantics`
6. `chore(deps): upgrade rusqlite to 0.40.2`

The evaluator may be a protected red branch during the transaction, but the
integrated product commit includes its reviewed regression tests and passes.
Stage only task-owned paths. Never commit user/concurrent state, credentials,
provider settings, generated sessions, or `.env` files. Commits do not authorize
a push, merge, release, deployment, issue update, or publication.

## Stop and rollback conditions

- Stop a lane if its frozen issue/PR baseline changed, protected inputs changed,
  both native hosts are not independently available, or a direct oracle is
  nondeterministic.
- Open the circuit on repeated process/transport failures; do not route through a
  forbidden provider. Preserve the candidate and issue a failure receipt.
- Reject a patch that touches undeclared paths, uses pre-patch artifacts, weakens
  a test/advisory ignore, changes publication authority, or leaves a file over
  500 lines.
- Roll back by omitting the unintegrated worktree commit or reverting the single
  coherent integrated slice. Never reset or overwrite the shared dirty tree.

## Programme definition of done

- ADR-0035's correction and ADR-0036's sequencing are reflected in verified #8
  and #9 commits; #10 is compatible with PR #12 and downstream `rusqlite 0.40.2`.
- #7 has honest provider-specific scope and no premature serving exposure; #6
  has either a consumer-red focused issue or a documented no-change disposition.
- The canonical `coding-harness` executes one real #8 transaction with both
  native vendors, full policy/repair/reliability/evidence features, direct
  product gates, a valid receipt chain, and no publication authority.
- Every hard gate passes, diagnostic blind spots are fixed or marked
  inconclusive, and the project-owned ADR-0037 score is at least 98 without
  averaging away a failed oracle; upstream `harnessFit` remains contextual.
- Ruflo task/outcome records and repository-local reusable lessons are stored;
  no secrets or project-specific facts are promoted to cross-project memory.
