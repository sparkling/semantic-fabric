# semantic-fabric

> Rust semantic-data application with Ruflo/MetaHarness development and evidence
> orchestration.
>
> **This file (`AGENTS.md`) is the single CANONICAL, shared instruction source for
> BOTH OpenAI Codex and Claude Code.** Codex reads it directly; Claude Code imports
> it via `@AGENTS.md` at the top of `CLAUDE.md`. Edit SHARED instructions HERE.
> Claude-Code-only guidance lives in `CLAUDE.md` (below its `@AGENTS.md` line).

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

**ruflo-interface-contract:v1**

## Ruflo Interface Contract

- Use `search_ruvnet` for RuvNet source and capability claims when the Brain is installed; cite its source.
- Use `guidance_brain` / `guidance_recommend` and the live MCP registry for this process's actual registered, configured, reachable, healthy, and authorized state.
- Prefer a live structured Ruflo MCP tool for coordination, memory, routing, learning, and status. Discover deferred tools and schemas; never guess names or arguments.
- For a genuine CLI-only gap, use `ruvnet_cli_help`, then `ruvnet_cli_run` with literal `argv` when that bridge is registered. Exact requested help must authorize the run; parent help or exit code alone is insufficient.
- Direct shell is for bootstrap and administration that cannot depend on MCP: install/init, first MCP registration/start, diagnostics, and deliberate daemon work.
- Native Claude/Codex agents execute. Ruflo tracks a swarm only after `swarm_init` and `agent_spawn` create records; a native agent alone is not proof.
- Before generic testing or security agents, discover specialized installed QE or adversarial-security capabilities and disclose any fallback.

**ruflo-managed:swarm:v2**

## Swarm & Coordination

Use the smallest capable structure derived from dependency edges, shared-state risk, and required evidence instead of a file count.

- Independent one-shot native agents need no Ruflo swarm.
- For persistent topology, shared memory, or tracked handoffs, discover the live schemas, call `swarm_init`, then register each worker with `agent_spawn({agentType: "...", agentId: "..."})`.
- A tracked record does not launch a native Claude/Codex agent; launch the matching executor separately.
- Give every writer an isolated worktree and non-overlapping ownership; name one integration owner.
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

- **Skill syntax**: invoke skills with `$skill-name`. (Claude Code uses `/skill-name`; see `CLAUDE.md`.)
- **Execution model**: `claude-flow` = LEDGER (coordinates memory, routing, swarm state); **Codex = EXECUTOR** (writes code, runs tests, creates files). Coordination commands return instantly, so DON'T STOP after them; continue immediately with the next implementation step.
- Codex config lives in `.agents/config.toml` (project) and `.codex/config.toml` (local overrides, gitignored).

## Links
- Documentation: https://github.com/ruvnet/ruflo
- Issues: https://github.com/ruvnet/ruflo/issues
