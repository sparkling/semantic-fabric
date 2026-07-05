#!/usr/bin/env node
// ADR-0026 B2 — corpus miner. Finds commits that pair a source change with an
// integration test, verifies the test fails when the source change alone is
// reverted (and passes when restored), and emits a hash-pinned manifest.
//
// A task's "revert" is stored as {commit, parent, srcFiles} rather than a raw
// diff blob: replaying it via `git checkout <parent> -- <srcFiles>` inside a
// worktree pinned at <commit> is robust to context drift in a way that
// applying a stored patch is not, and it is exactly as reproducible since
// every field is a pinned SHA.
//
// Usage: node mine-corpus.mjs [--limit N]

import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';

const REPO_ROOT = join(import.meta.dirname, '..', '..');
const CARGO_TARGET_DIR = join(REPO_ROOT, 'target', 'gepa-corpus-mining');
const TEST_TIMEOUT_MS = 5 * 60 * 1000;

const args = process.argv.slice(2);
const limitIdx = args.indexOf('--limit');
const LIMIT = limitIdx >= 0 ? parseInt(args[limitIdx + 1], 10) : Infinity;

function git(cwd, argv) {
  return execFileSync('git', argv, { cwd, encoding: 'utf-8' }).trim();
}

function findCandidates() {
  const log = git(REPO_ROOT, ['log', '--format=COMMIT %H|%s', '--name-only']);
  const candidates = [];
  let commit = null, subj = null, srcFiles = [], testFiles = [];
  const flush = () => {
    if (commit && srcFiles.length > 0 && testFiles.length > 0) {
      candidates.push({ commit, subj, srcFiles: [...srcFiles], testFiles: [...testFiles] });
    }
  };
  for (const line of log.split('\n')) {
    if (line.startsWith('COMMIT ')) {
      flush();
      const rest = line.slice('COMMIT '.length);
      const bar = rest.indexOf('|');
      commit = rest.slice(0, bar);
      subj = rest.slice(bar + 1);
      srcFiles = []; testFiles = [];
      continue;
    }
    if (/^crates\/[^/]+\/src\/.*\.rs$/.test(line)) srcFiles.push(line);
    else if (/^crates\/[^/]+\/tests\/.*\.rs$/.test(line)) testFiles.push(line);
  }
  flush();
  return candidates;
}

function crateAndStem(testFile) {
  const m = testFile.match(/^crates\/([^/]+)\/tests\/(.+)\.rs$/);
  return { crate: m[1], stem: m[2] };
}

function testCmdFor(testFiles) {
  // Multiple test files in one commit: require they're all in the same crate
  // (a cross-crate multi-file fix commit is out of scope for a single task).
  const parsed = testFiles.map(crateAndStem);
  const crate = parsed[0].crate;
  if (!parsed.every((p) => p.crate === crate)) return null;
  const args = ['test', '-p', crate];
  for (const p of parsed) args.push('--test', p.stem);
  return args;
}

function runCargo(cwd, testArgs) {
  const r = spawnSync('cargo', testArgs, {
    cwd,
    encoding: 'utf-8',
    timeout: TEST_TIMEOUT_MS,
    env: { ...process.env, CARGO_TARGET_DIR },
  });
  return { exitCode: r.status ?? 1, stdout: r.stdout ?? '', stderr: r.stderr ?? '', timedOut: r.error?.code === 'ETIMEDOUT' };
}

