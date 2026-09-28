# semantic-fabric

> Rust semantic-data application with Ruflo/MetaHarness development and evidence
> orchestration.
>
> **This file (`AGENTS.md`) is the single CANONICAL, shared instruction source for
> BOTH OpenAI Codex and Claude Code.** Codex reads it directly; Claude Code imports
> it via `@AGENTS.md` at the top of `CLAUDE.md`. Edit SHARED instructions HERE.
> Claude-Code-only guidance lives in `CLAUDE.md` (below its `@AGENTS.md` line).

## Harness repair authorization (2026-09-28 owner amendment)

The owner authorizes ADR-0058 implementation with the same model access and
limits as Semantic Builder. The Claude-only restriction below is historical and
revoked. Native Codex and Claude Code may execute and independently review
implementation, repair and review. Direct edits, tests and builds are authorized;
the harness is available orchestration, never a compulsory authoring/test gate.
Keep one integrator on canonical `main`, source-bound acceptance and independent
review. Application work remains paused until separately resumed by the owner.

Direct OpenRouter `deepseek/deepseek-v4.1-flash` authoring and fresh-context
independent review are authorized through an isolated adapter. Enforce at most
$1 per metered request before dispatch and `maxTotalUsd: null`; no cumulative
spending cap. Preserve accounting, unknown-charge stops and replay protection.
API credentials remain only in the API adapter, never check/native environments.
Capability/output failure admits configured Opus/high repair; only confirmed
nonexecuted HTTP 402 credit rejection admits lighter Sonnet or authorized Sol
subscription execution. Preserve actual executor identities and configured
native gateway transport. Native client/model failure pauses with its exact error.
Native subscriptions have no cost, token, request, invocation or quota budgets.
ADR-0058 governs this repair where historical policy below conflicts. No push,
publication, application resumption or expanded evolution programme is authorized.

## Historical Claude-only build (2026-09-22; superseded above)

Codex is not used for build, task execution, or review until the user
explicitly re-authorizes it. Every delivery task uses `host: 'claude-code'`;
independent review is a second, distinct Claude Code executor identity, not
Codex. This restores the same-provider policy last active 2026-09-15 through
2026-09-19 (commit `02736d56`, superseded 2026-09-19 by `1ad11c8f` when Codex
returned); the historical dual-provider sections below (session-capacity
amendment, parallel-execution policy) describe that later, now-paused period
and are retained as history, not current instruction.

Native subscription transport for Claude Code in this repository runs through
the user-authorized `9router` gateway on the Mac
(`ANTHROPIC_BASE_URL=http://macbook-pro.tail448fa.ts.net:20128/v1`,
already configured in `~/.claude/settings.json` with its own
`ANTHROPIC_AUTH_TOKEN` and `cc/claude-*` model aliases). This is a native
subscription transport substitution, not a provider-API-key or OpenRouter
exception: no other provider's API key, base URL, or fallback route is
permitted. Never print, log, or commit that token; it stays in the existing
untracked settings file.

### Independent model session capacity (2026-09-19 user amendment — historical, Codex paused)

Do not impose a fixed repository-wide session-count cap on independent Codex
(ChatGPT subscription) or Claude Code subscription processes. Four distinct
concurrent Codex sessions completed successfully on Codex 0.155.1; Claude
session capacity was not benchmarked. Do not add repository overrides for
native subagent counts. Client defaults and enforced per-session limits still
apply; independent sessions do not establish infinite capacity.
Select parallel work from ready dependencies and file ownership; preserve the
single integration writer and existing build/resource isolation. Historical
in-session capacity observations are not a global model-session limit.

### Parallel execution operating policy (historical Codex coordinator; superseded above)

The programme coordinator used native Codex `gpt-6-astra` with `xhigh`
reasoning effort (explicit user selection, 2026-09-20) while Codex was active.
With Codex paused (2026-09-22), the coordinating conversation is Claude Code
at Sonnet high effort via the 9router transport above; worker and reviewer
routes remain task-specific. Record actual runtime settings in harness
bindings, never a requested setting that the running host has not adopted.

Follow `docs/plans/native-parallel-execution-plan.md` and the delivery harness
README. Keep the existing coordinating conversation; provider capacity does not
determine coordination ownership. Dispatch bounded dependency-ready work through
native agents or independent native sessions, with explicit input revision,
scope, deliverable, acceptance checks and result recipient. Refill on accepted
completion; report active, ready, blocked and review queues and resource waits.
Keep one integration writer and stable source during checks and formal review.
Native agent concurrency and build/resource concurrency are separate controls.

Route each worker by its task, preserving the selected main model. Prefer tools
for deterministic work and escalate only a named unresolved question. The user's
2026-09-19 efficiency instruction permits comparing total tokens per accepted
outcome, including repeated context and rework, alongside latency and accuracy.
This is efficiency evidence, never a subscription price, usage budget, quota,
availability gate or reason to substitute an explicitly requested model.

## Rules

