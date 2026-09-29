import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { createDeliveryCandidate } from '../src/delivery-candidate.js';
import { deliveryPoolIdentity, runDeliveryPool } from '../src/delivery-pool.js';
import { git } from '../src/delivery-workspace.js';
import type { DeliveryHarness } from '../src/delivery-runtime.js';
import type { DeliveryTask } from '../src/delivery-contracts.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
function parentDirectory() { const root = mkdtempSync(join(tmpdir(), 'delivery-candidates-')); roots.push(root); return root; }
async function attest(harness: DeliveryHarness, task: DeliveryTask, edit?: () => void) {
  await harness.begin(task); await harness.bind(task.id, task.owner, native);
  let action = await harness.advance(task.id, task.owner);
  if (action.kind !== 'native') throw new Error('implementation required');
  edit?.();
  await harness.submit(task.id, task.owner, { schemaVersion: 1, requestId: action.request.id,
    sourceDigest: harness.snapshot().digest, native, outcome: 'completed', summary: 'Injected deterministic fixture', issues: [] });
  action = await harness.advance(task.id, task.owner);
  if (action.kind !== 'native' || action.request.stage !== 'review') throw new Error('review required');
  await harness.submit(task.id, task.owner, { schemaVersion: 1, requestId: action.request.id,
    sourceDigest: harness.snapshot().digest, native: { ...native, executorId: 'fixture-fresh-reviewer' },
    outcome: 'completed', summary: 'Injected fresh review fixture', issues: [] });
  return harness.verify(task.id, task.owner);
}

