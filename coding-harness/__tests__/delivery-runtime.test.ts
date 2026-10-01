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
import { buildCheckEnvironment, checkEnvironmentEvidence } from '../src/delivery-process.js';
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
const binding = { host: 'codex' as const, model: 'gpt-6.1-sol', effort: 'high' as const,
  executorId: 'native-sol', authentication: 'native-subscription' as const, observation: 'native agent metadata' };
async function started() {
  const f = fixture(); await f.harness.begin(f.task); await f.harness.bind(f.task.id, 'root', binding); return f;
}
async function checks(harness: DeliveryHarness, id = 'task-1') {
  const before = await harness.next(id, 'root');
  if (before.kind === 'native' && before.request.stage === 'implementation') {
    await harness.submit(id, 'root', responseFor(harness, before.request));
  }
  await harness.check(id, 'root', 'build'); await harness.check(id, 'root', 'public');
  const after = await harness.next(id, 'root');
  if (after.kind === 'native' && after.request.stage === 'review') {
    await harness.submit(id, 'root', responseFor(harness, after.request));
  }
}

describe('mandatory main-only delivery harness', () => {
  it('routes ordinary work through Sol 6.1/high and forwards explicit max/ultra without clamping', () => {
    const { task } = fixture();
    expect(selectDeliveryRoute(task)).toEqual({ host: 'codex', model: 'gpt-6.1-sol', effort: 'high' });
    expect(selectDeliveryRoute({ ...task, taskClass: 'pattern' }).model).toBe('gpt-6.1-sol');
    expect(selectDeliveryRoute({ ...task, taskClass: 'mechanical' }).effort).toBe('high');
    expect(selectDeliveryRoute({ ...task, taskClass: 'correctness' }).effort).toBe('high');
    expect(selectDeliveryRoute({ ...task, host: 'claude-code' })).toEqual({ host: 'claude-code', model: 'cc/claude-sonnet-5-5[1m]', effort: 'medium' });
    const { host: _host, ...ordinary } = task;
    expect(selectDeliveryRoute(parseDeliveryTask(ordinary))).toEqual({ host: 'codex', model: 'gpt-6.1-sol', effort: 'high' });
    expect(selectDeliveryRoute({ ...task, taskClass: 'difficult', selectionReason: 'named unresolved proof' })).toEqual({ host: 'codex', model: 'gpt-6.1-sol', effort: 'high' });
    expect(selectDeliveryRoute(parseDeliveryTask({ ...ordinary, host: 'openrouter' })).model).toBe('deepseek/deepseek-v4.1-flash');
    const claude = { ...task, host: 'claude-code' as const, selectionReason: 'main session' };
    expect(selectDeliveryRoute({ ...claude, requested: { host: 'claude-code', model: 'opus', effort: 'xhigh' } }).effort).toBe('xhigh');
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
      { ...task, checks: [{ ...task.checks[0], argv: ['cargo', 'publish'] }] },
      { ...task, checks: [{ ...task.checks[0], argv: ['cargo', 'nextest', 'archive'] }] },
      { ...task, checks: [{ ...task.checks[0], argv: ['cargo', 'nextest'] }] }]) {
      expect(() => parseDeliveryTask(bad)).toThrow();
    }
    const nextest = ['cargo', 'nextest', 'run', '--locked', '-p', 'sf-serve', '--lib'];
    const cargoCheck = { ...task.checks[1], argv: nextest, cwd: '.' };
    expect(parseDeliveryTask({ ...task, checks: [task.checks[0], cargoCheck] }).checks[1].argv).toEqual(nextest);
  });
  it('blocks a second writer and checks before matching native subscription handoff', async () => {
    const { task, harness } = fixture(); await harness.begin(task);
    await expect(harness.begin({ ...task, id: 'task-2' })).rejects.toThrow('WRITER_ALREADY');
    await expect(harness.check(task.id, 'root', 'build')).rejects.toThrow('NATIVE_HANDOFF');
    await expect(harness.bind(task.id, 'root', { ...binding, effort: 'medium' })).rejects.toThrow('ROUTE_MISMATCH');
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
    vi.stubEnv('CARGO_PROFILE_DEV_DEBUG', '2'); vi.stubEnv('CARGO_PROFILE_TEST_INCREMENTAL', 'true');
    vi.stubEnv('CARGO_PROFILE_RELEASE_DEBUG', '2'); vi.stubEnv('CARGO_INCREMENTAL', '0');
    const env = buildCheckEnvironment();
    expect(env.RUSTFLAGS).toBe('-C debuginfo=1'); expect(env.SF_TEST_MARKER).toBe('owned-fixture');
    expect(env).toMatchObject({ CARGO_PROFILE_DEV_DEBUG: '1', CARGO_PROFILE_TEST_DEBUG: '1',
      CARGO_PROFILE_DEV_INCREMENTAL: 'false', CARGO_PROFILE_TEST_INCREMENTAL: 'false', CARGO_INCREMENTAL: '0' });
    expect(env).not.toHaveProperty('CARGO_PROFILE_RELEASE_DEBUG');
    expect(env).not.toHaveProperty('CODEX_HOME'); expect(env).not.toHaveProperty('ANTHROPIC_API_KEY');
    const { harness } = await started(); await checks(harness);
    const currentPath = process.env.PATH!;
    vi.stubEnv('PATH', `/home/test/.codex/tmp/arg0/codex-arg0ABC:${currentPath}:${currentPath}`);
    expect(checkEnvironmentEvidence({ PATH: process.env.PATH }).PATH).toBe(checkEnvironmentEvidence({ PATH: currentPath }).PATH);
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(true);
    vi.stubEnv('PATH', `/meaningful/toolchain/v2:${currentPath}`);
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
    vi.stubEnv('PATH', currentPath);
    vi.stubEnv('RUSTFLAGS', '-C debuginfo=2');
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
  });
  it('terminally supersedes a paused run with a verified descendant while preserving its evidence', async () => {
    const { root, task, harness } = await started();
    await harness.check(task.id, 'root', 'build');
    await harness.pause(task.id, 'root', 'superseded implementation attempt');
    const before = structuredClone(harness.read(task.id));
    const successor = { ...task, id: 'task-2' };
    await harness.begin(successor); await harness.bind(successor.id, 'root', binding);
    writeFileSync(join(root, 'product.txt'), 'after\n'); await checks(harness, successor.id);
    git(root, 'add', 'product.txt'); git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'successor');
    const commit = git(root, 'rev-parse', 'HEAD'); await harness.finish(successor.id, 'root', commit);
    vi.spyOn(console, 'log').mockImplementation(() => {});
    expect(await deliveryCli([root, 'supersede', task.id, 'root', successor.id, 'closed by verified successor'])).toBe(true);
    const resolved = harness.read(task.id);
    expect(resolved.status).toBe('superseded');
    expect(resolved.resolution).toMatchObject({ successorTask: successor.id, successorCommit: commit,
      reason: 'closed by verified successor' });
    expect(resolved.checks).toEqual(before.checks); expect(resolved.handoffs).toEqual(before.handoffs);
    expect(resolved.workflow).toEqual(before.workflow); expect(resolved.verdict).toEqual(before.verdict);
    expect(resolved.events.slice(0, -1)).toEqual(before.events);
    await expect(harness.resume(task.id, 'root')).rejects.toThrow('PAUSED_OWNER_REQUIRED');
    expect(await deliveryCli([root, 'status', task.id])).toBe(true);
    expect(JSON.parse(vi.mocked(console.log).mock.calls.at(-1)![0]).status).toBe('superseded');
    await expect(harness.next(task.id, 'root')).rejects.toThrow('WRITER_MISMATCH');
    writeFileSync(join(root, '.metaharness/delivery/active.json'), JSON.stringify({ id: task.id }));
    expect((await harness.reconcile(task.id, 'root', 'none', 'interrupted cleanup')).status).toBe('superseded');
    expect(harness.inspect().active).toBeNull();
  });
  it('rejects unverified and non-descendant supersession claims', async () => {
    const { root, task, harness } = await started();
    await expect(harness.supersede(task.id, 'root', task.id, 'invalid')).rejects.toThrow('PAUSED_RUN_REQUIRED');
    await harness.pause(task.id, 'root', 'paused');
    const incomplete = { ...task, id: 'task-2' };
    await harness.begin(incomplete); await harness.pause(incomplete.id, 'root', 'also paused');
    await expect(harness.supersede(task.id, 'root', incomplete.id, 'invalid')).rejects.toThrow('COMPLETE_SUCCESSOR_REQUIRED');

    await harness.resume(incomplete.id, 'root'); await harness.bind(incomplete.id, 'root', binding);
    writeFileSync(join(root, 'product.txt'), 'successor\n'); await checks(harness, incomplete.id);
    git(root, 'add', 'product.txt'); git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'successor');
    await harness.finish(incomplete.id, 'root', git(root, 'rev-parse', 'HEAD'));
    const sameBase = { ...task, id: 'same-base' };
    await harness.begin(sameBase); await harness.pause(sameBase.id, 'root', 'new attempt at successor commit');
    await expect(harness.supersede(sameBase.id, 'root', incomplete.id, 'no progress')).rejects.toThrow('SUCCESSOR_DESCENDANT_REQUIRED');
    const read = harness.read.bind(harness);
    const spy = vi.spyOn(harness, 'read').mockImplementation(id => {
      const record = read(id);
      return id === incomplete.id ? { ...record, commit: 'f'.repeat(40) } : record;
    });
    await expect(harness.supersede(task.id, 'root', incomplete.id, 'missing object')).rejects.toThrow('SUCCESSOR_COMMIT_REQUIRED');
    spy.mockRestore();
    writeFileSync(join(root, 'unrelated.txt'), 'later\n'); git(root, 'add', 'unrelated.txt');
    git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'later base');
    const later = { ...task, id: 'task-3' };
    await harness.begin(later); await harness.pause(later.id, 'root', 'later paused attempt');
    await expect(harness.supersede(later.id, 'root', incomplete.id, 'older result')).rejects.toThrow('SUCCESSOR_DESCENDANT_REQUIRED');
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
