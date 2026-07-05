---
status: superseded by ADR-0026
date: 2026-07-05
tags: [meta-harness, darwin, gepa, evolve, claude-only, dev-loop, fitness-function, prompt-evolution, version-pinning]
supersedes:
  - ADR-0013
depends-on:
  - ADR-0005
  - ADR-0007
  - ADR-0012
implements: []
---

# Claude-only GEPA harness evolution (meta-harness, revised)

> **Dev / optimisation tool only — never on the fabric's runtime/serving path.**

## Context and Problem Statement

We want to evolve the coding agent's working policy for developing semantic-fabric — how it plans a task, whether it writes the test first, how much context it gathers before acting, how it retries, whether it self-reviews — and keep only changes that measurably improve real fitness on this repo, using **Claude models only** (no OpenRouter, no Gemini, no local non-Claude model, anywhere in the loop).

`@metaharness/darwin`'s `evolve` CLI does not fit this. Its real-repo evaluation path runs every variant's test command with `cwd: profile.root` — the one shared repo checkout — never the variant's own directory (`dist/sandbox.js`, `runVariantTask`). Parent and child therefore always execute an identical command against identical state, so `--bench` promotion cannot discriminate between variants regardless of suite design. This holds through the latest published release (0.8.0). The CLI's variant-discriminating evaluation modes (`sandboxMode: 'mock'` / `'agent'`) do differentiate variants, but only against a built-in synthetic file-location task suite, not against this repo. A tier that runs a real agent against real repo tasks per variant is not implemented upstream.

The same package ships a second, generic engine that does fit: GEPA (Genetic-Pareto reflective prompt evolution, `dist/gepa/`). Its genome is plain named prompt text, and both the evaluator and the mutation-proposer are functions supplied by the caller.

## Decision Drivers

* Claude-only: no third-party model anywhere in genome mutation or evaluation.
* Fitness must be real: `cargo test --workspace`, W3C differential conformance, `sf-bench` vs Ontop — never a proxy. No fitness definition may reward a correctness regression (ADR-0007's `=_bag` rail).
* Reproducibility: exact version pins, hash-pinned task corpus, seeded runs, persisted lineage.
* Bounded spend: every evolution sweep costs real Claude tokens; each sweep is an explicit, budgeted, human-launched operation.
* Discrimination must be proven before any paid sweep runs.

## Considered Options

* **`metaharness-darwin evolve` CLI** — real-repo evaluation is variant-blind by construction (see Context); rejected.
* **Patch `@metaharness/darwin`'s sandbox to execute in `variant.dir`** — variant directories contain only the seven policy files, not a Cargo workspace, so a patched sandbox would still have nothing repo-real to run; forks security-relevant third-party code for no signal gained; rejected.
* **Hand-rolled single-gene evolution loop** — reuses none of GEPA's tested Pareto-frontier/holdout-promotion statistics and is prone to trapping at local optima under noisy LLM-based fitness; rejected.
* **`@metaharness/darwin`'s GEPA module as a library, with our own injected Claude evaluator and reflector** — chosen.
* **Drop meta-harness entirely** — rejected; the read-layer telemetry (`score`/`genome`/`oia-audit`/`drift-from-history`) works as shipped and stays in use, and GEPA's frontier/promotion statistics are worth reusing.

## Decision Outcome

Chosen option: **GEPA-as-library with Claude-only injected functions**, because it is the only option that measures real fitness on this repo while enforcing Claude-only by construction — the evaluator and reflector are our own functions, so no other model can enter the loop — and it reuses tested selection/promotion statistics instead of a fragile hand-rolled loop.

### Specification

**1. Pinning.** `@metaharness/darwin` exact-pinned at `0.8.0` in `tools/gepa-loop/package.json` (its own workspace, isolated from the Cargo build — same pattern as `scratch/sqlx-spike`). Bump only after manually diffing the changed public contracts in `dist/gepa/*.d.ts` and `dist/bench/*.d.ts`. Never track `@latest`.

**2. Genome.** A `Record<string, string>` of named prompt-text components describing the dev agent's working policy:
   - `planning_directive` — how to decompose a task before touching code
   - `test_first_rule` — whether/when to write the failing test before the fix
   - `context_budget_rule` — how many related files / how much context to gather before acting
   - `retry_rule` — when to retry vs. escalate vs. give up
   - `review_rule` — whether/how to self-review before finishing
   Seed genome = the current working policy already implicit in this repo's agent prompts / `CLAUDE.md`, made explicit as these named components.

**3. Task corpus — build and validate this first, before anything else.** Mine `git log` for commits that both change a source file and add/modify a test. For each candidate commit: in an isolated worktree, revert only the source-file hunk (keep the test as committed); run the test and confirm it now fails; discard any candidate whose test does not fail on revert. Each surviving task is `{ parent_commit, worktree_setup, failing_test_id, task_description }` (description from the commit message). Write the surviving set as a hash-pinned manifest, split into train/holdout.

**4. Evaluator (our function, injected as `GepaEvaluator`).** For genome `G` on task `T`: render `G`'s components into the system prompt for a `claude -p` invocation; run it inside `T`'s isolated worktree under a scrubbed environment. Score:
   - `gold` = true iff the task's target test now passes AND `cargo test --workspace` is green AND `cargo test -p sf-conformance --test differential_tree` is green.
   - `feedback` = the failing command's output, truncated, for any task that did not reach `gold`.
   - `cost` = tokens spent on the run.

**5. Reflector (our function, injected as `GepaReflector`).** A single Claude call over GEPA's own `buildReflectionPrompt` output, returning proposed replacement text for the targeted component. No other model may be used here or anywhere else in the loop.

**6. Promotion.** Use GEPA's holdout predicate as shipped: promote a candidate genome only if, on the holdout split, it has no regression on any previously-gold task, its empty-output rate does not worsen, and its cost-per-resolved-task does not worsen. Build a small adapter mapping the evaluator's per-task results into GEPA's expected eval-artifact shape.

**7. Adoption.** A promoted genome's component text becomes a reviewable diff to the relevant agent prompts / `CLAUDE.md` — never an automatic, silent overwrite.

**8. Launch gate.** Each sweep runs with an explicit `maxCost` / `maxCandidates` / `maxStall` budget and is started deliberately by a human. Promotion within a sweep (step 6) is automatic on passing the predicate; starting a sweep at all is not.

**9. Hygiene.** One isolated git worktree per task run. `.metaharness/` and `tools/gepa-loop/runs/` stay untracked (`.gitignore`). Every `claude -p` child process runs under a scrubbed environment. This tooling never touches the fabric's runtime/serving path. The separate engine-perf "Path-B" loop (tuning the engine's own runtime knobs against `sf-bench` under the W3C non-degradation gate) is a different concern and is unaffected by this ADR.

