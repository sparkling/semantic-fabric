# semantic-fabric

> Rust semantic-data application with Ruflo/MetaHarness development and evidence
> orchestration.
>
> **This file (`AGENTS.md`) is the single CANONICAL, shared instruction source for
> BOTH OpenAI Codex and Claude Code.** Codex reads it directly; Claude Code imports
> it via `@AGENTS.md` at the top of `CLAUDE.md`. Edit SHARED instructions HERE.
> Claude-Code-only guidance lives in `CLAUDE.md` (below its `@AGENTS.md` line).

## Engineering execution policy (2026-09-29 owner amendment)

The owner authorizes ADR-0058 implementation with the same model access and
limits as Semantic Builder. The Claude-only restriction below is historical and
revoked. Native Codex and Claude Code may execute and independently review
implementation, repair and review. Direct implementation, repair, builds and tests
are authorized for application and harness work. Harness orchestration, parallelism
and learning remain optional; harness execution is never a compulsory gate.
Keep one integrator on canonical `main`, source-bound acceptance and independent
review. GCP resumption and harness deployment are now owner-authorized; preserve
its sole coordinator, existing Codex `gpt-6-astra` at `medium`, and current app work.
Ordinary planning, authoring and fresh-context review use native Codex
`gpt-6.1-sol`/`high` through existing 9router; Opus 5.5/high repairs remain.
Explicit task/reviewer pins and native learning remain; no global Claude-only rule.

Direct OpenRouter `deepseek/deepseek-v4.1-flash` authoring and fresh-context
independent review are authorized through an isolated adapter. Enforce at most
$1 per metered request before dispatch and `maxTotalUsd: null`; no cumulative
spending cap. Preserve accounting, unknown-charge stops and replay protection.
API credentials remain only in the API adapter, never check/native environments.
Capability/output failure admits configured Opus/high repair; only confirmed
nonexecuted HTTP 402 credit rejection admits lighter Sonnet or authorized Sol
subscription execution. Preserve actual executor identities and configured
native gateway transport. Native client/model failure pauses only that invocation
with its exact error; independent ready work continues. Inspect transient capacity
errors, then retry unchanged route after backoff, without a blind loop or fallback.
Native subscriptions have no cost, token, request, invocation or quota budgets.
ADR-0058 governs this repair where historical policy below conflicts. No push,
publication or expanded evolution programme follows from harness acceptance.

## Model access and spending (2026-09-26 owner amendment)

Claude Code and Codex access through the configured 9router gateway is authorized.
Direct OpenRouter access is also authorized through `OPENROUTER_API_KEY`.
This amendment supersedes earlier provider-access bans and Claude-only access
restrictions; preserve explicit task assignments and configured model choices.
Keep credentials in the existing user environment/settings, never in Git or logs.

For metered OpenRouter requests, enforce **at most $1 per individual request**.
There is **no task cost cap and no cumulative spending cap**. Do not introduce
dollar budgets for tasks, sessions, projects or programmes. Retain usage accounting,
unknown-charge records and same-request replay protection. Where a runtime accepts
it, the policy is `maxRequestUsd: 1` and `maxTotalUsd: null`; never use zero or a
large finite number to represent an unlimited cumulative allowance.
Use subscriptions through 9router for subscription-covered frontier models.
A direct API worker must check the maximum request cost before dispatch, including
input/output bounds and provider pricing; reject a request that could exceed $1.
Host credentials alone do not implement this guard or an automatic API dispatcher.
Existing native harness adapters remain as configured until an API adapter is
explicitly integrated; do not silently substitute transports.

## Parallel execution (2026-10-01 clarification)

No fixed repository-wide model-session cap. Assign every useful independent
ready packet, with explicit source, ownership, checks and recipient. Separate
model fan-out from CPU/RAM-heavy checks and isolate mutable outputs/resources.
One integration owner accepts reviewed commits as lanes finish, then releases
source-dependent work; unrelated siblings need not drain first.

Historical Claude-only, mandatory-harness and main-only instructions are retired.
Their original evidence remains in Git history and ADR-0055/0058; it is not active
execution policy. Current ordinary roles are Sol 6.1/high and Astra/medium
coordination, with declared capability repair and task-specific pins preserved.

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
- Task branches and isolated Git worktrees are authorized (2026-10-01). One writer owns each lane; one integrator accepts into `main`. Existing non-Git candidates remain valid. Preserve historical evidence and running work; do not switch another writer's checkout.
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
source/data isolation. Main programme resumption is authorized by the September 29
amendment; only explicit current user pauses hold execution.

### Historical delivery policy

Earlier Claude-only, mandatory-harness and no-worktree rules are superseded.
See ADR-0055/0058 and Git history for original evidence. Current direct-work,
parallel isolation and selected-route policies above govern execution.
Explicit user pauses, protected data and publication authority remain separate.

- **Skill syntax**: invoke skills with `$skill-name`. (Claude Code uses `/skill-name`; see `CLAUDE.md`.)
- **Execution model**: `claude-flow` = LEDGER (coordinates memory, routing, swarm state); **Codex = EXECUTOR** (writes code, runs tests, creates files). Coordination commands return instantly, so DON'T STOP after them; continue immediately with the next implementation step.
- Codex config lives in `.agents/config.toml` (project) and `.codex/config.toml` (local overrides, gitignored).

## Links
- Documentation: https://github.com/ruvnet/ruflo
- Issues: https://github.com/ruvnet/ruflo/issues
