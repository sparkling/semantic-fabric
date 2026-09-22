# Native parallel execution and model usage

Date: 2026-09-19. Updated: 2026-09-20. **2026-09-22 correction: Codex is
paused (user instruction; application work also remains paused).** The
coordinator and every worker/reviewer route below is Claude Code, via the
user-authorized local 9router subscription transport, until the user
explicitly re-authorizes Codex. Everything below that names Codex, `codex
exec`, or a Codex model tier is historical evidence from the dual-provider
period (2026-09-19 through 2026-09-22) and comparison data, not a currently
active dispatch instruction; do not launch a Codex process from this plan.
Status: execution authorized by the user's goal "complete the programme using
the harness"; G3 work is active, subject to the separate 2026-09-22 pause.
Scope: native Codex/ChatGPT and Claude subscription agents, using the existing
delivery harness. Based on the three-worker design discussion in Ruflo swarm
`swarm-1789853768325-0ps4xv` and repository evidence through `f2d2a469`.

## Objective and evidence

Shorten time to accepted, verified outcomes while reducing unnecessary tokens
and preserving accuracy. Agent count is not a success measure.

- Do not assume a repository-wide limit for independent native sessions.
- Four independent Codex sessions were observed successful on 0.155.1. The earlier
  audit also observed root plus three in-session workers. Neither proves infinity.
- Claude 2.1.274 has a baseline of 20 additional in-session subagents, with
  conditional enforcement. Independent Claude session capacity is unmeasured.
- Per-session child limits are distinct from independent process capacity.
- `delivery-runtime.ts` permits one writer; `delivery-workspace.ts` binds whole
  source and locks operations. `delivery-workflow.ts` releases checks, independent
  other-provider review and commit in order. Disjoint paths do not permit writers
  to bypass those invariants.
- Model defaults already exist in `coding-harness/src/delivery-contracts.ts`.
  Existing defaults are starting hypotheses, not measured optimal choices.

## Ownership and dispatch

The existing native coordinating conversation owns the dependency queue and is
the sole integration writer on canonical main. Choose ownership for continuity
and context, not a presumed provider capacity advantage. No new branches,
worktrees, competing implementation hosts or scheduler are introduced.

Each worker receives a compact task packet:

| Field | Required content |
| --- | --- |
| Identity | Task ID, native executor ID, provider/model/effort, recipient |
| Inputs | Commit/source digest, relevant files, accepted prerequisite results |
| Dependency | Explicit predecessors; unresolved prerequisites block dispatch |
| Ownership | Read-only paths for investigators; exact edit scope for sole writer |
| Outcome | Bounded deliverable, acceptance checks and stop condition |
| Resources | Shared build outputs, databases, ports or heavy fixture needs |

Keep these fields in existing coordinator/Ruflo records; do not pass unsupported
fields into the strict delivery task schema. Native tools execute; Ruflo records
track. Match every tracked worker to the real native executor and actual route;
Ruflo's advisory model labels are not proof of which model ran.

Dispatch ready tasks that shorten the critical path or prepare the immediate
next slice. Add workers as independent questions appear. On completion, validate
the result and source identity, release successors, then refill. Reject or
revalidate stale findings before applying them. Avoid a growing backlog of
speculative patch proposals.

Use native Codex `collaboration.spawn_agent`, `followup_task`, `send_message`,
`list_agents` and completion notifications; Claude uses `Agent` and `SendMessage`.
Independent native processes are available when useful work exceeds one host's
child slots. The coordinator collects their process completion/output and owns
cross-provider handoffs; there is no implicit cross-provider shared queue.

Example bounded read-only launches, after preparing task-specific prompts outside
tracked source and confirming installed client options:

```bash
codex exec --model gpt-5.6-sol -c model_reasoning_effort=medium \
  --sandbox read-only - < /absolute/task-prompt.txt
claude -p --model sonnet --effort medium --output-format json \
  --tools 'Read,Grep,Glob' --strict-mcp-config \
  --mcp-config '{"mcpServers":{}}' < /absolute/task-prompt.txt
```

Use configured native subscription authentication. Never read credentials,
introduce API-key transport, use `--bare`, add fallback providers or impose usage
budgets. If the requested native client/model fails, report its exact error and
pause affected model execution. Do not silently substitute a model.

## Parallel work and exclusive resources

Read-only investigation, test selection and review preparation may overlap a
writer. Formal current-source review starts only after checks pass; the whole
source must remain stable through review submission. An older commit review
does not satisfy a newer source-bound delivery request.

The integration owner serializes edits, generated manifests, Git index/commits,
and delivery checks. Reserve one heavy-build/packing slot initially. Respect
existing test settings (`vitest.config.ts` has two file workers), shared Cargo
targets, databases and ports. Increase build concurrency only after evidence of
safe isolation and a throughput improvement. Agent capacity is separate.