- Do what has been asked; nothing more, nothing less
- NEVER create files unless absolutely necessary; prefer editing existing files
- NEVER create documentation files unless explicitly requested
- NEVER save working files or tests to root; use `/src`, `/tests`, `/docs`, `/config`, `/scripts`
- ALWAYS read a file before editing it
- NEVER commit secrets, credentials, or `.env` files
- Do NOT add a `Co-Authored-By` trailer to user commits unless this project explicitly opts in
- Keep files under 500 lines
- Validate input at system boundaries

**ruflo-interface-contract:v2**

## Ruflo Interface Contract

- Use `search_ruvnet` for RuvNet source and capability claims when the Brain is installed; cite its source.
- Use `guidance_brain` / `guidance_recommend` and the live MCP registry for this process's actual registered, configured, reachable, healthy, and authorized state.
- Prefer a live structured Ruflo MCP tool for coordination, memory, routing, learning, and status. Discover deferred tools and schemas; never guess names or arguments.
- For a genuine Ruflo CLI-only gap, use `ruvnet_cli_help({executable: "ruflo", argv: ["<group>", "<command>"]})`, then `ruvnet_cli_run({executable: "ruflo", argv: [...]})` with the exact literal arguments that help authorized. Never guess `claude-flow` as the executable merely because an old package or instruction used that name.
- Direct shell is for bootstrap and administration that cannot depend on MCP: install/init, first MCP registration/start, diagnostics, and deliberate daemon work.
- Native Claude/Codex agents execute. Ruflo tracks a swarm only after `swarm_init` and `agent_spawn` create records; a native agent alone is not proof.
- Before generic testing or security agents, discover specialized installed QE or adversarial-security capabilities and disclose any fallback.

**ruflo-managed:swarm:v2**

## Swarm & Coordination

Use the smallest capable structure derived from dependency edges, shared-state risk, and required evidence instead of a file count.

- Independent one-shot native agents need no Ruflo swarm.
- For persistent topology, shared memory, or tracked handoffs, discover the live schemas, call `swarm_init`, then register each worker with `agent_spawn({agentType: "...", agentId: "..."})`.
- A tracked record does not launch a native Claude/Codex agent; launch the matching executor separately.
- Use exactly one integration writer on canonical `main`. Never create/switch branches or worktrees; historical recovery refs are read-only integration inputs.
- Read-only research may run concurrently. Continue independent work after spawning and wait only on a real dependency.
- Role strings such as `researcher`, `architect`, `coder`, and `reviewer` are labels, not proof of a specialized runtime.

**ruflo-managed:mcp:v2**

## MCP Integration

Use structured MCP tools for normal runtime work, then continue implementation. Coordination calls return immediately. Host-level registration is not proof of a tracked worker or a generated MetaHarness verifier.

| Need | Live structured tools |
|------|-----------------------|
| Guidance | `guidance_brain`, `guidance_recommend` |
| Swarm | `swarm_init`, `swarm_status`, `swarm_health` |
| Agents | `agent_spawn`, `agent_list`, `agent_status` |
| Memory | `memory_store`, `memory_search`, `memory_search_unified` |
| Hooks | `hooks_route`, `hooks_pre_task`, `hooks_post_task`, `hooks_worker_dispatch` |
| Status/performance | `system_status`, `performance_benchmark`, `performance_profile` |

Use AIDefence or other plugin tools only when the live registry reports them configured and reachable. Do not invent Hive-Mind, federation, workflow, claims, or session interfaces; discover the exact installed tool first.

**ruflo-managed:memory:v2**

## Memory & Learning

Memory is optional context, not a delivery gate. Use native Ruflo MCP/AgentDB tools for store, search, retrieve, recall, list, delete, statistics, diagnosis, and verification.

- Never open managed memory through direct SQL, `sqlite3`, `sql.js`, raw file reads/writes, or whole-image operations.
- A live `memory.db-wal` is expected while a native owner is active. Never checkpoint, delete, rename, replace, or unlink database sidecars.
- If recall fails or is safely refused, report it once and continue from repository/source evidence. Do not force a second driver or claim an empty result is healthy.
- Before relevant work, use `memory_search` / `memory_search_unified` and `hooks_route` when available.
- After a validated success, use `memory_store` and `hooks_post_task` when the result is genuinely reusable.
- Dispatch background work through `hooks_worker_dispatch` only after discovering its current schema and confirming that a worker is appropriate.

## Code Standards

- File organization: never save to root; use `/src`, `/tests`, `/docs`, `/config`, `/scripts`
- Files under 500 lines
- No hardcoded secrets or API keys
- Input validation at boundaries; typed interfaces for public APIs
- TDD (London School / mock-first) preferred

### Commit messages
```
<type>(<scope>): <description>

[optional body]
```
Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `chore`.
(Do NOT append a `Co-Authored-By` trailer to user commits unless the project opts in.)

## Security

- NEVER commit secrets, credentials, or `.env` files; NEVER hardcode API keys
- Always validate user input; use parameterized queries for SQL; sanitize output (XSS)
- Path security: validate all file paths, prevent directory traversal (`../`), use absolute paths internally

