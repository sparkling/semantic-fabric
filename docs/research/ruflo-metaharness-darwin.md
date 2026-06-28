# ruflo MetaHarness — Darwin Mode (`@metaharness/darwin`)

**Research key:** `ruflo-metaharness-darwin`
**Date:** 2026-06-27
**Evidence grade:** High — read from the installed `@metaharness/darwin@0.3.1`, the upstream integration ADR-153, and the ruflo plugin wrappers (`_darwin.mjs` / `evolve.mjs`); academic lineage web-verified.

---

## 1. What it is

> **"Freeze the model; evolve the harness."**

`@metaharness/darwin` **0.3.1** is the **write** layer of the meta-harness: population-based self-improvement of an agent harness. It generates child variants of a harness's policy surfaces, sandbox-scores them, archives the lineage as a tree, and promotes only **measured, safe** wins. The foundation model never changes — the *operating system around it* does. Dependency-free (Node built-ins).

**Academic lineage:** the **Darwin Gödel Machine** (Zhang, Hu, Lu, Lange, Clune — [arXiv 2505.22954](https://arxiv.org/abs/2505.22954), 2025 / ICLR 2026): a self-improving coding agent that iteratively rewrites its own code, validates each change empirically on coding benchmarks, keeps the foundation model frozen, and maintains an **open-ended archive** of agents (80 iterations, SWE-bench/Polyglot gains). `@metaharness/darwin` productises that pattern *at the harness level*; the archive-of-agents becomes the archive-tree of harness variants (a quality-diversity / MAP-Elites-style sampled archive).

## 2. The loop (upstream ADR-153)

```
repo → profile (RepoProfile) → baseline (7 surface files)
     → mutate ONE surface → sandbox (safety-inspect → run test cmd; no shell/net/secrets)
     → score (weighted base − penalty layer) → archive (parent→child TREE)
     → select (sample the WHOLE archive) → repeat
```

Output: `<repo>/.metaharness/{archive.json, lineage.json, variants/, runs/, reports/winner.json}`. The archive is a **tree, not a flat best-branch** — sampling the whole archive prevents premature convergence. Selection strategies in 0.3.x: `quality-diversity`, `behavioral-diversity`, `niche-steering`, `clade`, `pareto`.

### The seven mutation surfaces
`planner` · `contextBuilder` · `reviewer` · `retryPolicy` · `toolPolicy` · `memoryPolicy` · `scorePolicy`. **A variant is ONE mutation to ONE surface** — single degree of freedom keeps causal attribution clean.

### Safety model (load-bearing)
1. `inspectVariant()` rejects nested dirs / symlinks / non-regular files / secret-shaped / shell-out / network / env / dynamic-eval patterns.
2. `validateGeneratedCode()` runs **before** write (defence in depth).
3. Sandbox: variants run the test command with **no shell, no net, no secrets**.
4. Penalty layer: secret-exposure / destructive-action / hallucinated-file / tool-loop / cost-overrun subtract from score; a variant below parent by `promotionDelta` is **not** promoted.
5. **Exit code 99 = "disqualified by safety"** — a designed tripwire.

## 3. How it's invoked here

* CLI: `metaharness-darwin evolve <repo> [--generations --children --concurrency --seed --selection --sandbox]`.
* ruflo plugin `_darwin.mjs` shells to `npx -y -p @metaharness/darwin@~0.3.1 metaharness-darwin …` with graceful degradation + long async timeouts (evolve is the only minutes-to-hours verb).
* ruflo plugin `evolve.mjs`: **`--confirm` required** (dry-run plan + exit 0 otherwise); defaults `--generations 3 --children 3`; `--sandbox mock` for no-real-tests; exit codes `0` OK/dry-run/degraded, `1` no-improvement, `2` config/failure, `99` safety-disqualified.
* Three darwin subcommands: `evolve`, `bench <create|verify>`, `security bench` (**Darwin Shield**, upstream ADR-155).

## 4. Proof points (from the package itself)

`@metaharness/darwin@0.3.1` self-reports two measured applications:
1. **SWE-bench Lite code-repair: 7.7% open-loop → 58.3%** via cheap→frontier tiering (official swebench Docker, verified), **~$0.01–$0.74/instance** vs $1–20 for frontier agents.
2. **Darwin Shield** (v0.3.0) — defensive zero-day discovery harness (Semgrep + fuzz oracles, safety-gated, deterministic replay).

> **Version note:** upstream **ADR-153 documents the 0.1.0 integration**; the engine here is **0.3.1** — 0.2.x→0.3.x added the selection strategies, Darwin Shield, and the SWE-bench tiering result. Treat ADR-153 as the *integration design* and the package as ahead of it. **Latest upstream is now 0.7.1** (2026-06-26) — but the ruflo plugin pins `~0.3.1`, so via the plugin we run 0.3.1. 0.7.1 re-reports SWE-bench more rigorously (Lite 51.3% + Verified 55.6%, Wilson 95% CI), reframes around a GLM→Opus *compute-arbitrage* cascade, and grew ~3.5×. See ADR-0013 §3 for the version-currency decision.

## 5. What this means for semantic-fabric (the load-bearing distinction)

**Darwin evolves the agent-harness's seven *policy surfaces* — i.e. how agents BUILD the repo — NOT the engine's runtime knobs.** Two separate loops follow:

| Loop | What it optimises | Mechanism |
|---|---|---|
| **(i) `metaharness-darwin evolve`** | the *dev harness* building semantic-fabric (planner/tool/memory/… policies) | seed = `mint vertical:coding`; evolve against a bench suite |
| **(ii) Engine-perf (the Path-B *pattern*, not `evolve`)** | the *engine's* throughput/latency | sweep engine knobs (partition strategy, rayon/tokio/pool sizes, dedup thresholds) over `sf-bench`, gated by W3C conformance pass-rate |

Loop (ii) **borrows Darwin's philosophy** — archive + measured promotion under a non-degradation gate (Path-B) + a safety gate — but runs over the engine's own fitness numbers (ADR-0005), **not** `metaharness-darwin evolve`. Both are gated on the engine building + the test/bench harness producing numbers, so Darwin is the **last** loop, not a current step.

### Caveats
* **Sandbox tax:** mutation surfaces are limited to *pure policy logic* — anything needing side effects (net/fs/env) is rejected by the inspector. By design.
* **Time/compute:** generations × children × test-cost; a 3×4 run = 12 sandboxed variants × the test command. Default conservative.
* **Score ceiling:** the meta-harness `score` is a gate, not a quality discriminator (~0.985 ceiling) — engine quality is the test harness (ADR-0005/0012).
* **Promotion is manual:** Darwin leaves the winner under `.metaharness/`; promoting it = copying it back through normal PR review. **Do not auto-evolve in CI** (ADR-153).

## 6. Sources
- [Darwin Gödel Machine (arXiv 2505.22954)](https://arxiv.org/abs/2505.22954) · [jennyzzt/dgm](https://github.com/jennyzzt/dgm) — academic lineage (High)
- `@metaharness/darwin@0.3.1` package metadata + `dist/` (archive/clade/bench/cli) — read locally (High)
- ruflo upstream **ADR-153** — `@metaharness/darwin` integration (the loop, 7 surfaces, safety model, 4 constraints) (High)
- ruflo plugin `scripts/_darwin.mjs` + `scripts/evolve.mjs` — invocation contract (High)
- [ruvnet/agent-harness-generator `packages/darwin-mode`](https://github.com/ruvnet/agent-harness-generator) — upstream design ADR-070…075 (High)
