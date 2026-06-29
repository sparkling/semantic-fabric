---
status: accepted
date: 2026-06-26
tags: [meta-harness, darwin, evolve, fitness-function, readiness, dev-loop, optimization, path-b, version-currency]
supersedes: []
depends-on:
  - ADR-0005
  - ADR-0006
  - ADR-0012
implements:
  - ADR-0001
---

# Meta-harness development loop

> **Forward-looking.** The *readiness-telemetry* half is usable now; the *Darwin* half activates only once the engine builds and the test/bench harness (ADR-0005/0012) produces fitness numbers. The meta-harness is a **dev / optimisation** tool — **never** on the fabric's runtime/serving path. Grounded in `docs/research/ruflo-metaharness.md` + `ruflo-metaharness-darwin.md` (2026-06-27).

## Context and Problem Statement

The ruflo meta-harness is a development/optimisation tool, not part of the fabric runtime. Its Darwin Mode is the **Darwin Gödel Machine** pattern (Zhang/Hu/Lu/Lange/Clune, [arXiv 2505.22954](https://arxiv.org/abs/2505.22954), ICLR 2026) productised by `ruvnet/agent-harness-generator`: *freeze the model, evolve the harness.* It plays two roles for semantic-fabric, and precision matters.

## Considered Options

* **Use `metaharness-darwin evolve` to tune the engine's runtime knobs** — rejected: `evolve` mutates the seven agent-harness *policy surfaces* (how agents build the repo), not the engine's rewriter/cascade/pool/plan-cache knobs.
* **Separate engine-perf Path-B loop over `sf-bench`** — chosen for engine tuning; borrows Darwin's archive + measured-promotion + safety philosophy under the W3C conformance non-degradation gate (ADR-0005), without using `metaharness-darwin evolve`.
* **Readiness telemetry via `harness genome` + `harness score` in CI** — chosen, usable now as a readiness gate (not a quality discriminator) while the engine matures.
* **Target the plugin-pinned `@metaharness/darwin@~0.3.1`** — chosen supported path: ruflo-tested, graceful-degradation-honouring, reachable via the `metaharness_*` MCP tools.
* **Target latest `@metaharness/darwin@0.7.1` directly via `npx`** — available and verified working but off the ruflo-tested path (MCP `metaharness_evolve` cannot reach it) until the 0.3→0.7 delta is smoke-checked.

## Decision Outcome

### 1. Readiness telemetry — usable now
Snapshot `harness genome` + `harness score` in CI to track agent-readiness drift as the engine matures. Baseline captured 2026-06-26: `unknown` / compileConfidence 12 / scaffoldReady false → `rust_ci` / 100 / true. **Snapshot 2026-06-27** (full 5-dim via the `metaharness_*` MCP tools): harnessFit **67** · compileConfidence **100** · taskCoverage **79** · toolSafety **100** · memoryUsefulness **40**; scaffoldReady true, hardConstraints 6/6, archetype `rust-crate-harness`, est $0.048/run. Genome: `rust_ci`, topology [maintainer, tester, security, release], risk_score **0.21**, mcp_surface `local_default_deny`, test_confidence 0.8, publish_readiness 0.75. `oia-audit` composite worst = **clean** (no MCP surface; threat-model `info`), persisted for drift tracking to `metaharness-audit/audit-2026-06-27T22-05-23-872Z`. Lowest dim = memoryUsefulness 40 (no agent-memory wired — expected: not a ruflo project, engine needs none). *Caveat:* the score is a readiness **gate**, not a quality discriminator (~0.985 ceiling) — engine quality is the test harness (ADR-0005/0012), never the meta-harness score.

### 2. Darwin `evolve` — what it actually mutates
The loop (upstream ADR-153): `profile → baseline (7 surface files) → mutate ONE surface → sandbox (run test cmd; no shell/net/secrets) → score (weighted − penalty) → archive (parent→child TREE) → select (sample whole archive; clade/pareto/quality-diversity) → repeat`. Static safety inspector rejects secret/shell/net/eval patterns before any variant runs; **exit 99 = safety-disqualified**. Proof it works: `@metaharness/darwin` self-reports **SWE-bench Lite 7.7% → 58.3%** via cheap→frontier tiering (verified) + **Darwin Shield**.

**The load-bearing fact:** `evolve` mutates the **seven agent-harness policy surfaces** (planner/contextBuilder/reviewer/retry/tool/memory/score) — *how agents build the repo* — **not** the engine's runtime knobs. So two distinct loops:

* **(i) `metaharness-darwin evolve`** — optimises the *dev harness* building semantic-fabric. Seed = `mint --template vertical:coding` (the profile is the genome evolve mutates).
* **(ii) engine-perf Path-B loop over `sf-bench`** — tunes the *engine's* knobs (rewriter/cascade heuristics, rayon/tokio/pool sizes, plan-cache, semi-join thresholds) under the **W3C conformance non-degradation gate** (ADR-0005) with **GTFS-Madrid OBDA-track latency + constant memory** as the objective. Borrows Darwin's archive + measured-promotion + safety philosophy; does **not** use `metaharness-darwin evolve`. Two findings from the upstream Darwin study (AgentDB Run C, 2026-06-22) discipline this loop: **(a) measure against noise** — Darwin's scalar proxy drifts 1.5–5× and bench scores carry sd≈0.45, so a promotion needs **n≥4–5 repeated runs** (ADR-0012) before a latency/memory delta is believed; **(b) immutable anti-reward-hacking rails** (upstream Darwin Rails ADR-164 / SGM risk-budget ADR-079) — the engine analogue is the hard invariant that **cost may choose only among `=_bag`-equivalent plans, and an optimization fires only when its integrity-constraint precondition holds** (ADR-0007), so the loop can never "win" by trading correctness for speed.

### 3. Versions & currency (checked 2026-06-27)
Published: `metaharness` 0.2.7 (read layer), **`@metaharness/darwin` latest 0.7.1**, `@metaharness/router` 0.3.2, `@metaharness/kernel` 0.1.2. **But the `ruflo-metaharness` plugin (v0.1.0) pins `@metaharness/darwin@~0.3.1`** (`_darwin.mjs`) → via the plugin we run **0.3.1**, four minors behind upstream. **Read/write asymmetry:** the plugin *floats* the read layer (`_harness.mjs` runs `metaharness@latest`, currently 0.2.7) but *pins* the write layer (`_darwin.mjs` → `~0.3.1`) — so `score`/`genome` track latest, `evolve` does not. Lineage: 0.1.0 (ADR-153 era) → **0.3.1** (plugin pin; the SWE-bench result above) → **0.7.1** (latest; the plugin's `~` pin does not reach it). The 0.3.1→0.7.1 delta (4 minors in ~5 days, late June 2026): a more rigorous SWE-bench re-report — **Lite 51.3% (n=300) + Verified 55.6% (278/500, Wilson 95% CI [51.2–59.9], official gold eval, no leakage)** — via an explicit GLM→Opus *compute-arbitrage* cascade, plus ~3.5× code growth (266 KB → 930 KB: more selection strategies + Darwin Shield). Headline framing shifted toward cost-optimal model tiering.

**Decision (amended 2026-06-29 — auto-update):** **track the latest `@metaharness/darwin` automatically** rather than holding the plugin's `~0.3.1` pin. Invoke the write layer with `npx -y -p @metaharness/darwin@latest metaharness-darwin <verb>` so `evolve` floats to current (0.7.1+), symmetric with the already-floating read layer (`harness score`/`genome` track `metaharness@latest`). This supersedes the prior "pin 0.3.1 / re-check versions manually after each `/plugin` reload" stance. Tradeoff (recorded): auto-updating forgoes the manual 0.3→0.7 smoke-check, so a breaking upstream release can land unannounced — bounded by graceful degradation + the automated gates above, with a failing post-update `harness score` / `oia-audit` as the regression signal.

### Constraints (upstream ADR-153)
removable · optional (`optionalDependencies` only) · graceful degradation · CI-gate. **Auto-promotion is allowed** (operator decision 2026-06-29, superseding the upstream ADR-153 "never auto-evolve / manual PR review" stance): a Darwin `evolve` winner — and an engine-perf Path-B candidate — is promoted **automatically when it clears the automated gates**, with no manual PR-review step. The gates that license an auto-promotion are non-negotiable and unchanged: the static safety inspector (`exit 99` = disqualified), Darwin Shield (ADR-155, security genome), Darwin Rails (ADR-164, immutable anti-reward-hacking), the SGM risk-budget (ADR-079; ADR-090 only wires it), genome-similarity (ADR-152), and — for the engine loop — the W3C conformance non-degradation gate (ADR-0005) + the `=_bag`-equivalent-plans rail (ADR-0007) + n≥4–5 repeated runs (ADR-0012). Darwin mutates the *target repo's* surfaces, not ruflo's own. Authoritative write surface = the **12 `metaharness_*` MCP tools shipped inside `@claude-flow/cli@3.13.2`**, not the stale standalone plugin-cache 0.1.0.

### Consequences

* Good, because the readiness-telemetry half is usable now — `harness genome` + `harness score` snapshot agent-readiness drift in CI as the engine matures.
* Good, because keeping the meta-harness strictly a dev/optimisation tool keeps it off the fabric's runtime/serving path.
* Good, because the engine-perf Path-B loop inherits Darwin's archive + measured-promotion + safety discipline (n≥4–5 repeated runs against noise; immutable anti-reward-hacking rails) while staying bound to the W3C conformance non-degradation gate, so it can never trade correctness for speed.
* Neutral, because the Darwin `evolve` half activates only once the engine builds and the ADR-0005/0012 harness produces fitness numbers.
* Neutral, because the readiness score is a gate (~0.985 ceiling), not a quality discriminator — engine quality stays the job of the test harness (ADR-0005/0012), never the meta-harness score.
* Bad, because the 2026-06-29 auto-promote + auto-update amendment removes two human checkpoints — an untested upstream `@metaharness/darwin@latest` release, or a weak-but-gate-passing variant, can land without manual review; both are bounded by the automated gates (safety inspector `exit 99`, Darwin Shield/Rails, the non-degradation + `=_bag` rails) and graceful degradation, with a failing `harness score` / `oia-audit` as the regression signal. Note the MCP `metaharness_evolve` tool still routes through the pinned `_darwin.mjs`, so the floating write layer is reached via `npx … @latest`, not that tool.

### Confirmation

Verified via the CI readiness snapshots (baseline 2026-06-26; full 5-dim 2026-06-27 via the `metaharness_*` MCP tools: harnessFit 67 / compileConfidence 100 / taskCoverage 79 / toolSafety 100 / memoryUsefulness 40, scaffoldReady true, hardConstraints 6/6), the `oia-audit` composite worst = clean (persisted for drift tracking to `metaharness-audit/audit-2026-06-27T22-05-23-872Z`), and the static safety inspector (`exit 99` = safety-disqualified) gating every variant. Engine fitness is confirmed against the ADR-0005 conformance/bench gates and the ADR-0012 test strategy (n≥4–5 repeated runs). The 0.7.1 bypass path (`npx -y -p @metaharness/darwin@0.7.1 metaharness-darwin <verb>`) is verified working.

## More Information
* **Research (grounding):** `docs/research/ruflo-metaharness.md` (read layer), `docs/research/ruflo-metaharness-darwin.md` (write layer / Darwin).
* **Fitness numbers:** ADR-0005 (conformance + bench), ADR-0012 (test harness). **Tunable knobs:** ADR-0006. **Profile seed:** `mint vertical:coding`.
* **Lineage:** Darwin Gödel Machine [arXiv 2505.22954](https://arxiv.org/abs/2505.22954); `ruvnet/agent-harness-generator`; ruflo upstream ADR-153. **Cross-project Path-B (authoritative):** the upstream design corpus.
* **Memory index (records-only):** this project's research is not indexed, so semantic-fabric meta-harness records are stored in **the upstream design corpus' memory index, namespace `semantic-fabric`** (keys `metaharness-darwin-dev-loop`, `sota-outstanding-synthesis-2026-06-27`) — retrievable via memory search. Upstream Darwin grounding lives in that store's `research`/`research-synthesis` namespaces (`darwin-mode-rerun-2026-06-22`, `finding-darwin-mode-mechanism`, `darwin-version-adr-hygiene`).

## Amendments

* **2026-06-29 (operator) — auto-promote + auto-update.** Dropped the upstream ADR-153 "**never auto-evolve in CI / promote via manual PR review**" constraint and the "**pin `@metaharness/darwin@~0.3.1`, re-check versions manually**" stance. Darwin `evolve` winners and engine-perf Path-B candidates now **auto-promote on passing the automated gates** (safety inspector `exit 99`, Darwin Shield/Rails, SGM risk-budget, genome-similarity; for the engine loop the W3C non-degradation gate + the `=_bag` rail + n≥4–5 repeated runs) — no manual PR-review step — and the write layer **auto-updates to `@metaharness/darwin@latest`**, symmetric with the already-floating read layer. **The correctness invariant is untouched:** an optimisation still fires only among `=_bag`-equivalent plans with its integrity-constraint precondition met (ADR-0007); only the human-review and version-pin checkpoints are removed, never the gates that prevent trading correctness for speed.