## Build & Test

- ALWAYS run tests after code changes; ALWAYS verify the build before committing
- Follow ADR-0055: focused affected tests/builds per coherent commit; full workspace checks at meaningful public-feature integration boundaries and release. Repeat broad checks only for changed code, failures, unknown impact or unresolved risk.
- The product runtime and every deployable dependency are Rust/Cargo artefacts
- Node/npm is development and evidence infrastructure only; run it only for a
  changed package under `coding-harness/` or another explicitly non-deployable
  harness boundary

```bash
cargo fmt --all --check
cargo test --workspace --locked
cargo build --workspace --locked
```

## Codex platform notes

### Current delivery execution (2026-09-28)

Direct implementation, repair, tests and builds are permitted for all work.
Use the delivery harness when useful for orchestration and learning; it is not
a prerequisite. Both native hosts and the isolated ADR-0058 API adapter are
authorized. Retain one integrator, scoped tests/builds, independent review,
source/data isolation and the current application pause.

### Historical delivery execution (ADR-0055, superseded by September 28 amendment)

- Every building task must use the main-only delivery harness in `coding-harness/` (ADR-0055, user corrections 2026-09-10, 2026-09-19 and 2026-09-22). Begin a scoped task before editing, bind the actual native executor/model/effort, then use `advance`/`submit` for source-bound implementation, declared checks, failure-directed repair and independent read-only review. With Codex paused (2026-09-22), the host is Claude Code for both implementation and review; independent review uses a second, distinct Claude Code executor identity, not a second provider. The existing native host executes each returned request; do not start another implementation host. Verify, commit only that slice, then finish against the exact commit. See `coding-harness/README.md` for response schemas, recovery and trust boundaries. Ruflo MCP records coordination/results; local receipts do not prove remote synchronization. Manifests use `latest`, with exact tested dependency resolution retained in the lockfile. The historical closed candidate experiment remains optional and its worktree launchers remain prohibited.
- After an explicit user pause/review request, do not resume application tasks from the scheduler or an active goal until the user releases that pause. Preserve interrupted patches; explicit adoption records starting changes without treating them as verified.
- Finish integration and public request behavior before starting another foundation. Use the finite G1–G6 ledger in `docs/plans/sota-application-completion-programme.md`, not a next-unmetered-helper queue. Audit prerequisite test callsites before freezing task scope; group compatible work by complete phase/public outcome, retaining incremental verified commits. Count reduced named obligations and public acceptance, not commits, tests, scores, reviews or receipts alone.
- Preserve the selected main model. Use tools for deterministic work, and Haiku for bounded mechanical tasks, Sonnet for established patterns and normal implementation/review (Sonnet high for a specific correctness proof). With Codex paused, Claude Code writes and a second, distinct Claude Code executor identity reviews; a `reviewer` route is same-provider by policy while Codex is paused, and must still use a different executor ID than every implementation handoff. Escalate to Opus only for a named unresolved problem; Opus high is the top Claude tier, reserved for a bounded exceptional task (Fable is no longer routed, 2026-09-23). `ultra`/ultra-tier effort is Codex-only and unavailable while Codex is paused. Route each new task afresh and stop stronger reviewers once their question is answered.
- Astra `max` and `ultra` are supported choices. Forward explicit effort unchanged; never clamp it based on an obsolete adapter or silently substitute a model. Use stronger effort for demonstrated task difficulty, not as a blanket default.
- Native subscription authentication only: no API keys, OpenRouter, or spend/token/request/invocation/quota budgets. If the native subscription/requested model is unavailable, pause model execution and report the exact client, model and error.
- Do not expand Darwin/GEPA/AVO, retrieval tuning, benchmark trains or release research during v1 completion. Required security, exactness, boundedness, lifecycle, federation and minimum release checks remain blockers.
- Use the tracked six-hour prompt at `docs/plans/programme-six-hour-review-prompt.md`. Scheduled delivery queues into the existing conversation; never launch a competing writer/resume. A missed outcome requires a concrete course correction and an evidence-based forecast.
- Update the authoritative gate row and only affected living ADR decisions/status/date in the verified slice; avoid duplicating implementation diaries across ADRs. Refresh source-bound catalogue evidence when affected, without rerunning unrelated historical checks. Commit only scoped changes on `main`; push, tag, deploy or publish only when the current task explicitly authorizes it.

- **Skill syntax**: invoke skills with `$skill-name`. (Claude Code uses `/skill-name`; see `CLAUDE.md`.)
- **Execution model**: `claude-flow` = LEDGER (coordinates memory, routing, swarm state); **Codex = EXECUTOR** (writes code, runs tests, creates files). Coordination commands return instantly, so DON'T STOP after them; continue immediately with the next implementation step.
- Codex config lives in `.agents/config.toml` (project) and `.codex/config.toml` (local overrides, gitignored).

## Links
- Documentation: https://github.com/ruvnet/ruflo
- Issues: https://github.com/ruvnet/ruflo/issues
