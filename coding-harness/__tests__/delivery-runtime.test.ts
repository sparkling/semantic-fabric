// SPDX-License-Identifier: MIT
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DeliveryHarness } from '../src/delivery-runtime.js';
import { parseDeliveryTask, selectDeliveryRoute, type DeliveryTask } from '../src/delivery-contracts.js';
import { git, withOperationLock } from '../src/delivery-workspace.js';
import { deliveryCli } from '../src/delivery-cli.js';
import { buildCheckEnvironment } from '../src/delivery-process.js';
import { responseFor } from './delivery-workflow-fixtures.js';

const roots: string[] = [];
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllEnvs(); for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'sf-delivery-test-')); roots.push(root);
  execFileSync('git', ['init', '-b', 'main', root], { stdio: 'ignore' });
  git(root, 'config', 'user.name', 'Harness test'); git(root, 'config', 'user.email', 'test@example.invalid');
  mkdirSync(join(root, 'coding-harness'));
  writeFileSync(join(root, '.gitignore'), '.metaharness/\n');
  writeFileSync(join(root, 'coding-harness/check.mjs'), 'process.exit(0);\n');
  writeFileSync(join(root, 'product.txt'), 'before\n');
  writeFileSync(join(root, 'unrelated.txt'), 'preserve\n');
  git(root, 'add', '.'); git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'initial');
  const task: DeliveryTask = { schemaVersion: 1, id: 'task-1', owner: 'root', thread: 'test-thread',
    requirement: 'Preserve exact public behavior', taskClass: 'implementation', host: 'codex',
    scope: ['product.txt'], checks: [
      { id: 'build', kind: 'build', argv: ['node', 'check.mjs'], cwd: 'coding-harness' },
      { id: 'public', kind: 'acceptance', argv: ['node', 'check.mjs'], cwd: 'coding-harness' },
    ] };
  return { root, task, harness: new DeliveryHarness(root) };
}
const binding = { host: 'codex' as const, model: 'gpt-5.6-sol', effort: 'medium' as const,
  executorId: 'native-sol', authentication: 'native-subscription' as const, observation: 'native agent metadata' };
async function started() {
  const f = fixture(); await f.harness.begin(f.task); await f.harness.bind(f.task.id, 'root', binding); return f;
}
async function checks(harness: DeliveryHarness) {
  const before = await harness.next('task-1', 'root');
  if (before.kind === 'native' && before.request.stage === 'implementation') {
    await harness.submit('task-1', 'root', responseFor(harness, before.request));
  }
  await harness.check('task-1', 'root', 'build'); await harness.check('task-1', 'root', 'public');
  const after = await harness.next('task-1', 'root');
  if (after.kind === 'native' && after.request.stage === 'review') {
    await harness.submit('task-1', 'root', responseFor(harness, after.request));
  }
}

