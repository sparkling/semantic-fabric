---
status: accepted
date: 2026-07-05
tags: [meta-harness, darwin, gepa, claude-only, dev-loop, recipe, implementation]
supersedes:
  - ADR-0025
depends-on:
  - ADR-0005
  - ADR-0007
  - ADR-0012
implements: []
---

# Meta-harness and Darwin Mode: implementation recipe

> Development tooling only — never on the fabric's runtime/serving path. All model calls are Claude; no other LLM provider may appear anywhere in this system.

## Context and Problem Statement

semantic-fabric is developed by a Claude coding agent whose working policy (planning style, test-first discipline, context budget, retry behaviour, self-review) is currently fixed prose in `CLAUDE.md` and agent prompts. We want that policy to improve empirically: propose variations, measure each against the repo's real quality gates, and keep only measured wins. Two subsystems deliver this:

1. **Meta-harness read layer** — readiness/drift telemetry for the repo-as-agent-workspace.
2. **Darwin loop** — evolution of the agent policy itself, implemented with the GEPA engine (`@metaharness/darwin`'s `gepa` module used as a library: prompt-text genomes, caller-supplied evaluator and mutation-proposer). The package's `evolve` CLI is not used: its real-repo evaluation executes every variant in the shared repo root (`dist/sandbox.js`, `cwd: profile.root`), so it cannot distinguish variants; its discriminating modes only score a built-in synthetic task suite.

## Decision Drivers

* Claude-only, structurally: the only LLM touchpoints are functions we write.
* Real fitness only: the repo's own test, conformance, and bench gates; nothing gameable. A fitness definition may never reward a correctness regression (`=_bag` rail, ADR-0007).
* Reproducible: exact pins, hash-pinned corpus, persisted lineage.
* Bounded spend: sweeps are budgeted and human-launched.
* Fail honestly: if a gate cannot be passed, stop and report — never proceed on an unproven signal.

## Considered Options

* `metaharness-darwin evolve` CLI — rejected: variant-blind on real repos (see Context).
* Hand-rolled evolution loop — rejected: re-implements GEPA's Pareto/holdout statistics, badly, and is prone to local-optimum trapping under noisy LLM fitness.
* **GEPA-as-library with Claude-only injected functions + read-layer telemetry — chosen.**

## Decision Outcome

Chosen option: **GEPA-as-library + read-layer telemetry**, implemented exactly as specified below.

### Part A — Read layer (telemetry)

A1. Run once per session or CI cycle:
```bash
npx ruflo metaharness score --path . --format json
npx ruflo metaharness genome --path . --format json
npx ruflo metaharness oia-audit --path . --alert-on-worst high
npx ruflo metaharness drift-from-history --threshold 0.95 --alert-on-new-severity high
```
A2. Persist each JSON output as a CI artifact (90-day retention). Alert on `harnessFit` drop > 10 points or any new high-severity finding. These scores are readiness gates, not quality measures — engine quality is judged only by the ADR-0005/0012 harness.

### Part B — Darwin loop (GEPA)

**B1. Scaffold.** Create `tools/gepa-loop/` outside the Cargo workspace:
```
tools/gepa-loop/
  package.json          # { "dependencies": { "@metaharness/darwin": "0.8.0" } }  — exact pin, no ^ or ~
  package-lock.json
  mine-corpus.mjs       # B2
  genome.seed.json      # B3
  evaluate.mjs          # B4
  reflect.mjs           # B5
  sweep.mjs             # B6–B7
  corpus/manifest.json  # B2 output (committed)
  runs/                 # gitignored
```
Add `tools/gepa-loop/runs/` and `tools/gepa-loop/node_modules/` to `.gitignore`. Version bumps of the pin require a manual diff of `dist/gepa/*.d.ts` before merging.

**B2. Corpus miner (`mine-corpus.mjs`).** Build the task set from this repo's own history:
1. `git log --format=%H --diff-filter=M -- 'crates/**/*.rs'` and keep commits whose diff touches ≥1 non-test source file AND ≥1 test (a `#[test]`/`#[cfg(test)]` hunk or a file under `tests/`).
2. For each candidate, in a fresh `git worktree` at that commit: revert only the source hunks (keep the test as committed); run the touched test target; **keep the candidate only if the test now fails**, and re-verify it passes when the revert is undone.
3. Emit `corpus/manifest.json`: `[{ id, commit, revert_patch, failing_test_cmd, description }]` plus a SHA-256 of the manifest body; split 70/30 into `train` and `holdout` (seeded shuffle).
4. **Gate 1:** ≥ 8 surviving tasks, else stop and report the shortfall.

**B3. Seed genome (`genome.seed.json`).** Extract the current working policy — this is the upgrade path from today's harness — into named text components:
```json
{ "components": {
    "planning_directive":  "<how the agent decomposes a task before editing>",
    "test_first_rule":     "<when to write the failing test before the fix>",
    "context_budget_rule": "<how much of the codebase to read before acting>",
    "retry_rule":          "<when to retry, escalate, or stop>",
    "review_rule":         "<what self-review runs before declaring done>"
} }
```
Populate each component verbatim from the existing `CLAUDE.md` / agent-prompt practice so the seed measures the status quo. Validate with `validateGenome()` from `@metaharness/darwin`'s `gepa` export.

**B4. Evaluator (`evaluate.mjs`).** Signature: `async (genome) => { scores, feedbacks, cost }` (GEPA's `GepaEvaluator`). Per task `T` in the evaluation slice:
1. Create an isolated worktree from `T.commit`, apply `T.revert_patch`.
2. Render the genome's components into a system prompt; run `claude -p` with it in the worktree, scrubbed env (PATH + repo paths only), wall-clock timeout, task instruction = `T.description` + the failing test's output.
3. Score: `scores[T.id] = 1` iff `T.failing_test_cmd` passes ∧ `cargo test --workspace` green ∧ `cargo test -p sf-conformance --test differential_tree` green; else `0`. On failure, `feedbacks[T.id]` = truncated failing output. `cost` += tokens spent.
4. Destroy the worktree.

**B5. Reflector (`reflect.mjs`).** Signature: `async (prompt) => { raw, cost }` (GEPA's `GepaReflector`). One `claude -p` call with the prompt GEPA builds (`buildReflectionPrompt`); return the raw text. No other model, ever.

**B6. Pilot — discrimination proof.** Before any full sweep: run `evaluate()` on 2–3 train tasks for (a) the seed genome and (b) a deliberately crippled genome (e.g. `context_budget_rule` = "read nothing before editing"). **Gate 2:** the two score vectors must differ; if they tie, the corpus or evaluator is not discriminating — stop and fix before spending more.

**B7. Sweep (`sweep.mjs`).**
```js
import { gepaOptimize } from '@metaharness/darwin';
const result = await gepaOptimize({
  seed, evaluate, reflect,
  mutable: Object.keys(seed.components),
  maxCandidates: 10, maxCost: <explicit $ cap>, maxStall: 3,
  onEvent: logToRunsDir,
});
```
Each sweep is human-launched with these caps stated up front. Persist `result` (pool, frontier, history, budget) under `runs/<timestamp>/`. **Gate 3:** the sweep ends in either a candidate that beats the seed on the holdout slice under the strict predicate — no gold regression, empty-output rate not worse, cost-per-resolved not worse (`evaluatePromotion` from the `gepa` export, via a thin adapter mapping B4 results into its `EvalSummary` shape) — or an explicit no-improvement report. No silent partial outcomes.

**B8. Adoption.** A promoted genome's changed component text is applied to `CLAUDE.md` / the agent prompts as a normal reviewable diff (commit/PR), never a silent overwrite. Re-run Part A telemetry after adoption as the standing regression signal.

### Consequences

* Good, because policy changes are only adopted on measured holdout wins over the current practice.
* Good, because Claude-only and real-fitness-only are structural properties of the implementation, not configuration.
* Good, because pool/frontier/history persistence makes every sweep auditable and resumable.
* Bad, because a sweep costs real tokens (≈ candidates × tasks `claude -p` runs) — capped by `maxCost` and the human launch gate.
* Bad, because the `@metaharness/darwin` pin must be re-verified manually on every bump.
* Neutral, because if the corpus can't discriminate (Gates 1–2), the correct outcome is a report, not a sweep.

### Confirmation

Gate 1 (corpus ≥ 8 verified fail-on-revert tasks, hash-pinned) → Gate 2 (pilot discrimination: different genomes ⇒ different scores) → Gate 3 (budgeted sweep ends in holdout-promoted genome or explicit negative) → adoption lands as a reviewable diff and Part A telemetry stays green. Each gate must pass before the next spends anything.

## More Information

* Library contracts used: `@metaharness/darwin@0.8.0` — `gepa` export: `gepaOptimize`, `validateGenome`, `mutateComponent`, `buildReflectionPrompt`, `paretoFrontier`, `evaluatePromotion`, `summarizeEval`.
* Fitness gates: ADR-0005 (conformance/bench harness), ADR-0012 (test strategy), ADR-0007 (`=_bag` correctness rail).
* Method lineage: GEPA — Genetic-Pareto reflective prompt evolution; Darwin Gödel Machine (arXiv:2505.22954).