describe('upstream pool and ordinary lifecycle candidate roots', () => {
  it('rejects canonical source-visible parents before copying, including symlink aliases', () => {
    const f = workflowFixture(roots), directory = parentDirectory();
    const visible = join(f.root, 'candidates'); mkdirSync(visible);
    writeFileSync(join(f.root, '.gitignore'), '.metaharness/\ncandidates/delivery-candidate-probe\n');
    const alias = join(directory, 'alias'); symlinkSync(visible, alias, 'dir');
    const before = f.harness.snapshot().digest;
    for (const parent of [f.root, visible, alias]) {
      expect(() => createDeliveryCandidate(f.harness, { parentDirectory: parent, scope: ['product.txt'] })).toThrow('PARENT_NOT_ISOLATED');
    }
    expect(readdirSync(visible)).toEqual([]);
    expect(f.harness.snapshot().digest).toBe(before);
  });
  it('requires exact-file scopes and excludes ignored runtime trees', () => {
    const f = workflowFixture(roots), directory = parentDirectory();
    mkdirSync(join(f.root, 'src')); writeFileSync(join(f.root, 'src/input.txt'), 'input\n');
    mkdirSync(join(f.root, 'coding-harness/dist'), { recursive: true });
    writeFileSync(join(f.root, '.gitignore'), '.metaharness/\ncoding-harness/dist/\n');
    writeFileSync(join(f.root, 'coding-harness/dist/runtime.js'), 'ignored runtime');
    expect(() => createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['src'] })).toThrow('EXACT_FILE_SCOPE_REQUIRED');
    const candidate = createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['src/input.txt'] });
    try {
      expect(statSync(join(candidate.harness.root, 'src')).mode & 0o700).toBe(0o700);
      expect(statSync(join(candidate.harness.root, 'src/input.txt')).mode & 0o600).toBe(0o600);
      expect(() => statSync(join(candidate.harness.root, 'coding-harness/dist'))).toThrow();
      expect(candidate.harness.snapshot().digest).toBe(candidate.sourceBefore.digest);
      writeFileSync(join(candidate.harness.root, 'src/input.txt'), 'changed\n');
      expect(candidate.harness.snapshot().digest).not.toBe(candidate.sourceBefore.digest);
    } finally { candidate.cleanup(); }
  });
  it('detects cooperative out-of-scope replacement through the ordinary lifecycle', async () => {
    const f = workflowFixture(roots), candidate = createDeliveryCandidate(f.harness, { parentDirectory: parentDirectory(), scope: ['product.txt'] });
    try {
      await candidate.harness.begin(f.task); await candidate.harness.bind(f.task.id, f.task.owner, native);
      const check = join(candidate.harness.root, 'coding-harness/check.mjs');
      unlinkSync(check); writeFileSync(check, 'process.exit(1);\n');
      await expect(candidate.harness.next(f.task.id, f.task.owner)).rejects.toThrow('OUT_OF_SCOPE_CHANGE');
    } finally { candidate.cleanup(); }
  });
  it('refuses out-of-scope changes before begin and tracked runtime artifacts before copying', async () => {
    const f = workflowFixture(roots), directory = parentDirectory();
    const candidate = createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['product.txt'] });
    try {
      const check = join(candidate.harness.root, 'coding-harness/check.mjs');
      unlinkSync(check); writeFileSync(check, 'process.exit(1);\n');
      await expect(candidate.harness.begin(f.task)).rejects.toThrow('OUT_OF_SCOPE_CHANGE');
      expect(candidate.harness.inspect().active).toBeNull();
    } finally { candidate.cleanup(); }
    mkdirSync(join(f.root, 'target')); writeFileSync(join(f.root, 'target/source.txt'), 'tracked artifact\n');
    git(f.root, 'add', 'target/source.txt'); git(f.root, 'commit', '-qm', 'artifact source fixture');
    expect(() => createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['product.txt'] })).toThrow('TRACKED_ARTIFACT_SOURCE_REFUSED');
    expect(readdirSync(directory)).toEqual([]);
  });
  it('does not start callbacks when cancellation arrives before their dispatch microtask', async () => {
    const f = workflowFixture(roots), controller = new AbortController(); let called = 0;
    const pending = runDeliveryPool(f.harness, [{ id: 'cancelled-before-dispatch', mutationPaths: [], resources: [],
      run: async () => { called++; } }], { maxConcurrency: 1, signal: controller.signal });
    controller.abort(new Error('stop before dispatch'));
    const result = await pending;
    expect(called).toBe(0); expect(result.results[0].status).toBe('cancelled');
  });
  it('overlaps two independent complete lifecycle callbacks without writing canonical source', async () => {
    const f = workflowFixture(roots), directory = parentDirectory();
    writeFileSync(join(f.root, 'empty.txt'), '');
    writeFileSync(join(f.root, 'other.txt'), 'other\n'); git(f.root, 'add', 'other.txt'); git(f.root, 'commit', '-qm', 'other input');
    let started = 0; let release!: () => void;
    const bothStarted = new Promise<void>(resolve => { release = resolve; });
    const events: string[] = [], before = f.harness.snapshot().digest;
    const result = await runDeliveryPool(f.harness, ['product.txt', 'other.txt'].map((path, index) => ({
      id: `candidate-${index}`, mutationPaths: [path], resources: [`private-check-${index}`], run: async (_signal, record) => {
        const candidate = createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: [path] });
        record(candidate);
        try {
          events.push(`start-${index}`); if (++started === 2) release(); await bothStarted;
          const task = { ...f.task, id: `task-${index}`, scope: [path] };
          const run = await attest(candidate.harness, task, () => writeFileSync(join(candidate.harness.root, path), 'fixed\n'));
          expect(run.verdict?.pass).toBe(true);
          expect(run.workflow!.results.every(stage => stage.kernel.receiptsValid)).toBe(true);
          await expect(candidate.harness.finish(task.id, task.owner, run.baseCommit)).rejects.toThrow('CANDIDATE_CANNOT_COMMIT');
          events.push(`end-${index}`); return { passed: true, source: candidate.harness.snapshot().digest };
        } finally { candidate.cleanup(); }
      },
    })), { maxConcurrency: 2 });
    expect(result.peakConcurrency).toBe(2);
    expect(deliveryPoolIdentity().package).toBe('@claude-flow/cli');
    expect(deliveryPoolIdentity().sha256).toMatch(/^[a-f0-9]{64}$/);
    expect(events.slice(0, 2).sort()).toEqual(['start-0', 'start-1']);
    expect(result.results.every(result => result.status === 'fulfilled' && result.value?.passed)).toBe(true);
    expect(result.results.every(result => result.candidateRoot && result.evidenceDirectory)).toBe(true);
    expect(f.harness.snapshot().digest).toBe(before); expect(f.harness.inspect()).toEqual({ active: null, operation: null });
  });
  it('refuses overlapping paths, shared resources and a pre-aborted signal before callbacks', async () => {
    const f = workflowFixture(roots); let called = 0;
    const run = async () => { called++; };
    for (const second of [{ mutationPaths: ['src/file.ts'], resources: [] }, { mutationPaths: ['other'], resources: ['db'] }]) {
      await expect(runDeliveryPool(f.harness, [{ id: 'one', mutationPaths: ['src'], resources: ['db'], run },
        { id: 'two', ...second, run }], { maxConcurrency: 2 })).rejects.toThrow('RESOURCE_CONFLICT');
    }
    const controller = new AbortController(); controller.abort(new Error('stop'));
    await expect(runDeliveryPool(f.harness, [{ id: 'one', mutationPaths: [], resources: [], run }],
      { maxConcurrency: 1, signal: controller.signal })).rejects.toThrow('stop');
    expect(called).toBe(0);
  });
  it('holds cohort ownership until cancelled noncooperative callbacks actually settle', async () => {
    const f = workflowFixture(roots), controller = new AbortController();
    let start!: () => void, stop!: () => void;
    const began = new Promise<void>(resolve => { start = resolve; });
    const end = new Promise<void>(resolve => { stop = resolve; });
    let returned = false;
    const running = runDeliveryPool(f.harness, [{ id: 'slow', mutationPaths: ['product.txt'], resources: ['db'],
      run: async () => { start(); await end; } }], { maxConcurrency: 1, signal: controller.signal }).then(result => { returned = true; return result; });
    await began; controller.abort(); await new Promise(resolve => setImmediate(resolve));
    expect(returned).toBe(false);
    await expect(runDeliveryPool(f.harness, [{ id: 'conflict', mutationPaths: ['product.txt'], resources: [],
      run: async () => {} }], { maxConcurrency: 1 })).rejects.toThrow('RESOURCE_CONFLICT');
    stop(); const result = await running;
    expect(result.results[0].status).toBe('cancelled'); expect(f.harness.inspect().operation).toBeNull();
  });
  it('releases child source only after exact parent integration and refuses later drift', async () => {
    const f = workflowFixture(roots), directory = parentDirectory();
    writeFileSync(join(f.root, '.gitattributes'), 'product.txt text eol=lf\n');
    git(f.root, 'add', '.gitattributes'); git(f.root, 'commit', '-qm', 'accepted Git filter policy');
    await attest(f.harness, f.task, () => writeFileSync(join(f.root, 'product.txt'), 'accepted parent\r\n'));
    expect(() => createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['product.txt'], acceptedParent: f.task.id })).toThrow('ACCEPTED_MAIN_SOURCE_REQUIRED');
    git(f.root, 'add', 'product.txt'); git(f.root, 'commit', '-qm', 'integrated parent');
    await f.harness.finish(f.task.id, f.task.owner, git(f.root, 'rev-parse', 'HEAD'));
    writeFileSync(join(f.root, 'unrelated.txt'), 'unrelated integrated outcome\n');
    git(f.root, 'add', 'unrelated.txt'); git(f.root, 'commit', '-qm', 'unrelated integration');
    const candidate = createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['product.txt'], acceptedParent: f.task.id });
    try {
      expect(readFileSync(join(candidate.harness.root, 'product.txt'), 'utf8')).toBe('accepted parent\r\n');
      writeFileSync(join(f.root, 'product.txt'), 'unexpected canonical drift\n');
      expect(() => candidate.harness.snapshot()).not.toThrow();
      expect(() => createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['product.txt'], acceptedParent: f.task.id })).toThrow('ACCEPTED_INPUT_CHANGED');
    } finally { candidate.cleanup(); }
  });
  it('retains failed callback evidence and rejects artifact mutation scope', async () => {
    const f = workflowFixture(roots), directory = parentDirectory();
    expect(() => createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['coding-harness/dist/worker.js'] })).toThrow('ARTIFACT_SCOPE_REFUSED');
    let candidate: ReturnType<typeof createDeliveryCandidate> | undefined;
    const result = await runDeliveryPool(f.harness, [{ id: 'failed', mutationPaths: ['product.txt'], resources: [],
      run: async (_signal, record) => {
        candidate = createDeliveryCandidate(f.harness, { parentDirectory: directory, scope: ['product.txt'] });
        record(candidate); throw new Error('actual callback failure');
      } }], { maxConcurrency: 1 });
    try {
      expect(result.results[0]).toMatchObject({ status: 'rejected', error: 'actual callback failure',
        candidateRoot: candidate!.harness.root, evidenceDirectory: candidate!.harness.directory });
    } finally { candidate?.cleanup(); }
  });
});