describe('mandatory main-only delivery harness', () => {
  it('routes faster models by task and forwards explicit max/ultra without clamping', () => {
    const { task } = fixture();
    expect(selectDeliveryRoute(task)).toEqual({ host: 'codex', model: 'gpt-5.6-sol', effort: 'medium' });
    expect(selectDeliveryRoute({ ...task, taskClass: 'pattern' }).model).toBe('gpt-5.6-terra');
    expect(selectDeliveryRoute({ ...task, taskClass: 'mechanical' }).effort).toBe('low');
    expect(selectDeliveryRoute({ ...task, taskClass: 'correctness' }).effort).toBe('high');
    expect(selectDeliveryRoute({ ...task, host: 'claude-code' })).toEqual({ host: 'claude-code', model: 'sonnet', effort: 'default' });
    const claude = { ...task, host: 'claude-code' as const, selectionReason: 'main session' };
    expect(selectDeliveryRoute({ ...claude, requested: { host: 'claude-code', model: 'fable', effort: 'xhigh' } }).effort).toBe('xhigh');
    expect(() => selectDeliveryRoute({ ...claude, requested: { host: 'claude-code', model: 'opus', effort: 'ultra' }, explicitUltra: true })).toThrow('EFFORT_NOT_NATIVE');
    expect(() => selectDeliveryRoute({ ...task, preserveMainModel: true })).toThrow('CURRENT_MAIN_ROUTE_REQUIRED');
    const explicit = { ...task, requested: { host: 'codex' as const, model: 'gpt-6-astra', effort: 'ultra' as const }, selectionReason: 'named problem' };
    expect(() => selectDeliveryRoute(explicit)).toThrow('EXPLICIT_REQUEST');
    expect(selectDeliveryRoute({ ...explicit, explicitUltra: true }).effort).toBe('ultra');
    expect(selectDeliveryRoute({ ...explicit, preserveMainModel: true }).effort).toBe('ultra');
    expect(selectDeliveryRoute({ ...explicit, requested: { ...explicit.requested, effort: 'max' } }).effort).toBe('max');
  });
  it('rejects unbounded task metadata, empty gates, runtime Node and publication commands', () => {
    const { task } = fixture();
    for (const bad of [{ ...task, spendCeiling: 1 }, { ...task, scope: ['../outside'] },
      { ...task, checks: [] }, { ...task, requested: { ...binding, model: 'openrouter/x' } },
      { ...task, checks: [{ ...task.checks[0], cwd: '.' }] },
      { ...task, id: 'active' },
      { ...task, checks: [{ ...task.checks[0], argv: ['npm', 'publish'] }] },
      { ...task, checks: [{ ...task.checks[0], argv: ['cargo', 'yank'] }] },
      { ...task, checks: [{ ...task.checks[0], timeoutMs: 86_400_001 }] },
      { ...task, checks: [{ ...task.checks[0], maxOutputBytes: 100_000_001 }] },
      { ...task, checks: [{ ...task.checks[0], argv: ['cargo', 'publish'] }] }]) {
      expect(() => parseDeliveryTask(bad)).toThrow();
    }
  });
  it('blocks a second writer and checks before matching native subscription handoff', async () => {
    const { task, harness } = fixture(); await harness.begin(task);
    await expect(harness.begin({ ...task, id: 'task-2' })).rejects.toThrow('WRITER_ALREADY');
    await expect(harness.check(task.id, 'root', 'build')).rejects.toThrow('NATIVE_HANDOFF');
    await expect(harness.bind(task.id, 'root', { ...binding, effort: 'high' })).rejects.toThrow('ROUTE_MISMATCH');
    await expect(harness.bind(task.id, 'wrong-owner', binding)).rejects.toThrow('WRITER_MISMATCH');
  });
  it('runs real commands and MetaHarness verification then binds only the scoped main commit', async () => {
    const { root, harness } = await started();
    writeFileSync(join(root, 'product.txt'), 'after\n');
    await checks(harness);
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(true);
    git(root, 'add', 'product.txt'); git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'fix product');
    const result = await harness.finish('task-1', 'root', git(root, 'rev-parse', 'HEAD'));
    expect(result.status).toBe('complete'); expect(result.checks).toHaveLength(2);
    expect(result.checks[0].durationMs).toBeGreaterThanOrEqual(0);
    expect(harness.read('task-1').digest).toBe(result.digest);
  });
  it('invalidates successful checks after edits and never fabricates omitted results', async () => {
    const { root, harness } = await started();
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
    await checks(harness); writeFileSync(join(root, 'product.txt'), 'newer\n');
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
  });
  it('preserves pre-existing unrelated work and refuses newly changed outside files', async () => {
    const { root, task, harness } = fixture(); writeFileSync(join(root, 'unrelated.txt'), 'pending other task\n');
    await harness.begin(task); await harness.bind(task.id, 'root', binding);
    writeFileSync(join(root, 'product.txt'), 'after\n'); await checks(harness);
    git(root, 'add', 'product.txt'); git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'scoped');
    await harness.finish('task-1', 'root', git(root, 'rev-parse', 'HEAD'));
    expect(readFileSync(join(root, 'unrelated.txt'), 'utf8')).toBe('pending other task\n');
    const fresh = await started(); writeFileSync(join(fresh.root, 'unrelated.txt'), 'unexpected\n');
    await expect(fresh.harness.check('task-1', 'root', 'build')).rejects.toThrow('OUT_OF_SCOPE');
  });
  it('records failure, retains it through repair, and detects changed log evidence', async () => {
    const { root, task, harness } = fixture();
    writeFileSync(join(root, 'coding-harness/check.mjs'), "import {readFileSync} from 'node:fs'; process.exit(readFileSync('../product.txt','utf8').includes('fixed') ? 0 : 1);\n");
    await harness.begin(task); await harness.bind(task.id, 'root', binding);
    expect((await harness.check(task.id, 'root', 'build')).checks.at(-1)?.passed).toBe(false);
    writeFileSync(join(root, 'product.txt'), 'fixed\n'); await checks(harness);
    const verified = await harness.verify(task.id, 'root'); expect(verified.verdict?.pass).toBe(true);
    expect(verified.checks[0].passed).toBe(false);
    writeFileSync(verified.checks.at(-1)!.stdout, 'changed');
    expect((await harness.verify(task.id, 'root')).verdict?.pass).toBe(false);
  });
  it('requires a fresh handoff after pause/resume and retains history', async () => {
    const { harness } = await started(); await harness.pause('task-1', 'root', 'native client unavailable: exact error');
    expect((await harness.resume('task-1', 'root')).status).toBe('awaiting-native');
    await expect(harness.check('task-1', 'root', 'build')).rejects.toThrow('NATIVE_HANDOFF');
    const run = await harness.bind('task-1', 'root', binding); expect(run.handoffs).toHaveLength(2);
  });
  it('rejects evidence tampering and commit containing unrelated changes', async () => {
    const { root, harness } = await started(); await checks(harness);
    writeFileSync(join(root, 'unrelated.txt'), 'oops\n');
    git(root, 'add', 'unrelated.txt'); git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'unrelated');
    await expect(harness.finish('task-1', 'root', git(root, 'rev-parse', 'HEAD'))).rejects.toThrow('COMMIT_SCOPE');
    const path = join(root, '.metaharness/delivery/task-1.json'); const run = JSON.parse(readFileSync(path, 'utf8'));
    run.status = 'complete'; writeFileSync(path, JSON.stringify(run));
    expect(() => harness.read('task-1')).toThrow('DIGEST_MISMATCH');
  });
  it('requires explicit adoption of interrupted scoped work and rejects duplicate handoffs', async () => {
    const { root, task, harness } = fixture(); writeFileSync(join(root, 'product.txt'), 'in progress\n');
    await expect(harness.begin(task)).rejects.toThrow('EXPLICIT_ADOPTION');
    const run = await harness.begin({ ...task, adoptExistingChanges: ['product.txt'] });
    expect(run.adoptedSource['product.txt']).toMatch(/^100644:/);
    await harness.bind(task.id, 'root', binding);
    await expect(harness.bind(task.id, 'root', binding)).rejects.toThrow('ALREADY_BOUND');
  });
  it('rejects source mutation by a successful command', async () => {
    const { root, task, harness } = fixture();
    writeFileSync(join(root, 'coding-harness/check.mjs'), "import {writeFileSync} from 'node:fs'; writeFileSync('../product.txt','changed');\n");
    await harness.begin(task); await harness.bind(task.id, 'root', binding);
    const run = await harness.check(task.id, 'root', 'build');
    expect(run.checks[0].exitCode).toBe(0); expect(run.checks[0].passed).toBe(false);
  });
  it('preserves build configuration but excludes model credentials/config and rejects config drift', async () => {
    vi.stubEnv('RUSTFLAGS', '-C debuginfo=1'); vi.stubEnv('SF_TEST_MARKER', 'owned-fixture');
    vi.stubEnv('CODEX_HOME', '/not-a-build-input'); vi.stubEnv('ANTHROPIC_API_KEY', 'test-placeholder');
    const env = buildCheckEnvironment();
    expect(env.RUSTFLAGS).toBe('-C debuginfo=1'); expect(env.SF_TEST_MARKER).toBe('owned-fixture');
    expect(env).not.toHaveProperty('CODEX_HOME'); expect(env).not.toHaveProperty('ANTHROPIC_API_KEY');
    const { harness } = await started(); await checks(harness);
    vi.stubEnv('RUSTFLAGS', '-C debuginfo=2');
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
  });
  it('records timeout and output overflow as failures with bounded logs', async () => {
    for (const mode of ['timeout', 'output']) {
      const { root, task, harness } = fixture();
      writeFileSync(join(root, 'coding-harness/check.mjs'), mode === 'timeout'
        ? 'setInterval(() => {}, 1000);\n' : "process.stdout.write('x'.repeat(10000));\n");
      task.checks[0].timeoutMs = mode === 'timeout' ? 100 : 3000;
      task.checks[0].maxOutputBytes = 128;
      await harness.begin(task); await harness.bind(task.id, 'root', binding);
      const run = await harness.check(task.id, 'root', 'build');
      expect(run.checks[0].passed).toBe(false);
      expect(run.checks[0].error).toBe(mode === 'timeout' ? 'check-timeout' : 'check-output-limit');
      expect(readFileSync(run.checks[0].stdout).length).toBeLessThanOrEqual(128);
    }
  });
  it('serializes checks, refuses live-owner recovery and records cancellation', async () => {
    const { root, task, harness } = fixture();
    writeFileSync(join(root, 'coding-harness/check.mjs'), 'setInterval(() => {},1000);\n');
    await harness.begin(task); await harness.bind(task.id, 'root', binding);
    const abort = new AbortController(); const pending = harness.check(task.id, 'root', 'build', abort.signal);
    try {
      await expect(harness.check(task.id, 'root', 'public')).rejects.toThrow('OPERATION_BUSY');
      const lock = harness.inspect().operation as { nonce: string };
      await expect(harness.reconcile(task.id, 'root', lock.nonce, 'not allowed')).rejects.toThrow('OPERATION_BUSY');
    } finally { abort.abort(); }
    expect((await pending).checks[0].error).toBe('cancelled');
  });
  it('reconciles a crash using exact lock identity, preserves history and requires a new handoff', async () => {
    const { root, harness } = await started();
    unlinkSync(join(root, '.metaharness/delivery/active.json'));
    const nonce = '11111111-1111-1111-1111-111111111111';
    writeFileSync(join(root, '.metaharness/delivery/operation.lock'), JSON.stringify({
      pid: 1073741824, start: '1', nonce, phase: 'idle',
    }));
    await expect(harness.reconcile('task-1', 'root', 'wrong', 'recovery')).rejects.toThrow('LOCK_CHANGED');
    const run = await harness.reconcile('task-1', 'root', nonce, 'dead owner and no child confirmed');
    expect(run.status).toBe('awaiting-native'); expect(run.handoffs).toHaveLength(1);
    expect(run.events.at(-1)?.kind).toBe('reconcile');
    await expect(harness.check('task-1', 'root', 'build')).rejects.toThrow('HANDOFF_REQUIRED');
  });
  it('holds an OS lease independent of the crash-recovery metadata file', async () => {
    const { harness } = fixture(); let release!: () => void;
    const holding = withOperationLock(harness.directory, () => new Promise<void>(r => { release = r; }));
    try {
      unlinkSync(join(harness.directory, 'operation.lock'));
      await expect(withOperationLock(harness.directory, async () => {})).rejects.toThrow('OS lease unavailable');
    } finally { release(); await holding; }
    await expect(withOperationLock(harness.directory, async () => 'released')).resolves.toBe('released');
  });
  it('reconciles a completed record with an interrupted active-claim cleanup', async () => {
    const { root, harness } = await started(); writeFileSync(join(root, 'product.txt'), 'after\n');
    await checks(harness); git(root, 'add', 'product.txt');
    git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'scoped');
    await harness.finish('task-1', 'root', git(root, 'rev-parse', 'HEAD'));
    writeFileSync(join(root, '.metaharness/delivery/active.json'), JSON.stringify({ id: 'task-1' }));
    expect((await harness.reconcile('task-1', 'root', 'none', 'finish interrupted')).status).toBe('complete');
    expect(harness.inspect().active).toBeNull();
  });
  it('exposes the actual CLI lifecycle and returns failure for missing verification', async () => {
    const { root, task } = fixture(); vi.spyOn(console, 'log').mockImplementation(() => {});
    const input = join(root, '.metaharness/task.json'), native = join(root, '.metaharness/native.json');
    writeFileSync(input, JSON.stringify(task)); writeFileSync(native, JSON.stringify(binding));
    expect(await deliveryCli([root, 'begin', input])).toBe(true);
    expect(await deliveryCli([root, 'bind', task.id, 'root', native])).toBe(true);
    expect(await deliveryCli([root, 'verify', task.id, 'root'])).toBe(false);
    expect(await deliveryCli([root, 'check', task.id, 'root', 'build'])).toBe(true);
    expect(await deliveryCli([root, 'status', task.id])).toBe(true);
  });
});
