#!/usr/bin/env node
// evaluate.mjs — ADR-0026 B4. Claude-only GepaEvaluator: async (genome) => { scores, feedbacks, cost }.
//
// Per task: create an isolated git worktree at task.commit, revert task.srcFiles to task.parent's
// version (reproduces the pre-fix failing-test state), render the genome into a system prompt, run
// `claude -p` in the worktree with a hard per-task spend cap, then gate on three real signals:
// the task's own failing test, the full workspace suite, and the differential_tree reference oracle.
//
// NOTE on why this does NOT use @metaharness/darwin's gepa `validateGenome()` / `buildSystemFromGenome()`
// / `SEED_GENOME`: reading dist/gepa/genome.js directly shows those are hardcoded to darwin's OWN
// bespoke JSON-tool-call micro-agent protocol (REQUIRED_COMPONENTS = executor_preamble, tool_ls,
// tool_read, tool_grep, tool_edit, tool_line_edit, tool_run_tests, tool_submit, retrieval_policy,
// edit_policy, test_policy, protocol_reminder). Our genome uses different, project-defined component
// names (planning_directive/test_first_rule/context_budget_rule/retry_rule/review_rule) rendered as a
// plain system-prompt for real Claude Code (`claude -p`), not darwin's internal protocol. The rest of
// the gepa export (gepaOptimize, mutateComponent, paretoFrontier, buildReflectionPrompt,
// evaluatePromotion, summarizeEval) is genome-shape-agnostic (verified by reading loop.js/promotion.js)
// and is used normally in sweep.mjs.

