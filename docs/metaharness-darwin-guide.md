# Using metaharness + Darwin Mode to develop a port

How to use `metaharness` and `@metaharness/darwin` ("freeze the model, evolve the harness")
to drive a code port, following the pattern rUv (metaharness's author) demonstrated live
porting Python `PixelRAG` to Rust on `ruvector`.

**The one thing to internalize first:** in the demo, rUv never runs a `darwin` command in a
shell. He works inside a Claude Code session (in the target repo) and gives the *agent*
plain-English instructions — "use `@metaharness/darwin` to evolve the code to the best
possible arch." The agent orchestrates the tooling. That is the primary workflow. The
`npm run evolve` shell command exists and is documented, but it's the mechanism the agent
uses, not the thing you type by hand.

---

## ELI15

- **`metaharness`** is a generator. Point it at a repo and it writes a small folder of
  config — agent prompts, permissions, MCP wiring — that turns your AI coding assistant into
  something that "knows" that repo.
- **`@metaharness/darwin`** is a *separate*, smaller package with one idea: **the model
  stays the same; the harness around it changes.** It takes a piece of your harness's policy
  (how it plans, what tools it may use, how it reviews), makes several random variations,
  scores each against a benchmark (still pass the tests? still safe? faster/cheaper?), and
  keeps only the variations that *measurably* beat the original. Repeat for a few
  generations. Evolution — mutate, test, keep the winners — applied to prompts/policy, not
  model weights.
- **"Port"** = you have code in one language and want it in another (rUv's case: Python
  PixelRAG → Rust). Your Claude Code agent does the actual porting. `@metaharness/darwin`
  never touches your port code — it evolves the **harness** (the agent's own policy config)
  and scores each variant by *"does the agent, using this harness, produce a port that
  passes the tests."* Better harness → better port outcomes, but the code is the agent's
  output, not Darwin's genome.
- **The catch that trips everyone up:** Darwin can only evolve the harness if it has a real
  **benchmark** to score harness variants against — and a benchmark means real port tasks
  with real (passing/failing) tests. Until you've landed some actual ported code, there's
  nothing for Darwin to measure against, so an evolve run has no signal. Plan the port
  (ADR) → land the first crates → *then* Darwin has a benchmark and can start evolving.

---

## Step-by-step (for dummies)

This mirrors the exact order rUv used in the demo. You do it all from **inside a Claude
Code session opened in the repo you're porting into** (e.g. `semantic-fabric`).

### 1. Plan the port — tell the architect agent to write an ADR

If you have the `ruflo-metaharness` plugin (its `metaharness-architect` agent is installed),
just instruct it in prose — this is rUv's actual first command, verbatim:

```
using @"ruflo-metaharness:metaharness-architect (agent)" create a detailed adr/ to port
<source-repo-url> to <target-repo> and <target-language>. benchmark and optimize
```

The agent creates a branch, finds the next ADR number from your repo's convention, and
drafts the ADR — including the benchmark harness (datasets, metrics, exact commands) and
optimization levers as a plan with honest "to-be-measured" placeholders.

### 2. Ask the agent to wire in Darwin as the optimization loop

Again, prose to the agent (rUv's second command, verbatim):

```
lets also use @metaharness/darwin to evolve the code to the best possible architecture
```

Note rUv says *"evolve the code"* — but that's loose phrasing, and the agent **corrected it
in its own words**, recording `@metaharness/darwin` in the ADR's optimization milestone as
a *"harness-evolution loop (`bench create → evolve`)"*. Darwin evolves the harness, not the
code. It's scoped as **removable augmentation** — matching `ruvector`'s own
[ADR-256](https://github.com/ruvnet/ruvector/blob/main/docs/adr/ADR-256-metaharness-sdk-evaluation.md),
which decided to *borrow* metaharness's concepts rather than take it as a hard runtime
dependency. Nothing runs yet — this is still planning.

### 3. Land the first real code — it becomes Darwin's benchmark

This is the step everyone gets wrong, and it's *not* because Darwin edits code. Darwin
evolves the harness; to do that it needs a benchmark, and the benchmark is your real port
tasks with real tests. Until code exists, there's nothing to score harness variants
against. Straight from the demo agent's own status check:

> "There's **nothing to benchmark or darwin-evolve yet** — this stage produced an ADR (a
> plan), not the Rust port. Real benchmarking and darwin runs happen **after the M0-M3
> implementation lands**."

So the real order is: commit the ADR → scaffold the first crates (stub the `pixelrag-*` /
your-`*` crates so there's something to compile and test) → *now* there's a benchmark for
Darwin to evolve the harness against. Then you hand the whole thing off, again in prose
(rUv's later command, verbatim):

```
using @metaharness/darwin and related components create a harness to fully port the
<source> to <target>. use concurrent workflow. @"ruflo-goals:horizon-tracker (agent)"
continue until validated, benchmarked and optimized
```

`ruflo-goals:horizon-tracker` is a long-horizon agent that keeps the loop going across
sessions until the stated done-condition (validated + benchmarked + optimized) is met.

### 4. What Darwin actually does under the hood

When the agent runs Darwin, or when you run it yourself, the documented entry point is one
npm script the scaffolded harness ships (README: *"run `npm run evolve` and **the harness
mutates its own config**, tests each change in a sandbox, and keeps only what measurably
improves. The model stays frozen; the harness evolves."*):

```bash
npm run evolve:dry   # sandbox=mock — $0, synthesizes scores, sanity-checks wiring
npm run evolve       # sandbox=real — invokes your host model per variant; costs tokens/time
```

`evolve:dry` is safe to run anytime to see the shape of the output. `evolve` spends real
money — run it deliberately. Results land in `.metaharness/`:

- `archive.json` — every variant tried, full score breakdown, and its children (a tree, not
  a single line)
- `lineage.json` — the winning path from `baseline` to the final winner
- `reports/`, `runs/` — per-run detail

Example `evolve:dry` output (real, from this repo) — note the `[bracketed]` label on each
variant is the **harness policy surface** that variant mutated (`planner`, `contextBuilder`,
`reviewer`, `toolPolicy`, `retryPolicy`, `scorePolicy`, `memoryPolicy`). That's the genome:
Darwin is mutating harness config, and `pass`/`safety` are how well that harness did on the
benchmark — no source file was touched.
```
Darwin Mode — leaderboard
  0.875  g2_v5  [contextBuilder]  safety=1.00  pass=0.80 ◀ winner
  0.765  baseline  [planner]      safety=1.00  pass=0.60
  ...
Winner: g2_v5
Lineage: baseline → g1_v0 → g2_v5
Delta over baseline: +0.110
```

---

## Real-world reference — rUv's exact prompt series

Transcribed verbatim (typos and all) from a live rUv demo (2026-06-25), porting Python
`StarTrail-org/PixelRAG` to Rust on `ruvector`. Every one of these is **prose typed into a
Claude Code session** — no shell commands. The `$` figures are the running session cost, to
show this is one continuous session, not four separate invocations.

**Prompt 1** — plan the port (≈17:50, session at $1.60):
```
create new branch, using @"ruflo-metaharness:metaharness-architect (agent)" create a
detailed adr/ to port https://github.com/StarTrail-org/PixelRAG to ruvector and rust.
benchmark and optmize
```
> Agent: *"I'll create the branch, check the ADR conventions/next number, then hand off to
> the metaharness-architect agent with full context."* → runs `git checkout -b
> feat/pixelrag-rust-port`, sees the latest ADR is 263, so the new one is **ADR-264**.

**Prompt 2** — attach Darwin as the optimizer (≈17:51, queued, $2.53):
```
lets also uise npm @metaharness/darwin to evole the code to the best possible arch
```

**Prompt 3** — attach a research dossier (≈17:51, queued alongside prompt 2):
```
and use the @ruvnet/ruvn to create detaield dossier.
```

> Agent's combined status recap (≈18:20) on prompts 2 + 3:
> - *"@metaharness/darwin — verified real; wired into the ADR's optimization milestone as a
>   **harness-evolution loop** (bench create → evolve --bench), scoped as removable
>   augmentation per ADR-256."*
> - *"@ruvnet/ruvn — run as you asked; verified clean but has no dossier capability (only
>   init/doctor)."* → it fell back to `ruflo-goals:dossier-investigator` for the dossier.
> - The load-bearing gate: *"There's **nothing to benchmark or darwin-evolve yet** — this
>   stage produced an ADR (a plan), not the Rust port... Real benchmarking and darwin runs
>   happen **after the M0-M3 implementation lands**."*
> - Offered next steps: commit ADR-264 → run `ruflo-goals:dossier-investigator` (since ruvn
>   can't) → start the M0 scaffold (stub the `pixelrag-*` crates so there's something for
>   darwin to evolve).

**Prompt 4** — execute the full port (≈18:29, $6.55):
```
using @metaharness/darwin and related components create a harness to fully port the
pixelrag python to rust. use concurrent workflow. @"ruflo-goals:horizon-tracker (agent)"
continue until validate, benchmarked and optmized.
```

The arc: **plan (ADR) → attach Darwin + dossier tooling → land real code → hand to a
long-horizon agent to drive the port to done.** The two `@"...(agent)"` references
(`ruflo-metaharness:metaharness-architect`, `ruflo-goals:horizon-tracker`) are installed
Claude Code plugin agents — you need the `ruflo-metaharness` and `ruflo-goals` plugins to
reproduce it as written.

---

## What this is not

- **Not a shell-command workflow.** The demonstrated path is prose to a Claude Code agent.
  `npm run evolve` is the underlying script, not the primary interface.
- **Not the `metaharness` scaffolder's headline use case.** The README/USERGUIDE frame a
  *scaffolded harness* as a standalone product (`npx your-name`, optionally `npm publish`).
  `@metaharness/darwin` is the piece explicitly meant to run against an existing repo —
  confirmed by `agentic-flow`'s [ADR-075](https://github.com/ruvnet/agentic-flow), which
  adopts Darwin's `evolve()` as a runtime library while leaving the scaffolder out of scope.
- **Not free once you leave `evolve:dry`.** Budget for `evolve` like any agent loop that
  calls a real model repeatedly.

## Sources

- `ruvnet/metaharness` README.md, `docs/USERGUIDE.md`
- `ruvnet/ruflo` `docs/metaharness-user-guide.md`
- `ruvnet/ruvector` `docs/adr/ADR-256-metaharness-sdk-evaluation.md`
- `ruvnet/agentic-flow` `docs/adr/ADR-075-metaharness-harness-evolution-and-provenance.md`
- Live demo screenshots (2026-06-25, "Agentics Live Vibe" session)