### Supersession scope

Supersedes ADR-0013 in full. Carried forward by restatement here: the read-layer telemetry programme, the "dev tool only, never runtime" rule, and the engine-perf Path-B loop discipline (sf-bench knob tuning under the W3C non-degradation gate and the `=_bag` rail, with a repeated-run discipline against noise). Not carried forward: the `evolve`-CLI write-layer design, the `mint vertical:coding` genome seed, and tracking `@metaharness/darwin@latest`.

### Consequences

* Good, because evolution measures real semantic-fabric fitness by construction — no evaluator path can return a tied or uninformative score across genomes.
* Good, because Claude-only is structural, not a configuration flag.
* Good, because Pareto-frontier and holdout-promotion statistics are reused rather than reimplemented.
* Good, because exact pinning and a hash-pinned corpus make every sweep reproducible.
* Bad, because each sweep spends real tokens (order: dozens of `claude -p` task-runs per sweep) — bounded by explicit per-sweep budgets and a human launch gate.
* Bad, because a breaking change to GEPA's public contracts requires manual re-verification before any version bump.
* Neutral, because the whole approach depends on corpus quality; too few discriminating tasks should produce an explicit negative report, not a sweep.

### Confirmation

Gates, in order — each must pass before the next step spends anything:

1. **Corpus gate** — the miner yields at least 8 tasks whose test verifiably fails on revert and passes on restore; manifest is hash-pinned with a train/holdout split.
2. **Discrimination gate** — on a 2–3 task pilot, two deliberately different genomes produce different per-task scores.
3. **Sweep gate** — a budgeted `gepaOptimize` run completes within its caps and ends in either a holdout-promoted genome or an explicit no-improvement result.
4. **Adoption check** — any promotion lands as a reviewable diff to agent prompts / `CLAUDE.md`; CI readiness/drift telemetry continues as the standing regression signal.

## More Information

* Supersedes ADR-0013 (meta-harness development loop); see that record and `docs/research/ruflo-metaharness.md` / `ruflo-metaharness-darwin.md` for the earlier CLI-based design.
* Fitness gates: ADR-0005 (conformance + bench harness), ADR-0012 (test strategy), ADR-0007 (`=_bag` correctness rail).
* Package internals referenced: `@metaharness/darwin@0.8.0` — `dist/sandbox.js` (`runVariantTask`), `dist/gepa/{genome,loop,promotion}.d.ts`.
* Upstream lineage: Darwin Gödel Machine (arXiv:2505.22954); GEPA (Genetic-Pareto reflective prompt evolution).
* Session tracking: memory `darwin-mode-harness-evolution-horizon` (horizon-id `horizon-darwin-mode-harness-evolution`).
