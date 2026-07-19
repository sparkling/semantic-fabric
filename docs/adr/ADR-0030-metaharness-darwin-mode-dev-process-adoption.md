---
status: proposed
date: 2026-07-16
tags: [dev-process, tooling, ci-governance, cost-governance, metaharness, darwin-mode, agent-harness-generator]
supersedes: []
depends-on: []
implements: []
---

# Leveraging MetaHarness, Darwin Mode, and the `agent-harness-generator` ecosystem for semantic-fabric's development process

## Context and Problem Statement

`semantic-fabric` is a Rust R2RML/OBDA SPARQL-to-SQL virtualization engine (Ontop
competitor). This ADR is entirely about the **development-process tooling** around
this repo — never the engine's own runtime. It does not touch `sf-mapping`,
`sf-sql`, `sf-serve`, or any Cargo dependency graph.

Separately, `ADR-0026`/`ADR-0027` (accepted) adopted `agentic-qe` as this repo's
quality-engineering fleet — a coverage-gap-finding and test-generation tool, MCP-wired
into `.mcp.json`. **MetaHarness and Darwin Mode are unrelated to that adoption.**
`ADR-0026`'s own "Considered Options" section explicitly rejected `@metaharness/darwin`
for the coverage-gap purpose: *"it evolves the agent's own policy (planner/reviewer/
context-builder), never writes a test or touches product code."* That distinction holds
here too — this ADR is about evolving *how the dev-process harness itself behaves*, not
about testing or fixing the engine.

### What's real and already verified in this session, not hypothetical

- `metaharness@0.3.1` and `@metaharness/darwin@0.8.0` are installed globally via
  mise-managed npm (`$(npm root -g)/@metaharness/darwin`); `ruflo`'s own
  `ruflo-metaharness` plugin exposes the corresponding MCP tools
  (`mcp__ruflo__metaharness_score`, `_genome`, `_oia_audit`, `_evolve`, `_gepa`,
  `_mcp_scan`, `_threat_model`, `_bench`, `_security_bench`, `_similarity`,
  `_audit_list`, `_audit_trend`, `_drift_from_history`, `_learn`).
- `metaharness score .` and `metaharness genome .` already run clean against this
  exact repo: verdict `READY`, `harnessFit: 71`, `estCostPerRunUsd: 0.048`,
  recommended `archetype: rust-crate-harness`, `template: vertical:coding`, recommended
  topology `architect/implementer/reviewer/test-writer`.