import { execFileSync, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const execFileAsync = promisify(execFile);

const REPO_ROOT = join(import.meta.dirname, '..', '..');
// ONE shared target dir across every task/genome in a run (not per-task): a worktree's absolute
// path differs per invocation so our own sf-* crates still rebuild, but external dependencies
// (oxigraph et al. -- the bulk of a cold workspace compile) are path-independent and cache-share.
const CARGO_TARGET_DIR_BASE = join(REPO_ROOT, 'target', 'gepa-eval-shared');
const CLAUDE_TASK_TIMEOUT_MS = 10 * 60 * 1000;
const CARGO_TEST_TIMEOUT_MS = 8 * 60 * 1000;
const MAX_BUDGET_USD_PER_TASK = 1.0;

function git(cwd, argv) {
  return execFileSync('git', argv, { cwd, encoding: 'utf-8' }).trim();
}

const COMPONENT_ORDER = [
  'planning_directive', 'test_first_rule', 'context_budget_rule', 'retry_rule', 'review_rule',
];

function renderSystemPrompt(genome) {
  const c = genome.components;
  return COMPONENT_ORDER.filter((k) => c[k]).map((k) => `## ${k}\n${c[k]}`).join('\n\n');
}

async function runCargo(cwd, argv, targetDir) {
  try {
    const { stdout, stderr } = await execFileAsync('cargo', argv, {
      cwd,
      encoding: 'utf-8',
      timeout: CARGO_TEST_TIMEOUT_MS,
      maxBuffer: 32 * 1024 * 1024,
      env: { ...process.env, CARGO_TARGET_DIR: targetDir },
    });
    return { exitCode: 0, stdout, stderr, timedOut: false };
  } catch (e) {
    return { exitCode: e.code ?? 1, stdout: e.stdout ?? '', stderr: e.stderr ?? '', timedOut: e.signal === 'SIGTERM' };
  }
}

/** Run one task against one genome. Returns { score, feedback, cost }. */
async function evaluateTask(genome, task, worktreeRoot) {
  const wtPath = join(worktreeRoot, task.id);
  const targetDir = CARGO_TARGET_DIR_BASE;
  let cost = 0;
  try {
    git(REPO_ROOT, ['worktree', 'add', '--detach', wtPath, task.commit]);
  } catch (e) {
    return { score: 0, feedback: `worktree-add-failed: ${e.message.slice(0, 300)}`, cost };
  }
  try {
    git(wtPath, ['checkout', task.parent, '--', ...task.srcFiles]);
    const failingArgs = task.failingTestCmd.split(/\s+/).slice(1);
    const before = await runCargo(wtPath, failingArgs, targetDir);
    if (before.exitCode === 0) {
      return { score: 0, feedback: `task ${task.id}: reverted state did not fail (${task.failingTestCmd}) — corpus/task drift, skipping`, cost };
    }

    const systemPrompt = renderSystemPrompt(genome);
    const userPrompt = [
      `Fix this defect: ${task.description}`,
      '',
      `The following test currently fails and must pass: \`${task.failingTestCmd}\``,
      '',
      '--- failing test output ---',
      before.stdout.slice(0, 4000),
      before.stderr.slice(0, 4000),
      '',
      `Known affected file(s): ${task.srcFiles.join(', ')}`,
    ].join('\n');

    let parsedClaude;
    let claudeStdout = '';
    try {
      const { stdout } = await execFileAsync('claude', [
        '-p', userPrompt,
        '--append-system-prompt', systemPrompt,
        '--dangerously-skip-permissions',
        '--max-budget-usd', String(MAX_BUDGET_USD_PER_TASK),
        '--output-format', 'json',
        '--model', 'sonnet',
      ], {
        cwd: wtPath,
        encoding: 'utf-8',
        timeout: CLAUDE_TASK_TIMEOUT_MS,
        maxBuffer: 32 * 1024 * 1024,
        // Full env inheritance, not a scrubbed subset: auth here is OS-keychain/OAuth-backed (no
        // ANTHROPIC_API_KEY in this environment), not a plain env var -- a narrow {PATH,HOME} scrub
        // broke it silently (claude returned "Not logged in", is_error:true, cost $0, and evaluate.mjs
        // proceeded as if the agent had genuinely attempted and failed the task). This runs locally on
        // the same trusted machine as every other tool call in this session, so full inheritance is the
        // correct posture here (contrast the untrusted-sandbox scrubbing in mine-corpus.mjs's cargo runs).
        env: process.env,
      });
      claudeStdout = stdout;
    } catch (e) {
      // execFileAsync rejects on non-zero exit OR timeout; claude's own JSON (incl. is_error) may
      // still be on e.stdout (e.g. error_max_budget_usd exits non-zero but emits a valid JSON result).
      claudeStdout = e.stdout ?? '';
      if (!claudeStdout) {
        return { score: 0, feedback: `task ${task.id}: claude invocation failed: ${e.message.slice(0, 300)}`, cost };
      }
    }
    try {
      parsedClaude = JSON.parse(claudeStdout);
    } catch (e) {
      return { score: 0, feedback: `task ${task.id}: could not parse claude JSON output: ${e.message}. stdout: ${claudeStdout.slice(0, 500)}`, cost };
    }
    cost = parsedClaude.total_cost_usd || 0;
    if (parsedClaude.is_error) {
      return { score: 0, feedback: `task ${task.id}: claude reported an error (subtype=${parsedClaude.subtype}): ${parsedClaude.result || parsedClaude.errors?.join('; ')}`, cost };
    }

    // Real empty-patch signal (darwin's failureClass===3, ADR-0026 gate-3 "empty-patch rate improves"
    // criterion) -- did the agent change anything at all? NOT a placeholder: read from git, not guessed.
    const diffStat = git(wtPath, ['status', '--porcelain']);
    const emptyPatch = diffStat.trim().length === 0;

    // Shared CARGO_TARGET_DIR across concurrent tasks means cargo's own build-lock briefly
    // serializes these three -- acceptable: the compile+test phase is seconds (pilot: 20-50s cold),
    // while the `claude -p` agent call above (minutes) is the actual wall-clock cost and DOES run
    // fully parallel across concurrent evaluateTask() calls.
    const [targeted, workspace, diffTree] = await Promise.all([
      runCargo(wtPath, failingArgs, targetDir),
      runCargo(wtPath, ['test', '--workspace'], targetDir),
      runCargo(wtPath, ['test', '-p', 'sf-conformance', '--test', 'differential_tree'], targetDir),
    ]);

    if (targeted.exitCode === 0 && workspace.exitCode === 0 && diffTree.exitCode === 0) {
      return { score: 1, gold: true, failureClass: 0, feedback: `task ${task.id}: resolved (targeted + workspace + differential_tree all green), score 1 (cost $${cost.toFixed(4)})`, cost };
    }
    const parts = [];
    if (targeted.exitCode !== 0) parts.push('targeted test still fails');
    if (workspace.exitCode !== 0) parts.push('workspace regression');
    if (diffTree.exitCode !== 0) parts.push('differential_tree regression');
    const failOut = targeted.exitCode !== 0 ? targeted : workspace.exitCode !== 0 ? workspace : diffTree;
    return {
      score: 0,
      gold: false,
      failureClass: emptyPatch ? 3 : 1,
      feedback: `task ${task.id}: unresolved (${parts.join(', ')}${emptyPatch ? ', empty patch -- agent made no changes' : ''}). Trace:\n${failOut.stdout.slice(0, 2000)}\n${failOut.stderr.slice(0, 2000)}`,
      cost,
    };
  } catch (e) {
    return { score: 0, gold: false, failureClass: 1, feedback: `task ${task.id}: evaluator error: ${e.message.slice(0, 300)}`, cost };
  } finally {
    try { git(REPO_ROOT, ['worktree', 'remove', '--force', wtPath]); } catch { /* best effort */ }
  }
}

/** Run `items` through `worker` with at most `concurrency` in flight at once. */
async function pool(items, concurrency, worker) {
  const results = new Array(items.length);
  let next = 0;
  async function runner() {
    while (next < items.length) {
      const i = next++;
      results[i] = await worker(items[i], i);
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, items.length) }, runner));
  return results;
}