Report at each dispatch/completion: active executors and tasks, ready queue,
blocked tasks with prerequisite/resource, review queue and current exclusive
resource owner. Explain idle capacity when ready work exists. Do not launch idle
workers or duplicate work to fill capacity.

## Model selection and efficiency

| Task | Codex starting route | Claude starting route |
| --- | --- | --- |
| Deterministic work | Tools | Tools |
| Bounded mechanical leaf | Luna low | Haiku |
| Established pattern | Terra medium | Sonnet |
| Normal implementation/review | Sol medium | Sonnet |
| Specific correctness proof | Sol high | Sonnet; escalate if unresolved |
| Named difficult problem | Astra high | Opus |

Preserve the selected main model and explicitly requested routes. Claude defaults
use native default effort unless a task explicitly specifies supported effort.
Escalate for a demonstrated reasoning gap or unresolved high-impact question;
return descendant tasks to their own appropriate tier once it is answered.
Exceptional max/Fable use needs a bounded justification; ultra needs an explicit
user request. Transport/configuration failures are not reasoning failures.

Send relevant source and concise context rather than full conversation history.
Reuse accepted evidence only while its source remains valid. Duplicate analysis
only for a named uncertainty, with distinct hypotheses and a merge/stop rule.

During useful authorized work, record elapsed time to acceptance, first-pass
acceptance, repair/reopen counts, later defects, resource wait and total reported
tokens including repeated input and rework. Record missing usage as unknown;
separate input/output/cached usage where exposed. Compare comparable task classes,
not unlike tasks or headline tokens/second. Optimize token efficiency as the user
requested, without hard budgets, quota gates or invented subscription charges.
No separate benchmark programme is required. Retain routes that reduce time and
rework without losing acceptance quality; a larger model may use fewer total
tokens when it avoids failed attempts.

## Comparison with remaining application work