- A real dev harness is already scaffolded at this repo's root:
  `semantic-fabric-harness/` (gitignored — `.gitignore:35`, same convention as
  `.metaharness/` on the line above it and `.swarm/`/`.agentic-qe/` elsewhere in this
  repo's history). It was generated via `metaharness semantic-fabric-harness
  --template vertical:coding --host claude-code`. Its own
  `semantic-fabric-harness/CLAUDE.md` documents four agents
  (`architect`=opus, `implementer`=sonnet, `reviewer`=opus, `test-writer`=sonnet), one
  skill (`/plan-change`), one Darwin-facing skill (`/evolve`), and two commands
  (`doctor`, `review-diff`). `npm install` completed (57 packages, 0 vulnerabilities),
  `doctor` reports 4/4 pass (kernel backend `wasm` — expected per `ADR-154` below, not
  a bug), `npm test` 4/4 pass.
- A real, live `mcp__ruflo__metaharness_evolve` run was executed against the raw
  `semantic-fabric` repo directly (not the scaffolded harness — zero-config): 3
  generations × 3 children, deterministic mutator, no `--bench` suite, real sandbox,
  completed in 7.7s. **Every one of the 15 variants tied the baseline exactly
  (`finalScore: 0.235`); nothing was actually promoted.** The tool's own
  `improved: true` flag does not mean quality improved — it most likely just means the
  run completed and returned a valid winner record.
- Reading the installed `@metaharness/darwin@0.8.0` CLI source directly
  (`$(npm root -g)/@metaharness/darwin/dist/cli.js`) confirms and extends the team's
  prior finding: `evolve`'s real, current flag surface (line 121) is
  `--generations N --children N --concurrency N --seed N --bench <suite.json> --tie
  faster --selection quality-diversity|behavioral-diversity|niche-steering|clade|pareto
  --crossover --epistasis --risk-budget N --fdr Q --curriculum --sandbox
  real|mock|agent --mutator deterministic|ruvllm --ruvllm-url URL --ruvllm-model M`.
  `--mutator` accepts only `deterministic|ruvllm` (lines 152–156); the doc comment
  states explicitly: *"the OpenRouter LLM mutator stays library-only."* `--sandbox`
  defaults to `real` (line 150) when the flag is omitted — evolving without
  `--sandbox mock` runs the repo's actual test command by default.
  `ruvector/harnesses/timesfm-harness/scripts/evolve-openrouter.mjs`'s doc comment
  independently confirms the OpenRouter/Requesty mutators are "library-only, not
  exposed by the metaharness-darwin CLI," and documents the real, working pattern to
  add one: resolve the installed `@metaharness/darwin` devDependency, fall back to a
  `DARWIN_DIST=<path>/dist/index.js` env var for local/monorepo runs, and wire the
  custom mutator directly into the imported `evolve` engine. This driver-script
  pattern has **not** been built for this repo — it is a live option, not a done thing.
- `semantic-fabric-harness/package.json` pins `@metaharness/darwin: ^0.2.2` as a
  `devDependency`, while the globally-resolved `metaharness-darwin` binary the
  harness's own `npm run evolve` script invokes is `0.8.0` (mise shim resolution, not
  the pinned local one) — a real version-drift detail worth flagging, not yet a
  problem.
- `semantic-fabric-harness/.claude/skills/evolve/SKILL.md` documents `npm run evolve`
  (`--sandbox real --generations 3 --children 4`, deterministic mutator, no network) and
  `npm run evolve:dry` (`--sandbox mock --generations 2 --children 3`) as the harness's
  own two entry points, plus a set of measured SWE-bench-derived lessons (closed-loop
  repair ≈2× resolve-rate lift, cheap-first routing, output-format-contract-in-system-
  message) worth carrying into how this repo evolves and runs the harness.

### Upstream ADR statuses (do not conflate)

| ADR | Repo | Status | What it says |
|---|---|---|---|
| `ADR-150` (`ruflo/v3/docs/adr/ADR-150-metaharness-integration-surfaces.md`) | `ruflo` | **Implemented** (Phase 1–3§3.1 shipped; Phase 3§3.2-3.5 scoped separately in ADR-151) | The load-bearing architectural constraint: MetaHarness may augment ruflo but must never become a required runtime dependency — removable / optional-dependency-only / graceful-degradation / CI-gated-absent-path |
| `ADR-153` (`ruflo/v3/docs/adr/ADR-153-metaharness-darwin-mode-integration.md`) | `ruflo` | **Proposed** — design intent, not confirmed shipped on paper | Darwin Mode as ruflo's harness-*evolution* (write) layer, behind the same four ADR-150 constraints; explicitly states *"DO NOT auto-evolve ruflo itself in CI... Darwin Mode is a human-initiated operation on a harness, not a continuous background optimization"* |
| `ADR-154` (`ruflo/v3/docs/adr/ADR-154-metaharness-kernel-platform-binaries.md`) | `ruflo` | **Accepted** (WASM-only path is functional; native fast-path tracked upstream, never published) | `@metaharness/kernel`'s five per-platform NAPI-RS native binaries are not published to npm; `loadKernel()` falls back to WASM cleanly and this is the accepted, supported baseline, not a defect |

Despite `ADR-153`'s own Proposed paper status, the actual installed
`mcp__ruflo__metaharness_evolve` tool was verified live this session to be genuinely
wired (`degraded: false`, `exitCode: 0`, a real dry-run plan, and the real completed
7.7s run above) — the shipped capability is ahead of the ADR's own paper status. This
ADR does not underclaim what already works, nor overclaim what `ADR-153` itself still
marks as design intent (e.g. Phase 2's witness-signed archives, Phase 3's federated
lineage, are not built).

### The problem

Given all of the above is real (not a menu of hypothetical future capabilities) and one
real but flat Darwin trial already ran, this ADR decides: when the harness gets used,
how to get a Darwin signal that means something, what cost/safety guardrails apply given
this is a large multi-crate Rust workspace with live-DB integration tests, and which of
the many other MetaHarness surfaces are worth adopting now versus deferring.

## Decision Drivers

* **`ADR-150`'s removability discipline is non-negotiable and extends to this repo's
  own posture**, even though nothing here proposes wiring Darwin output into product
  code. `rupixel/rust/pixelrag-core/src/config.rs` is real Rust prior art for the
  pattern to mirror *if* that ever changed: a hard-coded default config the binary is
  fully usable with alone, an optional darwin-evolved genome loaded read-only, clean
  fallback on any load failure.
* **The measured flat-tie result is real signal, not noise to explain away.** All 15
  variants tying the baseline exactly is close to the expected outcome of the current
  setup, not a fluke: Darwin's seven mutation surfaces (`planner`, `contextBuilder`,
  `reviewer`, `retryPolicy`, `toolPolicy`, `memoryPolicy`, `scorePolicy`) are
  dev-process *policy* files, and the default scoring path (no `--bench`) grades a
  variant by running the repo's own test command. A mutation to *how the agent plans or
  reviews* has no mechanical reason to change whether `cargo test` passes — so ties are
  close to guaranteed by construction until the scorer is pointed at something the
  mutated surfaces can actually move.
* **Cost/safety**: `--sandbox` defaults to `real` (confirmed directly in the installed
  CLI source above) — an `evolve` invocation without explicitly passing `--sandbox mock`
  runs real `cargo build`/`cargo test` cycles per variant against a large multi-crate
  Rust workspace that also runs live Postgres/MySQL/SQL Server integration tests in CI
  (`ADR-0026`). This is materially more expensive and more DB-container-contentious
  than a typical harness's toy fixture repo — a guardrail is needed, not assumed.
* Decide concretely which of the many available surfaces (`bench`/`gepa`/`mcp_scan`/
  `threat_model`/`similarity`/`audit_trend`/`oia_audit`) are worth adopting now versus
  explicitly deferred, rather than leaving an open-ended menu.
* `ADR-153` itself already states the CI posture this repo should inherit verbatim:
  Darwin Mode is human-initiated, never an automatic/scheduled CI action.

## Considered Options

* **(a) Ignore MetaHarness/Darwin entirely; keep using ad-hoc Claude Code sessions
  only.** Rejected: `score`/`genome` already show measurable fit (`harnessFit: 71`,
  verdict `READY`) at effectively zero cost to check, and the `ADR-0026` precedent
  (adopting `agentic-qe` over hand-rolling) shows this project is willing to adopt a
  maintained external tool once it demonstrates real, checkable fit — the same bar
  MetaHarness's `score`/`genome` output already clears for the read-only surfaces.
* **(b) Adopt `semantic-fabric-harness/` as a wholesale replacement for ad-hoc Claude
  Code sessions.** Rejected: the harness's fixed four-role pod
  (architect/implementer/reviewer/test-writer) is narrower than this repo's existing
  swarm conventions (`AGENTS.md`'s hierarchical/specialized topology, up to 8 agents,
  custom agent types per task). Replacing all sessions with the narrower pod would be a
  process regression with no measured benefit backing it — and the flat Darwin result
  shows that zero-curated-benchmark evolution provides no measured harness-quality
  improvement today, so there is no evidence yet that the harness's evolved state is
  better than an ad-hoc session for a given task.
* **(c) Use the harness selectively, gated on concrete trigger conditions; build a real
  `--bench` suite before trusting any Darwin signal; adopt specific low-cost read-only
  surfaces now and defer the rest with a stated reason.** Chosen.

## Decision Outcome

Adopt option (c). The five sub-decisions below are what this ADR actually commits to.

### 1. When and how `semantic-fabric-harness/` gets used

**Not a blanket replacement for Claude Code sessions.** Most of this repo's day-to-day
iteration already has an established, more flexible convention (`AGENTS.md`'s
hierarchical/specialized swarm topology, any string as a custom agent type). The
harness's fixed four-role pod is a narrower shape that should be reached for when that
narrower shape actually fits — not by default.

**Trigger conditions — use `semantic-fabric-harness/` when:**

1. The task is scoped to a single crate or small crate-set change with a clear,
   already-passing test surface to evolve/score against (matches the recommended
   `rust-crate-harness` archetype / `vertical:coding` template `metaharness genome`
   already returned for this repo).
2. You specifically want the harness's `review-diff` command's correctness/security/
   reuse review as a second, harness-native check layered on top of (not instead of)
   this repo's own `/code-review` skill.
3. You want the four-role pipeline's explicit architect-before-code / test-writer-after-
   code discipline for a change that benefits from that separation (e.g. a new backend
   dialect, a mapping-compiler change) rather than a quick one-file fix.

Run `doctor` (`.claude/commands/doctor.md`) at the start of any session that uses the
harness — it checks kernel load, MCP wiring, and memory backend health before relying on
it for anything.

### 2. Getting a useful Darwin signal — recommended order of operations

1. **Build a `--bench` suite first**, via `metaharness_bench` / `bench create <repo>`
   (`ruflo/plugins/ruflo-metaharness/scripts/bench.mjs`: *"lets you evolve a harness
   against a fixed evaluation set independent of the repo's natural tests"*). Seed it
   from real past tasks with a clear, already-known-correct outcome this repo has
   already done by hand — e.g. `ADR-0026`'s coverage-gap-closing pass (did the harness
   find/fix the SQL Server `date_from_proleptic()` Rata-Die bug) or a similarly-scoped
   past bug fix.

   > **CORRECTION (§7 adversarial review, 2026-07-16, same day) — this step's own
   > premise is false at darwin@0.8.0.** The sentence originally here claimed a
   > bench-scored variant "can actually differentiate this planner/reviewer policy
   > finds real bugs faster in a way `cargo test` alone structurally cannot." Reading
   > darwin's actual bench-runner source falsified that: `bench/runner.js` is a
   > prototype whose own comments say `"variant never patches repo files"` (line 42)
   > and `"every variant runs the identical task command"` (line 45), and
   > `bench/score.js`'s `scoreBenchmark` takes only the three test-pass booleans +
   > safety + cost as input — none of which vary between variants when the repo is
   > never patched and the command is identical. So **every variant scores identically
   > on any bench task**; a bench suite does not differentiate policies at this tool
   > version. The real unblock is an actual agent-execution path (a real LLM mutator
   > driving an agent that patches repo code, plus the LLM-evaluator scoring the
   > runner's own comments say "returns" later) — NOT a bench suite alone. Build the
   > bench suite as scaffolding for when that lands; do not expect a signal from it
   > now. See §7.
2. **Until a bench suite exists, do not cite `evolve`'s `improved: true` flag, or any
   single `finalScore`, as evidence the harness got better.** The verified flat-tie
   result demonstrates this directly, not hypothetically.
3. **Mutator choice comes after the bench suite, not before.** The default
   deterministic mutator (no network, no key, air-gapped, per
   `semantic-fabric-harness/.claude/skills/evolve/SKILL.md`) is the correct default for
   cheap iteration once a bench suite exists. `--mutator ruvllm` needs a local `ruvllm
   serve` endpoint this repo does not currently run — lower priority, revisit if a local
   endpoint becomes available for other reasons. A new OpenAI-backed mutator (mirroring
   `evolve-openrouter.mjs`'s real pattern: resolve the installed `@metaharness/darwin`
   devDependency or fall back to `DARWIN_DIST`, wire a custom mutator into the imported
   `evolve` engine directly) is real and buildable, but a smarter mutator without a
   differentiating scorer still just produces more sophisticated ties — build the bench
   suite first.

### 3. Cost governance

* **`--sandbox mock` is the default posture for iteration and any exploratory
  `evolve` run** — matches the harness's own `npm run evolve:dry`. `--sandbox real`
  (which is what bare `evolve`/`npm run evolve` actually does, per the CLI default
  confirmed above) is reserved for a deliberate, human-confirmed run once a bench suite
  exists to make the run's cost worth paying.
* **`evolve --sandbox real` must never run in CI or on any schedule.** This mirrors
  `ADR-153`'s own explicit clause verbatim (*"DO NOT auto-evolve ruflo itself in
  CI... Darwin Mode is a human-initiated operation on a harness"*) — the same posture
  applies here with more force, since this workspace's CI already spins up three live DB
  service containers (`ADR-0026`) that a concurrent real-sandbox `cargo test` run would
  contend with.
* **Concurrency**: the harness's own defaults (`--generations 3 --children 4`,
  `--concurrency 4`) are conservative starting points inherited from
  `semantic-fabric-harness/package.json`'s `evolve` script; when running with
  `--sandbox real` against this workspace specifically, cap `--concurrency` to 1–2 and
  avoid running it while CI is actively exercising the live-DB integration suites —
  unlike a small toy fixture repo, `cargo test` here is not cheap, and each concurrent
  variant is a full build+test cycle.

### 4. Where related components fit — adopt now vs. defer

| Surface | Decision | Why |
|---|---|---|
| `metaharness score` / `genome` | **Adopt now** | Already run clean against this repo; read-only, no code execution, cheap; run periodically (e.g. before a release, alongside `doctor`) as an added health signal, no new gating behavior |
| `mcp-scan` / `threat-model` | **Adopted, but currently blind — do not trust the verdict** | Actually run 2026-07-16 against this repo's real `.mcp.json` (`agentic-qe`, `ruv-swarm`, etc.). Both tools expect `.mcp/servers.json`, not Claude Code's actual `.mcp.json` convention: `mcp-scan` returned `mcpEnabled: false, "No MCP surface"` outright; `threat-model` was internally inconsistent — `mcpInUse: true` with real `allowedTools: 4`/`deniedTools: 2` counts (likely read from `.claude/settings.json`), yet its own findings still said `"No MCP surface"` and returned `verdict: clean`. **`clean` here means "found nothing to scan," not "scanned and found no issues."** Until this repo's `.mcp.json` is either converted to `.mcp/servers.json` or the installed `metaharness` CLI adds `.mcp.json` support, do not cite either tool's verdict as a real security read of this repo's actual MCP surface |
| `metaharness_bench` | **Started 2026-07-16 — but see the §7 adversarial-review correction: a bench suite alone does NOT unblock a meaningful signal at darwin@0.8.0** | `.metaharness/bench.json` created via `--op create`, then hand-corrected: the auto-scaffolded smoke task defaulted to `npm test`/`package-lock.json`, wrong for this pure-Cargo workspace (confirmed: no root `package.json` at all) — fixed to the real `cargo build && cargo test --workspace`. Added `task-0002`, seeded from the `ADR-0026` SQL Server date bug — a **regression-guard** task, test command (`cargo test -p sf-sql --lib --features sqlserver-backend -- <4 test names>`) run directly and confirmed passing before being written in, not assumed. It is a real, valid regression artifact. **What §7's adversarial review then overturned**: this table row and §2 originally called the bench suite the "first-priority unblock" for a meaningful Darwin signal. That is wrong at this tool version — darwin@0.8.0's bench runner is a prototype that, in its own source comments (`bench/runner.js:42` `"variant never patches repo files"`, `:45` `"every variant runs the identical task command"`), never lets a variant modify repo code, so every variant scores identically on any bench task. A bench suite is necessary scaffolding for a future capability, not a present unblock. Also found: `--op verify` reports `degraded: metaharness-darwin-not-available` (a local Node module-resolution check that fails in a `package.json`-less Rust repo, even though `--op create` and `evolve` used the same global `@metaharness/darwin@0.8.0` fine) — validated the suite manually instead |
| `oia-audit` / `audit-trend` / `drift-from-history` / `similarity` | **Defer** | These compound over multiple historical snapshots (drift-from-history needs a prior baseline audit to diff against); this repo has run `score`/`genome` exactly once so far — revisit once a few sessions have accumulated audit history worth diffing |
| `gepa` | **Defer, with cause** | `gepaOptimize` requires an in-process `evaluate(candidate)` callback that cannot cross a subprocess/MCP boundary — confirmed directly: `ruflo/plugins/ruflo-metaharness/scripts/gepa.mjs`'s own doc comment states this is why the plugin does **not** surface `gepaOptimize`, only the read-only `genome`/`validate`/`render`/`analyze` subset. Real optimization runs need either a direct library import (`@metaharness/darwin/gepa`) or the darwin CLI's `evolve`, which already covers the same ground here once a bench suite exists |
| `security-bench` (Darwin Shield, upstream `ADR-155`) | **Defer, out of scope** | A distinct track (evolves a *security-detection* harness against a vuln/decoy corpus); unrelated to this repo's own dev-process harness, and this repo has no such corpus today |
| `@metaharness/router` / `@metaharness/kernel` | **No action needed** | Internal `ruflo`/MCP plumbing this repo consumes only indirectly through the MCP tool layer; `ADR-154`'s accepted WASM-only baseline is already what this repo's own `doctor` reports (kernel backend `wasm`, 4/4 pass) — nothing for this repo to decide here beyond what `doctor` already covers |

### 5. Explicit scope boundary

This ADR covers **only** the dev-process harness and its adjacent tooling. It does
**not** propose, and must not be read as authorizing:

* `semantic-fabric`'s engine/runtime crates (`sf-mapping`, `sf-sql`, `sf-serve`, etc.)
  consuming any Darwin-evolved configuration or MetaHarness-generated artifact at
  runtime, ever.
* Any change to this repo's product `Cargo.toml` or Rust dependency graph.
* Automatic or CI-scheduled `evolve` runs of any kind.

If a genuine future need arose for the engine itself to consume something
Darwin-evolved (unlikely — Darwin targets agent-harness policy surfaces, not SQL
engines), it would need its own ADR mirroring `rupixel/pixelrag-core/src/config.rs`'s
pattern: a hard-coded default the binary works from alone, an optional read-only
genome load, and a clean fallback on any failure — the same removability discipline
`ADR-150` establishes upstream.

### 6. Update (2026-07-16, same day) — the agent pod's own MCP server was found broken, then fixed

After this ADR was written, further verification (prompted by a direct challenge,
re-checked against a second, independently-generated harness —
`symbolic-scribe/harness/package.json` — for real corroboration, not just re-reading
the same file) found that `semantic-fabric-harness`'s four-agent pod
(`architect`/`implementer`/`reviewer`/`test-writer`) was **not actually invocable at
all**: `bin/cli.js` implemented only `init`/`doctor`/`--version`/`--help`, no `mcp`
case, despite `.claude/settings.json`'s `mcpServers.semantic-fabric-harness` entry
expecting `npx semantic-fabric-harness@latest mcp start` to serve one. No
`.claude/agents/*.md` existed either — the four roles were only `SYSTEM_PROMPT`
exports in `src/agents/*.ts`, wired to nothing. Confirmed this is a real,
reproducible gap in what this generator template currently emits (the second harness
showed the identical shape), not something specific to how this instance was
scaffolded.

**Fixed, same day, verified end-to-end — not just claimed:**

* `src/mcp-server.ts` — a real MCP server (Anthropic's official
  `@modelcontextprotocol/sdk@^1.29.0`, added as a real dependency; no RuvNet tool
  does protocol-level MCP serving — `@metaharness/kernel`'s own `mcpValidate()`
  only validates a launch spec, confirmed by reading its actual `.d.ts`, it does not
  run one). Registers one tool per agent; each tool takes a `task` string and
  returns that agent's system prompt + the task, framed for the calling model to
  adopt — these are prompt-only roles with no independent model access of their
  own, not sub-agents that make their own LLM calls.
* Compiled via the existing `"build": "tsc"` script (NodeNext module resolution,
  confirmed via `tsconfig.json` before writing the `.js`-suffixed relative
  imports); wired into `bin/cli.js`'s dispatch as `case 'mcp':` → `mcp(argv[1])`,
  which dynamically imports `dist/mcp-server.js` and gives a clear
  "run `npm run build` first" error (not a raw stack trace) if dist/ is missing.
* **A real correctness bug caught before landing**: `server.connect(transport)`
  resolves as soon as the listener attaches, not when the session ends — the
  original `bin/cli.js` guard (`run(...).then(code => process.exit(code))`) would
  have killed the process immediately after connecting, before it ever served a
  request. Fixed by blocking `start()` on `transport.onclose` so the process stays
  alive for the real session lifetime.
* Added `"prepublishOnly": "npm run build && npm test"` so `dist/` can never go
  stale before a real `npm publish` — the same class of gap that caused this bug
  in the first place (package.json's own `files` array already expected
  `dist/**` to exist and be current).
* **Verified, not assumed working**: a new end-to-end test
  (`__tests__/mcp-server.test.ts`) spawns the actual `bin/cli.js mcp start`
  subprocess and speaks real MCP protocol to it via the SDK's own client —
  `tools/list` returns exactly the 4 expected tools, `tools/call` on `architect`
  returns its real system prompt text plus the task. 6/6 tests pass
  (2 new + the 4 existing smoke tests, no regression). `doctor` still 4/4.

**Still open, not fixed, distinct gap**: `.claude/settings.json`'s second
`mcpServers.code_index` entry (`npx semantic-fabric-harness@latest mcp index`) has
no implementation at all — a different feature (code indexing), out of scope for
this pass, which only fixed `mcp start`.

### 7. Adversarial review (2026-07-16, same day) — what survived, what was overturned

This ADR's own decisions were adversarially re-verified against real source and by
empirical test, not re-asserted. Three claims examined:

* **`mcp-scan`/`threat-model` "blind" (§4) — VALIDATED and sharpened.** Read the real
  plugin source (`ruflo-metaharness/0.1.0/scripts/mcp-scan.mjs`): it shells out to
  `harness mcp-scan <path>`, which by design reads `.mcp/servers.json` +
  `.harness/claims.json`. Verified directly that **neither the repo root nor the
  generated `semantic-fabric-harness/` has that file** — the `claude-code` host emits
  `.claude/settings.json` (with an `mcpServers` block), not `.mcp/servers.json`, so
  the scanners can't see either surface. Confirmed the `threat-model` inconsistency's
  exact cause: it reads `.claude/settings.json` for permission counts (repo root's
  settings has exactly `allow: 4`/`deny: 2`, matching the tool's `allowedTools: 4`/
  `deniedTools: 2`) but reads findings from the absent `.mcp/servers.json`, so it
  returns `verdict: clean` while **missing a real broad `mcp__claude-flow__:*` wildcard
  grant** present in those very settings. The "adopt now / clean" reading was wrong;
  the corrected §4 row stands.
* **The `mcp start` `onclose` fix (§6) — VALIDATED empirically.** Reverted `start()` to
  the naive `await server.connect(transport)` (no `onclose` block), rebuilt, ran the
  suite: **both MCP tests failed with `MCP error -32000: Connection closed`** — the
  exact predicted failure (subprocess exits right after connect, before serving).
  Restored the fix → 6/6 green again. The fix is genuinely load-bearing and the test
  genuinely proves it; the §6 "correctness bug caught before landing" claim holds.
* **The `--bench` suite as "first-priority unblock" (§2, §4) — OVERTURNED.** The bench
  *artifact* (`task-0002`) is real and verified. But the *claim that building it
  unblocks a meaningful Darwin signal* is false at darwin@0.8.0, proven from the tool's
  own source: `bench/runner.js` is a prototype (`"variant never patches repo files"`,
  `"every variant runs the identical task command"`), and `bench/score.js`'s only
  variant-sensitive inputs are test-pass booleans that are therefore identical across
  all variants → structural ties on any bench task, exactly like the original
  zero-config flat-tie. §2 and the §4 row are corrected in place. The honest state:
  a meaningful Darwin signal needs a real agent-execution mutator + LLM evaluator, not
  a bench suite alone — the bench suite is scaffolding for that future, not a present
  unblock.

Net: two of three findings survived intact (one sharpened); the third — my own
"bench-suite-first" recommendation — was falsified by reading the source I had only
cited secondhand before, and is now corrected. The discipline that caught it is the
same one this whole ADR line runs on: verify against real source, overturn your own
claim when it doesn't hold.

### 8. Correction (2026-07-18) — the `agent` sandbox produces the signal §2/§7 declared impossible

§2, §4, §7, and **R3** all concluded that darwin@0.8.0 cannot produce a meaningful
signal on this harness — every variant ties the baseline. **That conclusion is an
artifact of the sandbox mode tested, not the tool, and is now overturned by empirical
test.** §7 reasoned only about `--sandbox real` (score = repo's `cargo test`, which a
policy mutation cannot move → ties by construction) and `--sandbox bench` (runner
prototype never patches repo files → ties). It never ran the **third mode shipped in the
installed 0.8.0**: `--sandbox agent` = the Tier-2 agent-executing sandbox (upstream
`agent-harness-generator/docs/adrs/ADR-106`, **Accepted/implemented**). That mode runs
each variant's *actual* `planner`/`contextBuilder`/`retryPolicy`/`toolPolicy` code in a
child process driving an agent loop, and scores by outcome — precisely the
variant-sensitive input §7 said did not exist.

Verified present in the installed binary: `dist/tier2-driver.js` + `dist/tier2-sandbox.js`,
and `dist/cli.js` parses `sandboxMode = sbRaw === 'mock' || sbRaw === 'agent' ? sbRaw :
'real'`. Empirical run 2026-07-18 (`evolve . --sandbox agent --generations 3 --children 3
--seed 42`, node v24):

```
0.802  g2_v5  [contextBuilder]  pass=0.67  ◀ winner
0.618  baseline [planner]       pass=0.33
Delta over baseline: +0.183     lineage: baseline → g1_v0 → g2_v5
```

Variants genuinely differentiate; the winner evolved the `contextBuilder` surface and
doubled the pass rate — matching ADR-106's own measured behaviour (wider context window
solves strictly more tasks).

**Honest scope (do not over-correct into overselling):** ADR-106's `agent` sandbox scores
against a *synthetic* deterministic file-location task, **not** semantic-fabric code. So
the signal is real, reproducible, and safe — but it optimizes the pod's policy for a
generic coding-competence proxy, not for building this repo specifically. Making the
evolution semantic-fabric-specific is the upstream **ADR-098** real-LLM-on-real-repo
substrate, or authoring a small set of repo-representative agent tasks. The bench-suite
pessimism of §2/§7 stands *for the bench path*; it does not generalize to the `agent`
path, which §7 simply never tested.

**What this changes:** the "no meaningful signal is reachable today" framing (§2 step 2,
§7 third finding, R3) is wrong. A meaningful, measured, promoted signal is reachable now
via `--sandbox agent` — on a proxy task. R3 is updated below accordingly.

### 9. LLM-mutator experiment (2026-07-19) — ceiling NOT broken; two findings

The §8 hypothesis chain ends here, measured: the deterministic mutator saturates at
0.802 (5 rounds: 3 and 8 generations, default and behavioral-diversity+crossover
selections), and the **OpenRouter LLM mutator (gemini-2.5-flash, library-only wiring
per the timesfm-harness driver pattern) scored 0.618 flat — 45/45 children tied to
baseline, zero promotions, $0.0519 total** (42k tokens, receipt real). Root cause is an
**information gap, not a capability gap**: the mutator prompt carries only the current
file + a bare parent score — never the task suite — so the model produces safe cosmetic
refactors under the no-new-capabilities gate, while `DeterministicMutator`'s EDIT_RULES
are co-designed with `DEFAULT_AGENT_TASKS`' literals (it finds the lever by
construction, not search). At darwin@0.8.0 on this synthetic suite, **neither more
generations nor a smarter mutator moves the ceiling; only task-aware mutation would.**

Second finding, upstream-reportable with byte-diff proof: the standing "0.802 via
`g1_v1 [retryPolicy]`" archive label is **mislabeled** — on disk, that variant's
`retry_policy.ts` is byte-identical to baseline and the real diff is
`context_builder.ts` `.slice(0, 30)` → `.slice(0, 40)` (the same window-widen lever
ADR-106 itself published). Mechanism: variant ids carry no run nonce, the archive is
idempotent-on-id (first label wins), and re-runs with a different `--seed` force-
overwrite variant files at the same coordinates — so any repeated CLI run against the
shared `.metaharness/` workRoot silently corrupts old labels. Recorded in
`semantic-fabric-harness/NOTES.md` with artifacts under `.metaharness/work-openrouter/`.

## Consequences

### What gets easier

* A standing, already-verified-working set of read-only health checks (`score`,
  `genome`, `mcp-scan`) this repo can reach for again without re-installing or
  re-deriving anything — same category of win `ADR-0026` already banked for
  `agentic-qe`'s coverage tooling.
* A documented, concrete trigger condition for when the four-role harness pod is worth
  reaching for, instead of an ambiguous "try it and see."
* A clear, falsifiable path to a Darwin signal that would actually mean something
  (bench-suite-first), instead of continuing to run zero-config `evolve` and
  misreading ties as neutral or inconclusive.

### What gets harder / costs incurred

* Building a real `--bench` suite is real, unscoped work — this ADR identifies the
  prerequisite and candidate seed tasks (the `ADR-0026` coverage-gap pass, the SQL
  Server date bug) but does not build the suite itself.
* `--sandbox real` runs against this workspace are genuinely expensive (full
  `cargo build`/`cargo test` per variant) compared to a typical harness's toy fixture
  repo; the concurrency-cap guardrail (#3) trades iteration speed for not contending
  with CI's live-DB containers.
* An OpenAI-backed mutator driver script, if built later, is new maintenance surface
  (a small standalone script per the `evolve-openrouter.mjs` pattern) — not built here,
  explicitly deferred until after the bench suite exists.

### Neutral

* **Fixed same day.** `semantic-fabric-harness/package.json`'s pinned
  `@metaharness/darwin` devDependency was `^0.2.2`, drifted from the `0.8.0`
  binary actually resolved at the `npm run evolve` call site (mise shim). Updated
  the pin to `^0.8.0` (confirmed `0.8.0` is npm's actual current published
  version via `npm view @metaharness/darwin version`, not guessed), re-ran
  `npm install` (clean), `doctor` (4/4 pass), `npm test` (4/4 pass). No longer
  drift — this note is now historical.
* `ADR-153`'s Proposed paper status does not block this ADR's recommendations — the
  actual installed `mcp__ruflo__metaharness_evolve` tool is genuinely wired and
  functions as described, independent of the upstream ADR's own status label.

## More Information

* Upstream sources cited directly, with repo/path (per this session's `search_ruvnet`
  grounding — see ground rules): `ruflo/v3/docs/adr/ADR-150-metaharness-integration-
  surfaces.md` (Implemented), `ruflo/v3/docs/adr/ADR-153-metaharness-darwin-mode-
  integration.md` (Proposed), `ruflo/v3/docs/adr/ADR-154-metaharness-kernel-platform-
  binaries.md` (Accepted), `ruflo/plugins/ruflo-metaharness/scripts/{evolve,gepa,bench,
  test-graceful-degradation}.mjs`, `ruflo/plugins/ruflo-metaharness/.claude-plugin/
  plugin.json`, `ruflo/docs/metaharness-user-guide.md`,
  `ruvector/harnesses/timesfm-harness/scripts/evolve-openrouter.mjs`,
  `rupixel/rust/pixelrag-core/src/config.rs`.
* Directly-read local evidence: `$(npm root -g)/@metaharness/darwin/dist/cli.js`
  (verified `--mutator deterministic|ruvllm` and `--sandbox` default `real`, this
  session), `semantic-fabric-harness/package.json`, `semantic-fabric-harness/CLAUDE.md`,
  `semantic-fabric-harness/.claude/skills/evolve/SKILL.md`, `.gitignore:34-35`.
* Related, not superseded: `ADR-0026`/`ADR-0027` (the `agentic-qe` QE-fleet adoption —
  a separate, unrelated tool this ADR does not duplicate or conflict with).
* Explicitly out of scope: any change to `semantic-fabric`'s own engine/runtime crates
  or Cargo dependency graph (see Decision Outcome §5).

## Rules

* **R1** — No `sf-*` runtime crate may read, import, or otherwise depend on any file
  under `.metaharness/` or any Darwin-evolved artifact, ever. This ADR is dev-process
  only; runtime consumption of Darwin output would need its own ADR (see §5).
* **R2** — `evolve --sandbox real` (or bare `evolve`/`npm run evolve`, which defaults
  to `real`) must never be invoked from CI or any scheduled/cron process. Human-
  confirmed, interactive runs only.
* **R3** (updated 2026-07-18, see §8) — No `evolve` run in `--sandbox real` or
  `--sandbox bench` mode may be cited as evidence of harness-quality improvement: those
  modes tie by construction on this repo. A **`--sandbox agent`** run's `finalScore`
  delta *is* a valid, measured signal (Tier-2 executes the variant's real surface code),
  but it is scored against a synthetic proxy task, not semantic-fabric — cite it as
  "improved the pod on a generic coding proxy," never as "improved at building
  semantic-fabric," until repo-representative tasks or the ADR-098 real-repo substrate
  back it.