function verifyCandidate(cand, worktreeRoot) {
  let parent;
  try {
    parent = git(REPO_ROOT, ['rev-parse', `${cand.commit}^`]);
  } catch {
    return { ok: false, reason: 'no-parent-commit-root-of-history' };
  }
  const wtPath = join(worktreeRoot, cand.commit.slice(0, 12));
  try {
    git(REPO_ROOT, ['worktree', 'add', '--detach', wtPath, cand.commit]);
  } catch (e) {
    return { ok: false, reason: `worktree-add-failed: ${e.message.slice(0, 200)}` };
  }
  try {
    const testArgs = testCmdFor(cand.testFiles);
    if (!testArgs) return { ok: false, reason: 'multi-crate-test-files-unsupported' };

    // Step 1: revert the source hunks only (checkout parent's version of the src files).
    git(wtPath, ['checkout', parent, '--', ...cand.srcFiles]);
    const reverted = runCargo(wtPath, testArgs);
    if (reverted.timedOut) return { ok: false, reason: 'reverted-run-timed-out' };
    if (reverted.exitCode === 0) return { ok: false, reason: 'test-still-passes-with-source-reverted' };

    // Step 2: restore the source hunks (checkout commit's own version) and confirm green.
    git(wtPath, ['checkout', cand.commit, '--', ...cand.srcFiles]);
    const restored = runCargo(wtPath, testArgs);
    if (restored.timedOut) return { ok: false, reason: 'restored-run-timed-out' };
    if (restored.exitCode !== 0) return { ok: false, reason: 'test-does-not-pass-when-restored' };

    return {
      ok: true,
      task: {
        id: cand.commit.slice(0, 12),
        commit: cand.commit,
        parent,
        srcFiles: cand.srcFiles,
        testFiles: cand.testFiles,
        failingTestCmd: ['cargo', ...testArgs].join(' '),
        description: cand.subj,
      },
    };
  } catch (e) {
    return { ok: false, reason: `verify-error: ${e.message.slice(0, 200)}` };
  } finally {
    try { git(REPO_ROOT, ['worktree', 'remove', '--force', wtPath]); } catch { /* best effort */ }
  }
}

function seededShuffle(arr, seed) {
  const a = [...arr];
  let s = seed;
  const rand = () => { s = (s * 1103515245 + 12345) & 0x7fffffff; return s / 0x7fffffff; };
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

function main() {
  const candidates = findCandidates().slice(0, LIMIT);
  console.log(`[mine-corpus] ${candidates.length} candidate commits (source + dedicated test file, both changed)`);

  const worktreeRoot = mkdtempSync(join(tmpdir(), 'gepa-corpus-'));
  const tasks = [];
  const rejections = [];

  for (const [i, cand] of candidates.entries()) {
    process.stderr.write(`[mine-corpus] ${i + 1}/${candidates.length} ${cand.commit.slice(0, 12)} — ${cand.subj.slice(0, 70)}\n`);
    const result = verifyCandidate(cand, worktreeRoot);
    if (result.ok) {
      tasks.push(result.task);
      process.stderr.write(`  -> KEPT\n`);
    } else {
      rejections.push({ commit: cand.commit, reason: result.reason });
      process.stderr.write(`  -> rejected: ${result.reason}\n`);
    }
  }
  rmSync(worktreeRoot, { recursive: true, force: true });

  console.log(`[mine-corpus] ${tasks.length}/${candidates.length} candidates verified as real revert-tasks`);
  console.log(`[mine-corpus] rejections: ${JSON.stringify(rejections.reduce((m, r) => { m[r.reason] = (m[r.reason] || 0) + 1; return m; }, {}), null, 2)}`);

  if (tasks.length < 8) {
    console.error(`[mine-corpus] GATE 1 FAILED: only ${tasks.length} verified tasks (need >= 8). Stopping -- not writing a manifest.`);
    process.exit(1);
  }

  const shuffled = seededShuffle(tasks, 42);
  const splitAt = Math.round(shuffled.length * 0.7);
  const train = shuffled.slice(0, splitAt).map((t) => t.id);
  const holdout = shuffled.slice(splitAt).map((t) => t.id);

  const manifestBody = { tasks, train, holdout };
  const bodyJson = JSON.stringify(manifestBody, null, 2);
  const sha256 = createHash('sha256').update(bodyJson).digest('hex');

  const manifest = { ...manifestBody, sha256, generatedAt: new Date(0).toISOString().replace('1970-01-01T00:00:00.000Z', '<set-by-caller>') };
  // Deterministic content; timestamp intentionally omitted from the hash input above.
  const outPath = join(import.meta.dirname, 'corpus', 'manifest.json');
  writeFileSync(outPath, JSON.stringify({ ...manifestBody, sha256 }, null, 2) + '\n');
  console.log(`[mine-corpus] GATE 1 PASSED: wrote ${outPath}`);
  console.log(`[mine-corpus] train=${train.length} holdout=${holdout.length} sha256=${sha256}`);
}

main();