Sources: [current G1-G6 ledger](sota-application-completion-programme.md#remaining-release-gates-2026-09-11),
[2026-09-19 handover](session-handover-2026-09-19.md#ledger-status), and exact command
definitions in `tests/capabilities/catalog-v1.json`. This comparison authorizes no
application execution by itself; the later user goal above releases that hold.
It replaces the earlier assumption that the maintenance
failures below describe the application's next critical path.

| Gate | Current evidence / remaining outcome | Useful parallelism and dependency |
| --- | --- | --- |
| G1 | Handover reports closed; ledger retains older G1c "still open" wording before later combined acceptance | Reconcile named obligations with final receipts during G3 prerequisite scoping; do not assume every historical gap remains active or silently certify closure |
| G2 | Ledger explicitly closes all six acceptance commands after the instrumented timeout fix | Reuse source-bound evidence; no new G2 investigation without a regression or affected dependency |
| G3 | Open: coherent backend/policy generations, DDL races, common lease for validation and cache identity | Next implementation gate after scoping; independent lifecycle, semantic admission and acceptance audits can run together |
| G4 | Open: source consistency, protected generations, admitted backend combinations | Prepare matrix now when authorized; final implementation/qualification depends on G1-G3 |
| G5 | Open: required transport/live matrix on one immutable candidate | Prepare fixture/readiness and command mapping alongside G3/G4; final qualification follows G1-G4 |
| G6 | Open: exact candidate release/admission evidence | Prepare local evidence checklist alongside earlier gates; final verdict follows G1-G5 and exact-delta reviews; publication remains separately authorized |

The release dependency chain is G3 -> G4 -> G5 -> G6, subject to reconciling G1's
prerequisite record. These gates cannot all close concurrently. More agents can
shorten investigation and preparation; they do not remove the integration or
immutable-candidate dependencies. No speedup or completion date is inferred.

### First application allocation, only after execution is released

Each row is a read-only deliverable owned by one named worker. The root owns all
eventual edits and selects exact file scopes only after the findings establish
what remains. Directory scopes here identify investigation inputs, not broad
permission to edit. G3 is still unscoped; inventing implementation subtasks now
would repeat the guessing the user explicitly rejected.

| Worker / starting route | Exclusive investigation responsibility | Deliverable and acceptance |
| --- | --- | --- |
| P1 / Sol medium | G3 lifecycle: `crates/sf-serve/src/pg_generation/`, `pg_direct_lifecycle/`, `snapshot.rs`, `request_generation.rs` | Trace lease lifetime and backend/policy/DDL-race invariants; identify concrete missing obligations with file references and reproduction plans |
| P2 / Sonnet | G3 admission/cache: `crates/sf-mapping/src/projection.rs`, `crates/sf-validation/`, `crates/sf-serve/src/request_cache_identity_tests.rs` | Map validation and cache identity to P1's lease interface; distinguish existing proof from gaps; no duplicate lifecycle audit |
| P3 / Terra medium | G3 evidence: catalogue commands, `runtime_snapshot_tests.rs`, `runtime_activation_http_tests.rs`, CLI `authored_generation.rs` and `direct.rs` tests | Map all eight G3 command IDs to actual assertions/fixtures, reconcile G1 prerequisite wording and receipts, propose affected checks; no green claim from skipped live tests |
| P4 / Sonnet | G4 preparation: `crates/sf-sparql/src/federation/`, serving federation tests, CLI `federated_lineage.rs` | Table of admitted backend/shape cells and generation dependencies; consume P1's interface findings instead of independently redesigning it |
| P5 / Luna low | G5/G6 preparation: catalogue, ADR-0055 section 3 and programme M7 | Inventory required commands, artifacts, fixture/resource needs and missing evidence; do not run heavyweight release checks or claim admission |

Initially P1-P3 are ready for independent scoping; P4/P5 are useful bounded
preparation if they do not compete for the same resources. Root integrates their
findings into a finite G3 scope with public acceptance and explicit file ownership.
P2/P4 conclusions involving the lease interface wait for P1's findings. A named
unresolved cross-component issue may justify Astra high/Opus; normal discovery
does not automatically receive the strongest route.

As the first G3 slice becomes ready, root implements through the harness while
remaining preparation continues. A fresh other-provider reviewer takes the next
ready review; completed investigators are reused only for new bounded questions.
After G3 closure, capacity moves to G4 and its reviews; after G4, to candidate-bound
G5 qualification. G6's licence/evidence inventory can start early, but final smoke,
digests and both-provider review require the actual immutable candidate. Any
candidate change invalidates affected downstream evidence.

G3 acceptance uses all eight command IDs from its ledger row; G4 uses its four
federation commands plus required mixed-source TLS cases; G5 uses its four
transport/live commands on the candidate; G6 uses ADR-0055 section 3. Resolve
these IDs from the catalogue when declaring task checks, rather than copying
stale command strings into new tasks. Build/test resources remain serialized.

## Separate harness maintenance queue

This is a proposed harness-only queue, not an application launch instruction or
a prerequisite to all G3 work. Prioritize a maintenance item only where it blocks
current delivery integrity or an affected check; historical fixture experiments
must not displace application completion with an unrelated repair programme.
Previously reported failures must be reproduced before treating counts as current.
All diagnostic workers are read-only; only the integration owner edits listed files.

| Task | Deliverable / file ownership for integration | Dependencies / acceptance |
| --- | --- | --- |
| A: inventories | Reconciliation proposal for `coding-harness/src/programme-capture-source-paths-v1.ts`, `programme-capture-protected-paths-v1.ts`, `protected-paths.ts` | Ready; inventory equality, protection preserved, files under 500 lines |
| B1: Ruflo fixtures | Exact mismatch report for `coding-harness/src/programme-v5-ruflo-runtime.ts`, `programme-v5-ruflo-contract.ts`, `programme-v5-ruflo-schema-v2-materialization.ts` | Ready; repair waits for authentic frozen artifacts |
| B2: system fixtures | Exact identity report for `coding-harness/src/programme-v5-system.ts`, `issue-8-system.ts` | Ready; executable and digest must agree; no historical repinning to pass |
| C: packing | Measured proposal for `coding-harness/__tests__/programme-v5-operator.test.ts` and `programme-v6-operator.test.ts` | Diagnosis ready; timing waits for stable HEAD and heavy-test slot |
| I: integration | One accepted slice; owner alone regenerates `.harness/manifest.json` and `.harness/controller-build.json` under `coding-harness/` | A, B1/B2 or C independently release a slice; build, affected tests, other-provider review, verify/commit/finish |

Initial allocation: existing root coordinates; Sol medium investigates A, Sonnet
investigates B1, Terra medium extracts B2 identities, Sonnet investigates C. These
are provisional task routes, not provider quotas. Root continues independent
integration preparation. No worker needs the complete programme history.

When A or C returns, root applies its accepted proposal while fixture investigation
continues. Passing checks release a fresh opposite-provider reviewer; root pauses
writes. Accepted review releases commit/finish, then the next ready slice. An
artifact-blocked B task does not prevent independent A/C completion. Additional
workers require newly discovered independent questions, not spare session slots.

Acceptance commands (from repository root; run through declared harness checks
for implementation slices):

```bash
npm --prefix coding-harness test -- __tests__/programme-capture-task-v1.test.ts __tests__/postgres-product-slice-closure.test.ts __tests__/manifest.test.ts
HARNESS_REQUIRE_NATIVE_INTEGRATION=1 npm --prefix coding-harness test -- __tests__/programme-v5-ruflo.test.ts __tests__/programme-v5-system.test.ts --maxWorkers=1
npm --prefix coding-harness test -- __tests__/programme-v5-operator.test.ts --maxWorkers=1
npm --prefix coding-harness test -- __tests__/programme-v6-operator.test.ts --maxWorkers=1
npm --prefix coding-harness run build
node coding-harness/scripts/sync-harness-manifest.mjs --check
```

Use the existing lifecycle from `coding-harness/`: `npm run delivery -- REPO
begin TASK_JSON`, `bind ID OWNER NATIVE_JSON`, `advance ID OWNER`, `submit ID OWNER
RESPONSE_JSON`, then `verify ID OWNER`, scoped commit and `finish ID OWNER SHA`.
Each subcommand takes the same `npm run delivery -- REPO` prefix. Do not launch
historical candidate worktrees or rely on stale dual-mode templates.

## Setup completion and remaining evidence

This setup slice adds the plan, wires it into AGENTS and the harness README,
removes the misleading generated eight-agent setting, and corrects Claude's
worktree instruction. Existing model routing, source locks and review sequencing
remain sufficient; no runtime scheduler changes are justified.

Validate with the harness build, delivery runtime/workflow regression tests,
configuration parsing, local link checks, diff checks and independent native
Claude review. Keep the setup commit scoped and finish against that exact commit.
No application task was released by completing setup alone; the subsequent
explicit programme-completion goal authorizes the execution below.

### Execution checkpoint (2026-09-20)

Five bounded investigations ran: Sol lifecycle, Sonnet semantic admission/cache,
Terra acceptance/G1 evidence, Sonnet federation preparation and Luna release
inventory. Results are proposals until source-checked by the integration owner.
The lifecycle audit rejected a suggested redundant PostgreSQL digest extension:
immutable application admission plus exact binding identity and schema/session
lease revalidation already supply that link. Ordinary SQLite/MySQL generations
remain unprotected, a real G3 blocker; G4 also needs the admitted combination
matrix qualified. Do not infer distributed database atomicity, which is not an
advertised guarantee, from the shared application snapshot requirement.

The first scoped task, `g3-admission-reload-20260920`, qualifies the missing public
semantic-admission/cache reload evidence in `runtime_activation_http_tests.rs`.
It uses observed SQLite types, exact duplicate typed results, cached-plan pointer
reuse, invalid-candidate rejection, a held pre-body request across activation,
and bidirectional old/new plan rejection. This proves application-generation
isolation, not a database DDL lease or policy hot reload. G3 stays open.

Completed at `343c7e56`, `g3-sqlite-generation-qualified-20260920` integrates the opt-in
file-backed SQLite WAL/DELETE generation path. Backend, serving and actual CLI
tests cover transaction ownership, queued cleanup, exact binding, typed results,
lineage, schema refusal and automatic reload/recovery. Independent native review and harness verification passed. The next scoped task,
`g3-protected-portable-candidates-20260920`, admits portable policies through the
same SQLite/PostgreSQL leases, validates mapped policy columns before activation,
and qualifies denial, subject isolation and reload through native CLI tests.
A read-only worker identified the policy-only column gap while root implemented;
root remains the sole writer and serializes builds and live fixtures. Formal
Claude review follows the source-stable declared checks. Ordinary modes, native
PostgreSQL RLS and federated generations remain in G3. The MySQL slice below adds the third protected authored backend.
G1 receipts confirm the completed declared slices, but retained historical gap
wording still needs reconciliation with source before a final programme verdict.

Actual cross-provider throughput, independent-session ceilings and model-route
efficiency remain unmeasured. Resolve them through the above useful work, recording
observed concurrency separately from configured limits and completed outcomes.

The next slice, `g3-protected-mysql-generation-20260920`, integrates the proven
MySQL ordering primitive from `7c2aa47b` into candidate, request and response
ownership. Root writes; a read-only source audit identified the required
NO_BACKSLASH_ESCAPES mode and explicit raw-transaction rollback. The pinned TLS
CLI acceptance covers snapshot/DDL, reload/policy drift and native cleanup.
After source-bound checks, independent Claude review releases commit/finish.
Ordinary-mode/native-RLS G3 work and protected federation G4 remain dependent
work; the existing five dependency Clippy errors also need a scoped correction
before aggregate integration gates.

The active `g4-protected-federation-20260920b` slice combines the three authored
lease variants under the existing two-source path. Root owns integration; a
read-only worker audits cleanup and designs six lifecycle cases while another
reconciles G1 evidence. The latter found surviving path-SQL accounting and native
row-decoding obligations; the handover closure inference is not a gate verdict.
The live matrix covers all 25 ordered profiles, followed by bounded lifecycle
sampling. Source-stable declared checks and independent native Claude review
remain required before the scoped commit and exact-commit harness finish.