/** Build a GepaEvaluator bound to a fixed task list (train slice, holdout slice, or a custom subset). */
export function makeEvaluator(tasks, { concurrency = 4 } = {}) {
  return async function evaluate(genome) {
    const worktreeRoot = mkdtempSync(join(tmpdir(), 'gepa-eval-'));
    const scores = {};
    const feedbacks = {};
    // Per-instance detail map, EvalArtifact-shaped ({gold, failureClass, thrash, cost}) -- for
    // sweep.mjs's holdout gate (summarizeEval/evaluatePromotion). Extra field; gepaOptimize itself
    // only reads scores/feedbacks/cost/metricCalls and ignores this.
    const details = {};
    let cost = 0;
    try {
      await pool(tasks, concurrency, async (task) => {
        process.stderr.write(`[evaluate] start ${task.id} — ${task.description.slice(0, 70)}\n`);
        const result = await evaluateTask(genome, task, worktreeRoot);
        scores[task.id] = result.score;
        feedbacks[task.id] = result.feedback;
        details[task.id] = { gold: result.gold ?? false, failureClass: result.failureClass ?? 1, thrash: 0, cost: result.cost };
        cost += result.cost;
        process.stderr.write(`[evaluate] done  ${task.id} -> score=${result.score} cost=$${result.cost.toFixed(4)}\n`);
      });
    } finally {
      rmSync(worktreeRoot, { recursive: true, force: true });
    }
    return { scores, feedbacks, cost, metricCalls: tasks.length, details };
  };
}

// CLI: node evaluate.mjs <genome.json> [taskId ...]  (default: all train tasks)
if (import.meta.url === `file://${process.argv[1]}`) {
  const manifest = JSON.parse(readFileSync(join(import.meta.dirname, 'corpus', 'manifest.json'), 'utf-8'));
  const [genomePath, ...taskIds] = process.argv.slice(2);
  if (!genomePath) {
    console.error('usage: node evaluate.mjs <genome.json> [taskId ...]  (default: all train tasks)');
    process.exit(2);
  }
  const genome = JSON.parse(readFileSync(genomePath, 'utf-8'));
  const ids = taskIds.length ? taskIds : manifest.train;
  const tasks = manifest.tasks.filter((t) => ids.includes(t.id));
  const evaluate = makeEvaluator(tasks);
  const result = await evaluate(genome);
  console.log(JSON.stringify(result, null, 2));
}
