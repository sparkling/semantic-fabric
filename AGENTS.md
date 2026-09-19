# semantic-fabric

> Rust semantic-data application with Ruflo/MetaHarness development and evidence
> orchestration.
>
> **This file (`AGENTS.md`) is the single CANONICAL, shared instruction source for
> BOTH OpenAI Codex and Claude Code.** Codex reads it directly; Claude Code imports
> it via `@AGENTS.md` at the top of `CLAUDE.md`. Edit SHARED instructions HERE.
> Claude-Code-only guidance lives in `CLAUDE.md` (below its `@AGENTS.md` line).

## Independent Codex session capacity (2026-09-19 user amendment)

Do not impose a repository-wide three- or four-session cap on independent
`codex exec` processes using the ChatGPT subscription. Four distinct concurrent
sessions completed successfully on Codex 0.155.1. This does not establish an
infinite capacity or override a native client's per-session subagent limits.
Select parallel work from ready dependencies and file ownership; preserve the
single integration writer and existing build/resource isolation. Historical
in-session capacity observations are not a global model-session limit.

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

### Delivery execution (ADR-0055, updated 2026-09-11)

- Every building task must use the main-only delivery harness in `coding-harness/` (ADR-0055, user corrections 2026-09-10 and 2026-09-19). Begin a scoped task before editing, bind the actual native executor/model/effort, then use `advance`/`submit` for source-bound implementation, declared checks, failure-directed repair and independent read-only review on the other native provider. The existing native host executes each returned request; do not start another implementation host. Verify, commit only that slice, then finish against the exact commit. See `coding-harness/README.md` for response schemas, recovery and trust boundaries. Ruflo MCP records coordination/results; local receipts do not prove remote synchronization. Manifests use `latest`, with exact tested dependency resolution retained in the lockfile. The historical closed candidate experiment remains optional and its worktree launchers remain prohibited.
- After an explicit user pause/review request, do not resume application tasks from the scheduler or an active goal until the user releases that pause. Preserve interrupted patches; explicit adoption records starting changes without treating them as verified.
- Finish integration and public request behavior before starting another foundation. Use the finite G1–G6 ledger in `docs/plans/sota-application-completion-programme.md`, not a next-unmetered-helper queue. Audit prerequisite test callsites before freezing task scope; group compatible work by complete phase/public outcome, retaining incremental verified commits. Count reduced named obligations and public acceptance, not commits, tests, scores, reviews or receipts alone.
- Preserve the selected main model. Use tools for deterministic work, Luna low/Haiku for bounded mechanical tasks, Terra medium for established patterns, and Sol medium/Sonnet for normal implementation/review (Sol high for a specific correctness proof). The selected Codex or Claude host writes and the other provider reviews; an explicit reviewer route remains cross-provider. Escalate to Astra high/Opus only for a named unresolved problem; max/Fable needs a bounded exceptional task. Ultra requires a specific user request, not routine follow-up. Route each new task afresh and stop stronger reviewers once their question is answered.
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
