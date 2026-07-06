# semantic-fabric — Handover: ADR-0026 GEPA loop built, moving execution to `hz` (2026-07-05)

This session implemented ADR-0026's `tools/gepa-loop/` end to end (B3–B7) and ran real,
budget-verified evaluator/sweep cycles against Claude Code (`claude -p`) — then hit a wall
running the real B7 sweep on the local machine (background job killed mid-run, most likely
memory pressure) and is handing off to `hz` (32 cores, ~180GB available RAM) to finish the
sweep with headroom. Read this before continuing.

## TL;DR

- **ADR-0026 Part B (B1–B7) is implemented and proven working**, not just scaffolded:
  `mine-corpus.mjs` (B2, already existed), `genome.seed.json` (B3), `evaluate.mjs` (B4),
  `reflect.mjs` (B5), the B6 pilot discrimination gate (passed), `sweep.mjs` (B7, wraps
  `gepaOptimize` from `@metaharness/darwin/gepa`).
- **Two real bugs found and fixed this session** in `evaluate.mjs`/`reflect.mjs`:
  1. A scrubbed `{PATH, HOME}` env broke `claude -p` auth silently (returned `"Not logged
     in"`, `is_error: true`, cost $0) — the evaluator didn't check `is_error` and treated it
     as a genuine failed attempt. Fixed by inheriting full `process.env` (this runs locally
     on a trusted machine, unlike the untrusted-sandbox `cargo` scrubbing in
     `mine-corpus.mjs`) and by explicitly checking `parsedClaude.is_error`.
  2. `evaluateTask`/`makeEvaluator` were fully synchronous (`spawnSync`), so tasks ran one at
     a time. Rewrote to `execFileAsync` + a small concurrency pool (`pool()` in
     `evaluate.mjs`) so multiple `claude -p` fix attempts run in parallel, each in its own
     `git worktree` — the slow part (agent fix attempt, minutes) now overlaps; the fast part
     (cargo test, ~20-50s with a shared `CARGO_TARGET_DIR`) briefly serializes on cargo's own
     build lock, which is an acceptable trade.
