---
status: implemented
date: 2026-09-26
updated: 2026-10-01
tags: [dev-process, metaharness, ruflo, parallel-delivery, openrouter]
depends-on: [ADR-0037, ADR-0048, ADR-0055, ADR-0057]
---
# ADR-0058: Upstream-first parallel hybrid delivery
- **Status**: Implemented; native output attribution, early integration, private-snapshot evaluator reservations and Cargo current-source revalidation corrected. Explicit owner-authorized evaluator/input migration uses an exact canonical commit/sourceDigest revalidateAgainst pin; defaults still reject stale hard inputs. Original candidate receipts/read closure remain unchanged, overlapping scope and active reservations refuse, every canonical check reruns and a fresh independent reviewer receives all changed inputs before verify/commit/finish. No automatic migration or author replay. September 29 native-progress repair adds delivery Claude `stream-json` partial events and Codex JSONL observation; legacy bounded adapters stay unchanged. Per-invocation `progress.jsonl` and bounded `native-tail.jsonl` contain task/stage/PID/time, tool lifecycle counters and text/reasoning/recognized StructuredOutput byte counts, never content, arguments or stderr. Substantive text/reasoning deltas, recognized tool-input/output progress and distinct tool start/result transitions refresh inactivity; warning at120seconds and cancellation at300seconds reuse existing process-group drainage. Heartbeats, malformed events, stderr, replayed tool transitions and unframed argument bytes cannot keep a call alive. Native cumulative stream volume no longer trips deterministic-check output limits; frames/final response and metadata tail remain bounded. Warning/cancel metadata emits immediately on existing stderr progress channel with durable evidencePath. Codex item events follow pinned0.157.1 exec_events.rs; cumulative text/output growth excludes unchanged item updates. Silent reasoning without observable events for300seconds is cancelled under the owner-required idle policy even if computation is healthy; this limitation is not a subscription failure. Codex reconnect error notices remain diagnostic; turn.failed is terminal. Full-stream digests bind both hosts without journaling contents. A separate bounded stderr tail redacts complete lines before truncation and retains redacted native failure diagnostics where available; ambiguous private-key blocks remain suppressed until explicit END, with suppression metadata rather than invented attribution; missing final envelopes fail explicitly. `DELIVERY_NATIVE_STALLED` is distinct from subscription unavailability and causes no automatic retry, model substitution or fallback. Frozen calls/receipts remain unchanged; subsequent invocations only. Deterministic check time/output policy and source/pool acceptance authority remain intact; focused fake-process and independent review evidence gate this repair.
- **Date**: 2026-09-26
- **September 29 redacted reasoning correction**: Live Claude emits `thinking_delta` with empty `thinking` and numeric `estimated_tokens`; installed native client derives `system/thinking_tokens` from these deltas. Treat positive safe-integer delta estimates within an active thinking block as substantive progress, recorded separately as `estimatedReasoningTokens`, never invented reasoning bytes or subscription budgets. Ignore duplicate system summaries, invalid/unframed estimates, pings and stderr. Block/message boundaries retire estimate admission. Keep 120s warning/300s cancellation, bounded capture, process-group drainage and no automatic retry/fallback. Historical mapping stall receipts remain negative; their counters cannot retrospectively prove exact event shapes. Frozen calls unchanged; subsequent invocations only. Truly unobservable reasoning still has the documented idle-policy limitation.
- **September 29 output correction**: Native Claude defaults `CLAUDE_CODE_MAX_OUTPUT_TOKENS=128000`; positive safe-integer overrides survive, while Claude Code retains its per-model clamp. No `MAX_THINKING_TOKENS`, fixed thinking budget, effort downgrade, Codex or API change. [Sonnet 5.5 overview](https://platform.claude.com/docs/en/models/sonnet-5-5/overview.md) documents 1M context/128K output; [Claude Code environment variables](https://code.claude.com/docs/en/env-vars.md) documents the unknown-alias 32000 default and model clamp. Fabric prioritizes bounded, credential-redacted stdout `is_error.result` over stderr warnings for completed processes; exact output-exhaustion attribution excludes quota/auth token limits. Process timeout/cancel/spawn/capture errors remain primary; auth failures still stop. Strict build and 33 focused regressions passed locally. Owner-authorized GCP deployment `53b8fd20` preserves six cloud application commits and passes build/44 tests. Failed output-exhaustion evidence stays negative; stale candidates require current-source recovery, not receipt/custody rewriting.
## September 29-30 coordinator repair
September 30 parallel-delivery recovery: existing coordinator launch instructions now match the configured ordinary Codex `gpt-6.1-sol`/`high` route through 9router, replacing stale Sonnet prose. Astra/medium coordinator, explicit task pins, Opus repair, learning and acceptance/refill remain unchanged. Local launch correction only; existing GCP coordinator is not restarted and application recovery remains separately source-bound. Direct integrator repairs made before a resumed request now compare against the prior substantive source, not the fresh request containing repaired bytes. Historical no-progress refusals remain immutable but do not reset that baseline; unchanged broken repair still pauses. Both regressions fail old code; 59 focused tests and independent source review pass. Actual cloud candidate recovery remains required; no author replay, artificial edit or new gate. October 1 injected-width proof exercises existing ready dispatcher/upstream pool at 16 and 32 simultaneous callbacks, four queued refills before held sibling drain, path/resource refusal, ordinary failure release and 16-way cancellation/drain. Existing accepted-parent fixture now dispatches its dependent through the ready adapter, verifies exact inherited bytes and rejects canonical drift; candidate-only parent admission refuses before executor. Canonical two-file suite passes 29/29 in 8.22s and strict harness/controller build passes; independent source review clear. cohort-drained.sourceRevalidated:false retains existing private-completion contract, not canonical acceptance. No production change, native model call, application throughput or cloud-readiness claim. October 1 same-task recovery: existing integrate input accepts explicit recover:true only for a paused, rejected prepared integration at its exact current canonical source and owner. Replacement preserves the complete task contract, exact prior immutable read/resource/accepted-source custody and normally validated candidate receipts; original admitted scope baseline survives repeated rejected joins, and changed scope bytes are required. Prior run and rejected scoped bytes archive unchanged; canonical check events/attempts remain so failed logs are not overwritten. Every current check reruns and existing integration review requires a fresh independent result before verify/commit/finish. New task IDs, author replay and weakened ordinary candidate binding are unnecessary. Interrupted apply resumes through ordinary integrate without the one-shot recover flag. Focused integration/workflow/runtime join81/81 and strict TypeScript pass in an isolated local patch snapshot; no application acceptance, cloud write or owner resumption.
September 30 local corrections: prepared integration admits disjoint ready/queued candidates from accepted sourceBefore/baseCommit; changed files restore exact Git blobs through existing immutable-copy guards. Missing/overlapping reads, IDs/writes/resources, unfinished apply and live runtime mutation refuse; main retains its writer. Cargo metadata narrows unrelated new review context, retaining declared/hard/pre-existing inputs and conservative fallback. Captured packet passes at746670bytes versus1117112 previously. Build/41 regressions pass, including reader refusal, pause/resume while independent lane runs, parent acceptance and refilled-lane integration. Initial Opus findings corrected; Sonnet5.5/high unchanged-route retry independently APPROVED (review SHA256 13f7f3d03a31c1abc8cbbfb132572f3acd66c523b6824055b65346fe40401449). Current-source native proof .metaharness/delivery/whole-outcome-proof-cCqzqQ/result.json (SHA256 544055d28c86081bb35db4aaddbad4174084ca2dc137c1380e555336f74828d0) passes two Sonnet5.5/high parents plus accepted-input child, planner overlap4429ms, clean drain; child started after siblings settled, so early refill relies on regressions. No application/cloud acceptance claim; prior failures and receipts immutable.
September 29 cross-cohort correction: private candidate admission excludes duplicate IDs, overlapping writes and shared resources, not snapshot read/write overlap. Canonical active-reader guards, accepted-parent pins and stale-read rejection stay at integration. Native proof `.metaharness/delivery/whole-outcome-proof-5p1gMM/result.json` (SHA-256 `f398ec684cfe5b960a243f6ec5788138eb360f2c905e50a8a1dfcc545d23e454`): nine Sonnet5.5/high stages, planner overlap4409ms, Claude PIDs1214015/1214028 overlap2498ms, CPU idle74.6%, zero memory/I/O PSI; sibling finished first, so early refill rests on deterministic tests. Later same-day evaluator-reservation correction: GCP XML package lanes (explicit readPaths, standalone `--manifest-path vendor/*/Cargo.toml` checks) reserved every nested `tests/`/`benches/` file repo-wide, so `DELIVERY_INTEGRATION_ACTIVE_DEPENDENCY` blocked unrelated endpoint-test acceptance. Ready reservations now carry `privateSnapshot` for candidate-local checks with no argv naming canonical or parent paths; this trusted-check classification is not a filesystem sandbox, so other live inputs must be declared or readPaths omitted; they reserve declared reads, accepted-parent inputs and live `coding-harness/`, `scripts/`, `config/` runtime. Snapshot evaluator inputs are re-pinned by `requiredDeliveryInputs` at that lane's own integration: stale inputs refuse, exact `revalidateAgainst` reruns checks and fresh review. Unflagged, legacy/frozen, omitted-read and canonical-argv reservations keep conservative pins; writes/resources refuse. Old code fails the endpoint-parent/child regression; build and 125 focused tests pass. Root source review and fresh build/35-test rerun pass; cloud adoption pending.
September 29 nested-file correction: private candidates create empty declared parent directories after snapshot unseal, fixing `ENOENT` without dummy source, canonical edits or relaxed path resolution. Reserved Git/environment/runtime paths refuse; ancestor collisions clean private copies; siblings stay out of scope. Three new expectations fail old code; build/73 focused tests pass. Cloud owner adopted `0e626419` in `d5842f0f` after form acceptance `bfbeb4c7`; cloud build/77 tests pass and HTTP child started. Follow-up lockfile parity copies Builder `095e9ec8f` exact generated dependency basenames: Cargo/npm/pnpm/Yarn/Bun/Poetry/uv lockfiles exempt only from authored-source line limit, never path/scope/512KiB/check/review guards. Vendor/source files remain limited. Nine generated-lock expectations fail old code; build and86 runner/candidate/integration tests pass after fix, including nested Cargo.lock and source/false-name/oversize rejection. New lockfile slice awaits safe cloud adoption; no source acceptance or throughput claim.
September 30 owner selection: new ordinary tasks default to native Codex `gpt-6.1-sol`/`high` for planning, authoring and fresh review through existing 9router; future confirmed nonexecuted HTTP402 fallback selects that same route. Explicit Claude host assignments retain Sonnet defaults; task/reviewer model/effort pins, legacy admitted fallback identities, Opus5.5/high capability repair, Astra coordinator and native learning remain. Existing Codex120s/300s inactivity/drain policy stays active. Strict build/88 focused tests pass; real selected-route delivery adapter/preflight returns structured READY at Sol6.1/high (local smoke only). Historical observations/receipts are immutable; no model-latency or cloud-adoption claim follows.
Initial local proof kept GCP stopped; later owner authorization resumed its existing coordinator. Harness repair never accepts application WIP. Native Sonnet 5.5/high through existing 9router, Opus repair and explicit pins remain. `npm --prefix coding-harness run coordinator` resumes existing `SEMANTIC_FABRIC_COORDINATOR_SESSION_ID` with Astra/medium; stop any previous writer first. Coordinator examines every unfinished authorized outcome at refill; status/handoff answers do not pause authorized work, while explicit owner pauses prevail. Ready execution retains upstream bounded pool and per-outcome read/write/resource reservations; canonical operation lock protects admission/snapshot/integration, not inference cohorts. Verified parents integrate serially and release accepted-input children while independent siblings continue. Private stages validate their snapshots; omitted read closure still pins all source. Declared semantic reads, evaluator/runtime files, manifests/locks, Cargo config/build scripts and toolchain remain hard pins. Cargo implicit source inputs no longer freeze unrelated canonical paths: only completed accepted-sibling lineage permits preserved-patch integration, then canonical `delivery <root> run <id> <owner>` reruns every declared check and fresh source-bound independent review over all changed implicit inputs before verify/finish. Original candidate receipts remain immutable; old review cannot attest combined source. Failed checks/review remain negative and stop acceptance, without planner/author replay. Cancellation retains custody until existing recovery resolves children. Pool drain reports `sourceRevalidated:false`; only integration validates acceptance. Earlier build/123-test evidence covers early integration; real Cargo same-crate regressions additionally exercise parent/child refill, hard-read rejection and current-source check/review failure. No new scheduler, automatic acceptance, cloud deployment or throughput claim follows from this local repair.
Native follow-up: `node coding-harness/scripts/live-delivery-pool-proof.mjs --whole-outcome --native` passed on runtime `d81e040b` with the native proof-script extension. Evidence `.metaharness/delivery/whole-outcome-proof-NJt8uW/result.json`, SHA-256 `0360e835aef457f3ffad55aed449bf1f92ccb46cc76a336bc7af6e29e6dbbf31`; `source-binding.json` binds all 411 runtime/proof/package files and all remained unchanged after final build. Nine actual Sonnet 5.5/high stage invocations plus separate client preflights completed two parents and their accepted-input child; scratch commits `616fbf2b`, `88f031de`, `3dbc72b5`. Planner intervals overlapped 3828ms including preflight; `processes.json` independently records Claude PIDs 786034/786445 overlapping at least 1505ms under runner PID 780948. Source remained unchanged, both owners drained, fresh review and canonical checks passed. Capacity samples: 70.98-73.16% idle during proof, 82/78% before final focused build/tests, zero memory/I/O PSI. Strict build and 51 focused regressions pass; root independent review approved. This proves local native fixture lifecycle, not application completion. Root owns separately authorized GCP synchronization/restart after review; this lane performed neither.
## September 28 implemented scope and exact evidence (historical)
Ordinary `delivery <root> ready <manifest.json>` reaches the upstream pool.
`mode: "run"` now invokes an upstream HarnessKernel whole-outcome driver:
architecture, scoped authoring, deterministic checks, capable repair, fresh review
and verification. Native calls reuse subscription adapters and process-group
custody; API calls reuse the bounded transport and exact proposal binding.
Ready cohorts preserve accepted-parent validation, path/resource exclusions,
cancellation drain and retained candidate evidence. Candidate success awaits
sole-root integration; no automatic main write or programme resumption occurs.

Run `npm --prefix coding-harness run delivery -- /absolute/repository ready /absolute/ready.json`.
Manifest fields: `schemaVersion: 1`, existing external absolute `parentDirectory`, positive `maxConcurrency`, `mode: "packet"`, `"propose"` or `"run"`, and `outcomes`.
Each outcome contains ordinary `task`, actual executor `handoff`, explicit `resources`, and optional `acceptedParent`/`acceptedInputs`. Packet mode awaits execution; proposal mode requires the isolated API route.
Continue returned `candidateRoot` with ordinary CLI status/submit/advance/verify. Reload validates canonical custody and source drift; candidate finish stays forbidden. Shared canonical API accounting preserves unknown-charge holds.
A real subprocess test continues implementation, checks, independent review and verification without claiming a real model invocation.

Native-only verified author observations now publish immutable deltas and reduce
serially into the existing PersistentRoutedAgentPool/Router. Assigned routes stay
pinned; persisted native evidence selects between eligible Sonnet/Sol medium only
after confirmed nonexecuted API 402. Mixed/API and infrastructure outcomes never
train native quality. Policy digests isolate learning regimes. Repair defaults to
two rounds, matching Builder's outer controller; this is not a subscription budget.

Explicit `evaluateDeliveryPolicies` uses upstream flywheel with ordinary isolated
native outcomes, frozen guidance-only levers, disjoint suites, per-task regression
checks and independent clean replay. Promotion requires at least five cases in
each suite. Signed activation binds policy bytes, suites, saved kernel receipts,
base/runtime/lockfile inputs and trusted signer/root. Local compare-and-swap keeps
previous activations for validated rollback. Ordinary delivery fully verifies
artifacts; unrelated task IDs retain seed guidance. Evaluation never updates
production routing, changes assigned models or installs policy automatically.
`config/delivery-policy.json` supplies operator-controlled signer/root pins;
without it ordinary delivery uses seed guidance. No policy was activated here.

Earlier implementation: strict build/manifest hardening and 138 tests across 13 injected suites passed (runner, learning, policy, ready/candidate/API/workflow, manifest, native adapters/client/process/runtime ledger); earlier pool slice passed 84 tests.
Activated policy reaches ordinary context without evaluation override. These are injected tests, not live model proof.
Flywheel 0.1.12 exposed an existing output-ceiling race: child exit zero can precede termination. Test now checks actual success refusal, bounded output and termination evidence, not OS timing.

### September 28 live whole-outcome proof
Command: `node coding-harness/scripts/live-delivery-pool-proof.mjs --whole-outcome`.
At `3ce6c2be`, two isolated fixtures passed real DeepSeek/high planning, authoring, frozen checks and fresh independent review through ordinary ready pool.
Evidence: `.metaharness/delivery/whole-outcome-proof-Oe3Dyf/result.json`; accepted candidates are not integrated application outcomes.
Proof digest: `0a84c877162780e561ead164221be7b98503649e8fad7fd3171a337c99531d4c`; both saved outcome hashes and upstream receipt chains verify.
Six confirmed API calls cost $0.002079054; maximum pre-dispatch request bound $0.2650685. Planner overlap was 5661ms; no speedup claim.
Earlier `whole-outcome-proof-wOy3Lr` remains negative ($0.00163645): unsupported fixture build argv stopped both outcomes; one real Opus/high planner repair succeeded.
Correction `3ce6c2be` preflights exact build/acceptance argv through the harness boundary; build and 18 focused tests pass. No model blame or blind replay.
Read-only restart shows no Fabric/scratch canonical active writer or operation; source unchanged. Initial/final host samples show 74.3% interval CPU idle; checks bounded to two.
Historical manual handoff: `manual-handoff.json` verifies review chains, four canonical checks, scratch commit `7e63869b735c106a2d2790f4e88d6e6061319934` and committed-byte readback (proof digest `ff9e24e262b84a1e84e2ca41d10c86cca79c5205d537675c82da1a56c4484693`). That proof lacked production receipt/state adoption and live child execution; the following slice closes those gaps, without rewriting history.
Production `integrate <integration.json>` takes `candidateRoot`, `id`, `owner`, `expectedDigest`. Root integrates, runs declared checks (canonical `run <id> <owner>` adds required fresh Cargo integration review), verifies, commits only admitted paths, then calls exact-commit `finish`; integration never auto-commits.
Original candidate workflow, logs and outcome receipts remain immutable. Canonical evidence copies them and revalidates receipt chains; durable per-file application supports partial-apply and commit-before-finish recovery. Same-base siblings require completed prior integrations and unchanged declared read dependencies; omitted `readPaths` pins all outside-scope source, including newly added files. Runtime/evaluator/package inputs cannot be omitted.
Real parents in `.metaharness/delivery/whole-outcome-proof-CSlRvE/` passed and integrated serially at `e12acc8469c6906148cf3462b608f2c2bf42e2bb` and `baf83f686b9408ae97b9dbd0b514db615d661fd6`; planner overlap 7134ms. One malformed planner required successful configured Opus/high repair. No speedup claim.
Original `result.json` stays negative: child proposed edits with `changes-requested`, rejected without source mutation. File SHA-256 `8ae0f509001ffc596ad06b62249e0541b5998f5d16722f75c59fd43f7f289925`; known API cost $0.01445282, no unknown charge. Adapter now rejects this contradiction before valid-output admission, preserves paid failure accounting, and clarifies proposal completion in the shared prompt.
Same child resumed through existing bind/advance/submit/check/review lifecycle, without parent or planner replay. Fresh real DeepSeek author and independent review passed; accepted input `product.txt="fixed\n"` produced `child.txt="fixed\nfixed\n"`. Production integration finished at scratch commit `518f91ac883fd56998a51405cdadfed953e82863`.
Continuation: `continuation-f7f6c1bb-1409-48c5-a3e7-1debde9ddd35/result.json` beneath that proof directory; file SHA-256 `c8a10a451af4d24c839dd69fabdac82b4c84b2719025ada5fa748d84a620ec0a`. Two additional actual API calls cost $0.000646005; original failed evidence remains unchanged. Root independently verified all three completed records, receipt chains, copied logs, fresh review identities and canonical checks.
Observed planner outlier `gen-1790598162-IhDVYBcrwYTwY1ECAsot`: provider OpenInference, generation 1628807ms, latency 718247ms, confirmed $0.00912259; completed, not cancelled. No routing, timeout, reasoning or resource limits changed. Host samples every 30 seconds during live work: 69-81% CPU idle; final child continuation 80.2%, one deterministic worker; memory/I/O healthy.
Final scoped validation: strict build/hardening and 112 tests across 10 suites pass, including 13 integration regressions and contradiction rejection. Independent read-only review cleared corrections. Full suite remains non-green: 1087 passed, 29 failed, 2 skipped; 16 failures reproduce on clean baseline, remaining failures concern historical native/Ruflo/config/source pins and two packed-controller timeouts. No application repair, resumption, policy activation, publication or release-safety claim.

Owner authorized direct implementation, repair, builds and tests for all work,
with both configured native hosts and isolated OpenRouter execution. The harness
remains optional orchestration/learning, not a compulsory application gate.
Application programmes remain paused; no push or publication is authorized.
This supersedes the narrower maintenance-only wording in the historical plan.

The existing delivery CLI now exposes `propose`, using source-bound pending
requests and an isolated API adapter. New tasks without an explicit host default
to DeepSeek/high; explicit native assignments remain unchanged. API handoffs
record actual API authentication and usage, never pretend native execution.
Known malformed output is task-held; unknown completion and abandoned process
reservations stop dispatch. Live process-owned reservations permit concurrent
calls. Source application, checks and acceptance stay with the existing main-only
integrator and MetaHarness verifier seam. No second scheduler was introduced.

Node 24 engines/declarations and strict library checking are active. Commits
`acec0d71` and `1197b258` add bounded API proposals, exact application binding,
repair prerequisites and process custody. `a3d8c8d9` completes the real
`known-http-classification-20260928` outcome: DeepSeek/high author and fresh
DeepSeek/high review, exact proposed bytes, build and 55 tests, valid MetaHarness
receipts and exact-commit finish. Known charged HTTP failures keep confirmed cost;
they never become unknown completion or a credit fallback. Author/review cost was
$0.0440979; terminal run digest is
`86e38ed69b955828d1b9f655e03bee3ccb34c138eea9322b0d280d9913293fe7`.

`e20781fe` adds the actual `@claude-flow/cli` 3.47.0 bounded-pool leaf,
`dist/src/services/bounded-worker-pool.js`, SHA-256
`757824847c1b3a394f78441f84e519a0edcdf6731d39fdfcd7fb2ed37a00fce0`.
It invokes no Ruflo CLI or MCP server. Thin adapters preserve path/resource
exclusions, pre-abort checks, noncooperative drain and retained failed evidence.
Non-Git candidates reuse the ordinary lifecycle, require external parent roots
and exact-file scopes, reject tracked runtime artifacts, and enforce original
out-of-scope source before begin as well as afterward. Only main can integrate.
Accepted-parent input requires a completed integrated outcome and unchanged file
bytes/mode; unrelated later commits do not invalidate unchanged selected inputs.

Final validation: `npm run build` and 77 tests across delivery candidate, API,
runtime, workflow and manifest suites. The existing manifest regression exposed
missing ADR-0056/0057/0058 protection entries; the independently reviewed fix adds
only those three paths and generated metadata. Build retains production hash coverage
and runtime hardening. The optional pool needs the full development install; its
standalone leaf has no imports and is absent from the frozen controller's 92-module
closure. Existing Router/learning paths stay intact; no new promotion is claimed.
The full development dependency audit reported 35 advisories (24 moderate, 10 high,
one critical); this slice does not claim broad dependency remediation or release safety.

Live proof records are local under `.metaharness/delivery/pool-proof-<digest>/`:

- `b4e89dbd1fbe7e65e74b075e86351e0d91415e7f35363009a4ac60fd95543c09`
  records 124202ms actual API overlap, $0.0872046 known cost and accepted custody
  review. Its source review rejected a pre-begin baseline gap; the batch stays negative.
- `7a54367acc4da800473ea079d83b5029617a55fa89f53d7112faad379d6620a6`
  records accepted fresh source-only review after repair, valid kernel receipt,
  65 tests/build and $0.0567822 known cost. It claims no new parallel overlap.
- The join reuses custody claims only for byte-identical pool/context/process/API
  and package files. Changed candidate code is covered by the final source review,
  never by the earlier receipt. Earlier rejected batches remain untouched.
- Both review packets consumed `coding-harness/src/delivery-api.ts` from the
  retained external child snapshot, accepted from `a3d8c8d9`; SHA-256
  `e4de48b7c0348fed7d866ff50b0a40a6dc07d2596b9e322ed6651439363a90e4`.
  Final child root is `/tmp/fabric-pool-proof-Np2cYj/delivery-candidate-OqKGGQ`.
  No unknown charges occurred in these proofs.

Read-only restart inspection after acceptance found no active writer or operation:
90 completed, 88 superseded and five paused historical task records. Paused SQLite
demand-driven work still needs owner resumption and source reconciliation; older
scope-amendment/native-stall records are history, not ready dispatch. No application
task is authorized ready by this audit. Direct execution remains available after
resumption; harness use is optional. Native fallback admission is tested, not a new
live native-repair claim. No application concurrency, speedup, GCP parity, broader
evolution or product/release completion follows from this bounded implementation.

## September 28 implementation handoff (current plan)
This section supersedes conflicting September 26 proposals below. Those sections
remain historical source findings, not another executable checklist. The owner
authorized the bounded implementation evidenced above; this is not a claim that
Fabric copied every Builder capability. No publication or application
programme resumption follows from documentation or harness acceptance.

### Finish criteria and scope discipline

Compose upstream tools with thin project adapters, not a second framework.
Preserve working acceptance, parallelism and learning. Do not copy Builder's entire
private runtime, add a scheduler/service, regenerate a working harness, or install
unused packages. Before changing a seam, show its actual local defect or missing
contract; reuse the existing implementation when it already satisfies the contract.
Keep one task/outcome through repair; no new task, calibration, inventory or repin
for each finding. Focused regression, independent review and impacted build once
per coherent slice; broader join at acceptance, not between every small repair.
Direct implementation, repair, builds and tests are allowed for application and
harness work. Optional orchestration retains parallelism and learning; no compulsory
harness gate remains. Direct edits never inherit old worker acceptance receipts.

Completion means: configured routes work, deterministic checks and independent
review pass, failure/repair and source-handoff contracts are proved, and restart
admission passes without starting the programme. Successful cheap/frontier cascade
counts as success. Perfect DeepSeek-only execution, model bake-offs, highest harness
score, a PR system, or new evolution benchmarks are NOT completion gates.
Preserve/exclude pre-existing managed helper files. Concurrent updates are expected
only where owner-confirmed; never absorb them into scoped harness commits.
Unexpected owned source changes still require reconciliation.

### Reuse and remove map

| Responsibility | Required composition / removal |
| --- | --- |
| Stage execution | Retain actual `@metaharness/harness` kernel/pool/verifier callback seam and project acceptance. Factory/templates scaffold; they do not prove active execution. |
| Concurrent ready callbacks | First assess `runBoundedPool` in Ruflo `v3/@claude-flow/cli/src/services/bounded-worker-pool.ts`, as documented in Builder ADR-0054 R8. Wrap existing outcome callbacks, not a new agent platform. |
| Export compatibility | Pool is exposed through `./dist/*`, not stable dedicated high-level API. Resolve installed export/declarations on Node 24, record package/version/lock and cancellation behavior. Declare any dependency actually used; no reliance on an accidental global install. |
| Scheduling authority | Pool bounds callbacks; project retains dependency acceptance and same-file/ancestor-path/named-resource exclusion. Do not remove them or claim pool handles them. Root releases children only after integrated predecessor source reaches their input. |
| Source isolation | Non-Git candidate snapshots/patches, one integrator on the authorized checkout specified below. No new branches/worktrees/PR machinery is required for this repair. Earlier worktree proposal needs separate future authorization. |
| Planning | Share architecture renderer across native/API wrappers; packet-only mode explicitly has no tools, retains admitted source and requests a plan instance, not schema definition. Native contract stays unchanged. |
| Review | One existing coordinator for ordinary/recovery paths; one production prompt renderer shared by native/API wrappers and qualification. Remove duplicate active prompts/coordinators only after preserving stronger checks. |
| Evidence | Reviewer sees current admitted source, patch, task and sanitized deterministic results/file-policy facts; never hidden evaluator, expected verdict or author rationale. Bind qualification to original source/evaluator, not today's checkout. |
| Progress | Use existing process/progress seams for PID/start, phase, throttled activity, safe tool names and completion. Bound capture; callback errors cannot orphan children. No raw prompts, tool bodies, credentials or reasoning in ordinary logs. |
| Repair | Invalid completed output retains actual usage/cost and safe diagnostic, not completion-unknown. Infrastructure/stale-source belongs to integrator; evaluator mutation to evaluator owner. Identical failed patch stops, changed repair remains eligible. |
| Learning | Preserve existing Router/outcome/memory/evolution connections and sole-writer reduction. Keep explicit local promotion/qualification boundaries; packages installed or settings present are not operational-learning proof. No new benchmark programme or removal of learning to simplify delivery. |

Builder inspected pool versions 3.38.20 locally and 3.45.0 on GCP, not this repo.
Its pre-aborted signal may still start callbacks; timeout can return before a
noncooperative callback stops. Check cancellation before dispatch and retain resource
ownership until child termination is observed. Verify on the intended server before
claiming deployment parity. Existing supported pool may be retained if equivalent.
Keep remote inference concurrency separate from Cargo/.NET/Node job workers.
Sample effective CPU, interval utilization, memory and I/O pressure before heavy
work, every 30 seconds and at refill; account for all jobs on that host. Use supported
worker controls, isolated targets/DBs/ports; do not invent CPU-based model-session caps.

Inventory factory/kernel, Router, Darwin/GEPA, AVO/Flywheel, AgenticOW and QE only
where present or required by accepted local decisions: label installed, configured,
operational and deferred separately. Reuse existing hooks/adapters rather than
local substitutes. Preserve ordinary outcome capture and learned policy reduction;
promotion still needs real evaluator and held-out proof. Missing optional promotion
evidence neither strips learning nor blocks delivery, and does not authorize a
new evolution campaign.

### Approved target routing; local activation remains an implementation step

- Eligible planner/implementer and separately qualified fresh-context reviewer:
  `deepseek/deepseek-v4.1-flash` via direct OpenRouter, not 9router API forwarding.
  Qwen/GLM are not this target. Same-model review uses a separate qualified reviewer identity and fresh API request,
  without author conversation/rationale; it is not cross-model/developer consensus.
  Preserve stronger declared review policies.
- Capability/output failure: configured capable native subscription repair,
  Builder reference Opus/high `cc/claude-opus-5-5[1m]`. Verify exact client alias
  locally; no model rename presented as runtime proof.
- Only proven nonexecuted HTTP402 credit rejection permits lighter Sonnet/Sol
  subscription fallback. Admit Sol only where Codex is authorized; Claude-only
  repositories use eligible Sonnet after policy amendment. Never Opus solely for
  exhausted credits. Authentication, unknown completion and local request-limit
  failures do not authorize fallback. Native unavailable pauses with exact
  client/model/error; never switch transport silently.
- Enforce maximum $1/request before dispatch; no cumulative/task/session ceiling.
  Preserve actual versus unknown charges, reservation/replay protection and actual
  fallback author/reviewer identities. Keep API keys out of native/tool/verifier
  environments. Existing native subscription gateway settings stay intact.
- Builder defaults: high reasoning, `reasoning.exclude=true` (hide returned thoughts,
  not disable computation), 131072 max output tokens, 1800000ms API timeout,
  2000000 response bytes, provider ceilings $0.50 input/$2 output per million.
  These are configurable defaults, not test-sized budgets or performance promises.
- New confirmed-output holds bind task digest, policy and packet class, never
  disable unrelated tasks. Retain same-task exclusions and unknown-charge stops.
  Old unscoped failures remain effective until explicitly evidence-migrated;
  never erase receipts or rotate a policy merely to bypass a hold.

### Builder donor evidence and bounded acceptance

Read Builder ADR-0051/0053 updated September 28 and ADR-0054 R8, then inspect donor
diffs rather than cherry-picking across unrelated runtimes:
`cf00eb06b` progress/cancellation and maintenance-authoring amendment;
`3430c6365` shared review/file-policy evidence/unchanged-repair stop;
`803cff426` production-context reviewer qualification;
`e759396a6` confirmed-credit Sonnet/Sol fallback;
`fa82f77fa` shared planner and completed-proposal diagnostics;
`701ea878b` task-scoped holds and fixed finish criteria.
Donor path prefix: `src/tools/application-development-harness/src/`;
inspect `native-worker-contracts-v1.ts`, `control-plane-review-v1.ts`
(if renamed, follow `ordinaryReviewAgentV1`), `native-model-invocation-v1.ts`,
`hybrid-worker-implementation-v1.ts`, `hybrid-suspension-v1.ts`,
`hybrid-credit-fallback-v1.ts`, and qualification/runtime adapters.

Builder live `deepseek-full-cascade-live-20260927-r2` passed in 236410ms:
DeepSeek plan, rejected implementation, Opus repair, DeepSeek approval; all
verifiers passed, API cost $0.02369829258. Receipt
`sha256:9f7324f0d77de7b0622e788a96a2e783f56f4f997c9d8455f34df58a3be8729e`
verifies; final scoped regression was 181 tests/build/restart audit.
This proves Builder cascade, not sibling adoption or perfect DeepSeek authorship.
Upstream ADR-127 (Accepted June 18) uses exact search/replace sentinel blocks;
Builder uses JSON edits. Keep safe exact matching; neither transport is proof
of model correctness. Do not blame JSON without captured evidence.

Acceptance in this repository, within one repair outcome:
1. Reconcile policy/config once, then implement only local gaps mapped below.
2. Inject transport/process tests for source/review binding, failed and unchanged
   repair, safe progress, cancellation, task A held/task B eligible, HTTP402 versus
   auth/unknown failures, credit fallback independence, $1 bound and no total cap.
3. Exercise two genuinely independent existing outcomes through compatible upstream
   pool with actual overlap evidence; retain conflict exclusion. Reuse accepted local
   proof when source-current. Prove one dependent receives accepted parent source.
   If programme graph is truly serial, report that constraint; do not invent
   application independence, strip business dependencies or launch filler tasks.
4. Run one local end-to-end cascade with independent review and authoritative
   checks, accepting legitimate frontier repair. Preserve failed attempts.
5. Integrate verified source serially; run read-only restart admission and document
   active/ready/manual/blocked work separately from harness defects. Record
   installed versions, exact commands, source/receipt identities, costs and limits.
   Update this ADR to Implemented only for proven scope; leave programme stopped.

## Historical Fabric baseline gap map (September 28)

Rechecked clean main `99af9439121ea0879496b8bac61a349a3caa1cf2`.
Fabric is Rust semantic-data delivery; preserve public-query and conformance
acceptance plus SQLite/PostgreSQL/MySQL isolation. Harness is development-only.

- Keep `coding-harness/src/delivery-cli.ts` begin/bind/advance/submit/check/verify/
  finish/pause/resume lifecycle and `delivery-stage.ts` kernel seam. Its supplied
  callback result is not proof a model was launched; wire executor only where absent.
- `delivery-contracts.ts` native handoff rejects slash-qualified model IDs.
  Add explicit API/native transport variant, not a fake native OpenRouter identity.
  `assertHostEnabled` currently has no execution callsite: wire admission without
  breaking historical receipt parsing.
- Reuse `delivery-runtime.ts` / `delivery-workflow.ts` source/check/review join;
  consolidate shared prompts/review coordination, bounded progress, exact error
  attribution and unchanged-failed-candidate stop at those seams.
- Keep `delivery-workspace.ts` OS lease/PID custody and main-only integrator;
  separate non-Git candidate observations from canonical integration. Global
  `active.json` writer lease must not be removed before candidate ownership exists.
- `delivery-process.ts` inherits target/job/DB settings, not resource allocation.
  Allocate private Cargo targets, SQLite files, database schemas and ports;
  validate locks across invocations. Serial learning reduction keeps one writer.
- Local AGENTS remains Claude-only/native-only; amend owner-authorized routing
  policy and relevant ADR-0037/0055/0057 instructions together before API activation.
  Preserve Fabric's hold on expanding evolution research; retained learning and
  honest local outcome capture do not require that separate programme.
- Package engines currently admit Node20 and `@types/node: latest` is not a
  Node24 guarantee. Pin major24 declarations and prove strict dependency closure.

Validation commands may run directly or through an optional delivery task. Scope,
source ownership, meaningful acceptance and independent review remain required.
```bash
npm --prefix coding-harness run build
npm --prefix coding-harness test -- --run __tests__/delivery-runtime.test.ts __tests__/delivery-workflow.test.ts
```
Build already includes TypeScript, manifest sync and hardening; retain all three.
Add focused route/admission, process and pool contracts in existing test layout.
Use affected Rust/database tests and ADR-0055 integrated public-feature join;
skipped database tests stay not-run, not passing. No broad release programme.

## Historical September 26 assessment (superseded instructions)

Source findings below retain their original date and evidence limits. Earlier Qwen,
worktree/PR prerequisites and heavier orchestration proposals are historical, not
current implementation direction. Use September 28 handoff above for execution.

## Purpose and authority

Make independent Fabric outcomes execute concurrently through the existing
delivery harness, with isolated source/build/database state and exact integration
evidence. Preserve Rust product acceptance, native escalation and independent
review. Neither role records nor an installed kernel prove parallel execution.

The requested target is Node 24, direct OpenRouter
`deepseek/deepseek-v4.1-flash` authoring and independent
`qwen/qwen3-235b-a22b-2507` review, native subscription escalation through configured routes,
at most $1 per metered request and no cumulative spending cap. Native subscriptions
have no cost, token, request, invocation or provider-quota budget. Model admission
follows ready dependencies and measured host resources, not a fixed session cap.
The requested source workflow uses task branches/worktrees and reviewed PRs.

That target conflicts with current main-only personal instructions and Fabric's
Claude-only/native-only policy. This proposed ADR records those conflicts as
repair prerequisites; it does not abandon the target or silently authorize
worktrees. First reconcile the governing instructions explicitly, then change
guards and dispatch. This review changes no execution policy, source, package,
runtime state, scheduler, publication or application pause.

## Exact local baseline

Inspected `/home/claude/src/hm/semantic-fabric`, canonical `main`, clean at
`f8d8acf817f888b6d63dd5960ac56cc78de4db30` on 2026-09-26. Local Node is
`v24.14.1`. Installed `coding-harness/node_modules` contains
`@metaharness/harness` 0.2.0, `@metaharness/router` 0.4.0 and `@types/node`
20.19.43. `@claude-flow/codex` is absent at that package location; this is not
a host-wide absence claim. Manifests use `latest`, but engines admit Node 20.

ADR-0037 is Accepted, dated 2026-08-25, updated 2026-09-10. ADR-0055 is
Accepted, dated 2026-09-07, updated 2026-09-23. ADR-0057 remains Proposed,
dated 2026-09-24, updated 2026-09-25. Their historical assertions are not
substitutes for current source inspection or runtime proof.

This is local source/package evidence, not GCP inspection or server validation.
No model job, build, database test or throughput experiment ran. No Fabric-scoped
Ruflo MCP connection is available in this session; Builder's connection must not
hold Fabric facts. ADR graph registration is pending the correct project connection.
User memory was searched and the exact phase-gating pattern retrieved; its
superseded native-only guidance cannot override the current requested target.

## Findings: active, miswired and missing

Paths below are repository-relative at the baseline commit.

| Area | Evidence | Classification and implication |
| --- | --- | --- |
| Active entry | `coding-harness/package.json:11`; `coding-harness/src/delivery-cli.ts:8` | `delivery` invokes the existing external-host lifecycle, not a background worker launcher. Keep this public entry. |
| Real kernel | `coding-harness/src/delivery-stage.ts:8` | `verifyNativeStage` uses `HarnessKernel`, `AgentPool`, `AlgorithmRouter`, `PolicyGate`, `VerifierRegistry`. Its worker returns an already supplied response; it invokes no model or concurrent task scheduler. |
| Outer controller | `coding-harness/src/delivery-runtime.ts:92`; `coding-harness/src/delivery-workflow.ts:51` | Source-bound begin/bind/check/repair/review/finish exists. Declared checks are sequenced; task state is not a DAG dispatcher. |
| Writer exclusion | `coding-harness/src/delivery-runtime.ts:114`; `coding-harness/src/delivery-workspace.ts:15` | One `active.json` writer and main-only root guard deliberately reject independent writing lanes. Increasing model settings cannot remove this serialization. |
| Operation custody | `coding-harness/src/delivery-workspace.ts:87` | OS lease plus PID/start identity and child checks exist. Preserve recovery; do not replace with an unowned lock file. |
| Host admission defect | `coding-harness/src/delivery-contracts.ts:69` | `ENABLED_HOSTS` and `assertHostEnabled` exist but have no source callsites. Structural parsing accepts Codex; begin/bind do not enforce the declared Claude-only guard. This is miswired code, not a missing constant. |
| API route missing | `coding-harness/src/delivery-contracts.ts:53`; `coding-harness/src/delivery-contracts.ts:31` | Model validation rejects slash-qualified IDs, hosts are native-only, handoff authentication is `native-subscription`. Direct OpenRouter requires an explicit transport variant, not relabelling a native handoff. |
| Runtime mismatch | `coding-harness/package.json:22`; `coding-harness/package.json:30` | `@types/node: latest` resolves to major 20 and engines allow 20. Running Node 24 does not establish a Node-24-only contract. |
| Resource controls | `coding-harness/src/delivery-process.ts:8`; `coding-harness/vitest.config.ts:9` | Cargo job/target settings are inherited and Vitest has a local two-worker bound. These are not shared resource admission or an evidence-based server allocation. |
| Output and DB isolation | `coding-harness/src/delivery-runtime.ts:141`; `coding-harness/src/delivery-process.ts:11` | Check logs are attempt-scoped, but caller-provided `CARGO_TARGET_DIR` and `SF_TEST_*` are not allocated per task. Independent checks need explicit target, database/schema, port and cleanup ownership. |
| Historical worktrees | `coding-harness/src/git-worktrees.ts:319`; `coding-harness/README.md:283` | Detached-worktree machinery exists in the older experiment and is explicitly prohibited for current delivery. Do not revive it as an unreviewed parallel adapter. |
| Configuration wiring | `coding-harness/src/delivery-cli.ts`; `.agents/config.toml` | Active delivery does not consume `loadSwarmAutomationConfig`; no `[swarm.automation]` block was found in the two inspected Codex config files. Adding one alone changes no delivery behavior. |

The source-wide snapshots also mean a disjoint edit can invalidate another task's
review. A path ownership table alone cannot make parallel writers safe against
that evidence model. Bind each candidate to an immutable source identity before
checks, then revalidate the combined integration source.

## Reusable upstream interfaces

Grounding: Brain lookup on 2026-09-26 returned Ruflo
`v3/@claude-flow/codex/src/dual-mode/cli.ts` and its tests. Detailed upstream
inspection is pinned in Semantic Builder ADR-0054 at Ruflo
`a91db6768ba5e8c9e31c840ad25c1c96478f3f0f` and MetaHarness
`7bffe2f58ee72c367cea07beb62ee00fc2bee755`. These are source pins, not proof of
npm parity or installation on this host.

- [`DualModeOrchestrator` and configuration loader][orchestrator] provide native
  Claude/Codex execution with dependency graphs and separate worker/writer bounds.
  Use `runCollaboration` only after a compatibility test proves effective config,
  native gateway preservation and permitted source roots. The loader chooses the
  first existing `.agents/config.toml` or `.codex/config.toml`; it does not merge.
- [`CodexWorktreeCoordinator`][worktrees] already supplies `prepare`, `status`,
  `integrate`, `cleanup`. Reuse after policy reconciliation; do not build another
  Git workspace manager. `prepare` defaults to HEAD; pin an accepted base SHA.
  `integrate` is a local merge, not a GitHub PR workflow or automatic predecessor
  source transfer. Do not call it as an implicit merge authorization.
- [Ruflo's CLI][cli] chains repeated workers unless `--parallel-workers` is set.
  The default feature template is sequential. Dependency batches have barriers;
  neither increasing bounds nor declaring `dependsOn` proves continuous refill
  or accepted source handoff into already prepared worktrees.
- Keep [MetaHarness kernel][kernel] verification and public pool/router APIs at
  the current ready-stage seam. Its serial step loop is not the missing outer
  concurrent executor. Ruflo ADR-324 is Accepted (2026-07-28); ADR-327 remains
  Proposed (2026-07-28), and MetaHarness ADR-047 remains Proposed (2026-06-16).
  Proposed distributed fencing is not a prerequisite for ordinary local delivery.

Two real integration gaps remain: this upstream dual-mode worker interface is
native-only, and its memory bootstrap uses Ruflo CLI. The requested API transport
and MCP-only memory policy therefore need a tested adapter seam or an upstream
change. Do not assert they are existing options or launch a forbidden CLI fallback.
Keep the existing external-host contract usable while establishing compatibility.

## Bounded repair sequence and acceptance

1. **Resolve policy and freeze the adapter contract.** Record effective provider,
   model, worktree, integration and PR authority in one consistent instruction
   update. Preserve historical records without permitting disabled execution.
   Freeze tests showing all admission entrypoints reject disabled hosts; wire
   the existing guard rather than duplicate it. Pending explicit worktree policy
   reconciliation, use read-only research and existing main-only delivery only.
2. **Establish Node 24 and transport types.** Set development engines/tooling to
   Node 24 and `@types/node: "24"`; test the full strict type closure. Add an
   isolated OpenRouter author/reviewer contract with exact model IDs verified
   against current provider metadata. Verify availability and pricing of the
   exact requested IDs above; do not silently reselect either model.
   Bound maximum input/output cost before every call; reject unknown price or a
   bound above $1, retain unknown-charge accounting and same-request replay
   protection, and represent unlimited cumulative allowance as `null`.
   API credentials stay only in the API adapter, never native/check environments.
   Rejection-driven native escalation preserves exact route/effort; native
   unavailability reports client/model/error and pauses, without fallback.
3. **Adapt upstream execution and source ownership.** Keep `DeliveryHarness`
   task/check/review evidence; add only integration needed to invoke verified
   upstream executor/worktree APIs. Independent candidates own non-overlapping
   paths and pinned bases. Accepted predecessor commits must be materialized and
   checked before dependent dispatch. One integration owner resolves manifests,
   locks and accepted PR order; Fabric integration and documentation target
   canonical `main`. PR creation/push/merge remain separately authorized.
   Test dirty-state retention, failed dependencies, stale handoff rejection,
   cancellation and no writes outside the assigned source root.
4. **Add shared heavy-work admission.** Allocate build and test resources separately
   from model sessions, using current cgroup/affinity CPU capacity, interval CPU,
   memory peaks and I/O pressure across projects sharing that host. Each server
   has separate admission; do not pool CPU capacity across hosts. Account for Cargo jobs, Rust test
   threads, nextest and Vitest children. Give each candidate a private Cargo target,
   SQLite files, PostgreSQL/MySQL database/schema, ports and output directory.
   Test conflicting claims, crash recovery and cleanup of owned fixtures only. October 1 storage fix: workspace dev/test use limited `debug=1`, `incremental=false`; real offline Cargo build/test graphs retain assertions/overflow checks. Ordinary checks/native clients also set four exact `CARGO_PROFILE_{DEV,TEST}_{DEBUG,INCREMENTAL}` defaults after sanitization so historical manifests receive them; Codex shell dotted keys preserve configured entries even with `inherit=none`. Explicit `CARGO_INCREMENTAL` remains higher precedence; frozen qualification unchanged. Existing lock-protected incremental cleaner now discovers shared targets as well as private lanes; bins, source, receipts and frozen evidence remain. Historical compiled variants still need owner-confirmed retirement; no cloud adoption or bounded-total-storage claim.
   A skipped database suite remains not-run. Never equate model count to CPU count.
5. **Prove one useful parallel outcome.** Record overlapping native/API worker
   timestamps and PIDs/request IDs, exact source and accepted handoff hashes,
   effective routes, resource samples and independent review. Compare whole
   verified-outcome time with a serial baseline on the same work; do not claim
   speedup from isolated model latency. Run focused Fabric semantic/public-query
   tests and impacted builds, then integrated regression at the outcome join.
   Preserve per-task receipts and serial learning reduction. No GCP claim until
   the same evidence is collected on the named server.

Keep implementation in the existing harness workflow with frozen regressions and
incremental verified commits. Do not introduce a second scheduler, generic service,
benchmark programme, API fallback or evolution gate merely to repair these seams.

## Consequences

Upstream reuse avoids duplicating native execution and workspace management while
preserving Fabric's acceptance boundary. Explicit source and resource ownership
costs integration work; upstream batching and API/MCP mismatches remain visible.
This proposal earns no product-completion points and changes no published routes.

## Links

- [Existing repair proposal](ADR-0057-repair-the-native-development-harness.md)
- [Current delivery decision](ADR-0055-v1-product-completion-and-release-profile.md)
- Cross-project research: `semantic-builder/docs/adr/ADR-0054-upstream-first-parallel-harness-execution.md`.

[orchestrator]: https://github.com/ruvnet/ruflo/blob/a91db6768ba5e8c9e31c840ad25c1c96478f3f0f/v3/@claude-flow/codex/src/dual-mode/orchestrator.ts
[cli]: https://github.com/ruvnet/ruflo/blob/a91db6768ba5e8c9e31c840ad25c1c96478f3f0f/v3/@claude-flow/codex/src/dual-mode/cli.ts
[worktrees]: https://github.com/ruvnet/ruflo/blob/a91db6768ba5e8c9e31c840ad25c1c96478f3f0f/v3/@claude-flow/codex/src/worktrees/coordinator.ts
[kernel]: https://github.com/ruvnet/agent-harness-generator/blob/7bffe2f58ee72c367cea07beb62ee00fc2bee755/packages/harness/src/kernel.ts