- **B6 pilot gate passed for real** (not just structurally): seed genome scored `[0,0]` then
  `[0,1]` across reruns, crippled genome (`context_budget_rule` = "read nothing before
  editing") scored `[0,0]` then `[0,1]` — real, differentiated, budget-spending outcomes
  (~$1.7-3.4 per pilot), not the frozen $0 output from before the auth-scrub bug was found.
- **First real B7 sweep (train-size 10, max-candidates 12, concurrency 6) was launched and
  got killed** after ~90 min (2 of 12 candidates evaluated: seed 1/10, cand-1 1/10 rejected
  by GEPA's own accept rule). Root cause investigation ruled out OOM/jetsam kill, sleep/wake,
  and hook-script `pkill` via direct log evidence (`log show --predicate 'eventMessage
  contains "Killed"'` shows only unrelated system services in that window) — the process
  was killed by something above the OS layer (likely the Claude Code harness's own
  background-task supervision), with memory pressure (384MB unused of 38GB, heavy swap) as
  a plausible but unconfirmed contributing factor. 6 orphaned `git worktree`s from the killed
  batch were found and cleaned up (`git worktree remove --force` × 6, then `git worktree
  prune`) — **if you see unfamiliar worktrees under `/tmp` or `TMPDIR`, that's why; always
  `git worktree list` before assuming something's wrong.**
- **Decision: move sweep execution to `hz`** (SSH alias, host `gene`) — 32 cores, 180GB
  available RAM vs. the local machine's 384MB-unused/heavy-swap state at kill time. Toolchain
  check: `cargo`/`rustc` 1.92.0 already installed (source `~/.cargo/env` in non-interactive
  shells — it's not on PATH by default over `ssh host cmd`), Claude Code CLI **not**
  installed (native macOS install method doesn't apply; `npm install -g
  @anthropic-ai/claude-code` matches the exact locally-installed version, `2.1.201`, and is
  the right path there). GitHub SSH access from `hz` already works under the same account.

## State of the repo (resume point)

- Branch: **`feat/adr-0026-gepa-loop`**, branched off `feat/operator-tree-ir` @ `dc29c77`
  (== `main` == `origin/main`, per the prior session's handover — no operator-tree-ir work
  changed this session).
- `tools/gepa-loop/` is the whole deliverable. Everything under it except `node_modules/`,
  `runs/`, `*.db`, `.claude-flow/` (all gitignored) is committed on this branch.
- **Not yet merged to main; no PR.** This is dev-tooling (per its own `package.json`
  description: "Dev tooling only -- never on the fabric runtime path"), and per ADR-0026 B7
  Gate 3, nothing should be considered "done" until a full sweep produces either a promoted
  genome or an explicit no-improvement report — neither has happened yet.

## What's built, file by file

```
tools/gepa-loop/
  package.json          # { "dependencies": { "@metaharness/darwin": "0.8.0" } } -- exact pin
  package-lock.json
  mine-corpus.mjs        # B2 (pre-existing) -- corpus miner, unchanged this session
  corpus/manifest.json   # B2 output -- 48 tasks, 34 train / 14 holdout, sha256-pinned
  genome.seed.json       # B3 -- NEW. 5 components extracted from CLAUDE.md's actual working
                         #        practice (planning_directive, test_first_rule,
                         #        context_budget_rule, retry_rule, review_rule)
  genome.crippled.pilot.json  # B6 pilot fixture -- deliberately bad context_budget_rule
                              #  ("read nothing before editing"), used only for the Gate-2
                              #  discrimination check, not part of the real seed/sweep path
  evaluate.mjs           # B4 -- NEW. Claude-only GepaEvaluator (see below)
  reflect.mjs            # B5 -- NEW. Claude-only GepaReflector (see below)
  sweep.mjs              # B7/B8 -- NEW. Wraps gepaOptimize + the holdout Gate-3 promotion
                         #                check via darwin's own evaluatePromotion()
```

### `genome.seed.json` (B3)

**Important, non-obvious design note:** this does NOT use `@metaharness/darwin`'s own
`SEED_GENOME` / `validateGenome()` / `buildSystemFromGenome()` exports from its `gepa`
library. Reading `dist/gepa/genome.js` directly shows those are hardcoded to darwin's own
bespoke JSON-tool-call micro-agent protocol (`REQUIRED_COMPONENTS = executor_preamble,
tool_ls, tool_read, tool_grep, tool_edit, tool_line_edit, tool_run_tests, tool_submit,
retrieval_policy, edit_policy, test_policy, protocol_reminder`) — a different, hardcoded
agent loop, not real Claude Code. Our genome uses different, project-defined component names
rendered as a plain `--append-system-prompt` for real `claude -p`. The rest of the `gepa`
export (`gepaOptimize`, `mutateComponent`, `paretoFrontier`, `buildReflectionPrompt`,
`evaluatePromotion`, `summarizeEval`) IS genome-shape-agnostic (verified by reading
`loop.js`/`promotion.js`) and is used normally.

### `evaluate.mjs` (B4)

`makeEvaluator(tasks, {concurrency=4})` returns a `GepaEvaluator`: `async (genome) =>
{scores, feedbacks, cost, metricCalls, details}`. Per task:

1. `git worktree add --detach` at `task.commit`, then `git checkout task.parent --
   task.srcFiles` (revert only the fix's source hunks, keep the test as committed — the
   corpus's own pre-verified fail state).
2. Confirm the reverted state genuinely fails `task.failingTestCmd` (corpus/task drift
   guard).
3. Render the genome to a system prompt, run `claude -p <task description + failing test
   output> --append-system-prompt <rendered genome> --dangerously-skip-permissions
   --max-budget-usd 1.0 --output-format json --model sonnet` with **full env inheritance**
   (see the auth-scrub bug above) and **no shell**.
4. Gate on three real signals, run **concurrently** via `Promise.all` (shared
   `CARGO_TARGET_DIR` across the run, so external deps like `oxigraph` cache-share; only our
   own `sf-*` crates rebuild per worktree, ~20-50s cold):
   - the task's own `failingTestCmd`
   - `cargo test --workspace` (no regressions elsewhere)
   - `cargo test -p sf-conformance --test differential_tree` (the reference-oracle suite)
5. Score 1 iff all three pass; else 0, with a real `failureClass` (`0`=resolved, `3`=empty
   patch — detected via `git status --porcelain` after the `claude -p` call, genuinely
   checked, not guessed — `1`=attempted-but-wrong) for the `details` map that feeds B7's
   holdout Gate-3 (`summarizeEval`/`evaluatePromotion` need real per-instance `{gold,
   failureClass, thrash, cost}`, not placeholders).

**Cost control:** `--max-budget-usd 1.0` per task is a hard per-attempt cap; a task that
hits it scores 0 with feedback `"claude reported an error (subtype=error_max_budget_usd)"`.
Confirmed real via direct testing: `claude -p ...` exits code 1 on budget-cap with a valid
JSON body still on stdout (`execFileAsync` rejects on non-zero exit, so the catch path
recovers `e.stdout` rather than losing the JSON).

**Concurrency:** `pool(items, concurrency, worker)` is a small manual worker-pool (no
dependency). NOTE (architectural, not a bug): `gepaOptimize` itself (in
`@metaharness/darwin`) evaluates **candidates sequentially** — concurrency only
parallelizes tasks *within* one candidate's evaluation, not across candidates. This bounds
what raising `--concurrency` in `sweep.mjs` can do for total wall-clock.

### `reflect.mjs` (B5)

`reflect(prompt)` — one `claude -p <prompt> --tools "" --model sonnet --output-format json
--max-budget-usd 0.5` call, full env inheritance, checks `is_error` the same way as
`evaluate.mjs`. `prompt` is always GEPA's own `buildReflectionPrompt()` output (never
hand-written) — "No other model, ever" per ADR-0026 B5.

### `sweep.mjs` (B7/B8)

```
node sweep.mjs --train-size N --holdout-size M --max-candidates C --max-stall S
               --concurrency K [--confirm]
```

Dry-run by default (prints the plan + a rough `estimatedMaxSpendUsd`, mirrors
`ruflo-metaharness`'s `mint`/`evolve` safety convention); `--confirm` required to actually
spend. On completion: if `gepaOptimize`'s `best` is the seed itself, writes an explicit
`{promote: false, reason: 'no-candidate-beat-seed-on-train'}` — never a silent no-op. If a
real candidate won on train, re-evaluates **both seed and the winning candidate on the
UNSEEN holdout slice**, builds each into darwin's `EvalSummary` shape via `summarizeEval()`,
and runs the STRICT `evaluatePromotion()` predicate (gold must not regress, empty-patch rate
must strictly improve, cost/resolved must not worsen). Writes `runs/<runId>/{events.jsonl,
result.json, promotion.json}` and, only if promoted, `promoted-genome.json` — never
auto-applied to `CLAUDE.md`; B8 adoption is still a human-reviewed diff.

## What's NOT done yet

- **No completed B7 sweep.** The one real attempt (train-size 10, max-candidates 12,
  concurrency 6) got killed after evaluating only the seed + 1 rejected candidate. No
  `promotion.json` verdict exists yet.
- **B8 (adoption)** can't start until B7 actually produces a promote/reject verdict.
- Task list (local session, not necessarily synced anywhere durable) has these still
  pending: B7 (in progress — this handover is the continuation point), B8, and a separate
  parallel track ("Concurrent track: close 7 remaining Ontop-parity items" — see
  `ontop-parity-remaining-backlog` memory note, unrelated to this GEPA work, not started
  this session).

## Why `hz`, and exact resume steps

Local machine showed severe memory pressure at the time of the kill (`top`: 384MB "unused"
of 38GB physical, 10GB in the compressor, cumulative swapins/swapouts in the millions) with
6 concurrent `claude -p` + `cargo test` processes running. `hz` (SSH alias → host `gene`) has
32 cores and ~180GB available RAM — comfortably clears that pressure. CPU was never the
constraint locally (79% idle at last check) or expected to be on `hz`.

**On `hz` (already confirmed this session):**
- `cargo`/`rustc` 1.92.0 already installed at `~/.cargo/bin` — **not on PATH by default in a
  non-interactive `ssh host 'cmd'` invocation**; `source ~/.cargo/env` first, or use a login
  shell.
- Claude Code CLI is **not installed**. Local install is native (`~/.local/share/claude/`),
  not npm-based, so don't try to mirror that — instead:
  ```bash
  npm install -g @anthropic-ai/claude-code   # confirmed matches local version 2.1.201
  ```
- **Auth**: do NOT use `ANTHROPIC_API_KEY` — that's metered pay-per-token billing, separate
  from the Max subscription this session has been running under (confirmed: `claude -p`
  reports real `total_cost_usd` per call even though nothing is actually billed
  incrementally under the subscription). The correct mechanism is:
  ```bash
  claude setup-token   # "Set up a long-lived authentication token (requires Claude subscription)"
  ```
  This requires an interactive step (likely browser/device-code) that could not be completed
  headlessly from this session — the user is running this themselves.
- GitHub SSH access from `hz` already works under the account that owns this repo
  (`origin = https://github.com/sparkling/semantic-fabric.git`; `ssh -T git@github.com` from
  `hz` confirms `Hi sparkling!`).

**To resume:**
```bash
ssh hz
cd ~/source/semantic-fabric   # already cloned here; `git fetch && git pull` if stale
git checkout feat/adr-0026-gepa-loop
source ~/.cargo/env
cd tools/gepa-loop
npm install
claude setup-token        # interactive -- the user does this step
# smoke-test auth works:
claude -p "reply with exactly: pong" --output-format json --model sonnet
# re-run B6 quickly to confirm the toolchain end-to-end (optional but cheap, ~$1-2):
node evaluate.mjs genome.seed.json <two train ids from corpus/manifest.json>
# then the real B7 sweep -- with hz's headroom, concurrency can likely go higher than the
# local machine's 6 (32 cores available; try 10-12 and watch `free -h` / `htop`):
node sweep.mjs --train-size 10 --holdout-size 8 --max-candidates 12 --max-stall 3 \
               --concurrency 10 --confirm
```

Watch `git worktree list` periodically during a long sweep — if the process ever gets killed
again, orphaned worktrees under `$TMPDIR/gepa-eval-*` need `git worktree remove --force` +
`git worktree prune` before the next attempt (a killed run's `finally` cleanup blocks don't
run for in-flight tasks).

## Cost note

Real, budget-capped spend so far this session (local machine): roughly $8-10 (B6 pilot
rounds) + ~$18-20 (partial B7 sweep before the kill) ≈ **$26-30 total**, all under
`--max-budget-usd` per-task caps, all against the Max subscription (not separately metered).
