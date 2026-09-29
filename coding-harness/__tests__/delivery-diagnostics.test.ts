import { createHash } from 'node:crypto';
import { linkSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { hash } from '@metaharness/harness';
import { repairDiagnosticContext } from '../src/delivery-diagnostics.js';
import type { DeliveryHarness } from '../src/delivery-runtime.js';
import type { NativeStageRequest } from '../src/delivery-workflow-contracts.js';

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
const digest = (text: string) => createHash('sha256').update(text).digest('hex');
function fixture(text = 'error[E0308]: expected &Router, found &RequestDeadlineService\n') {
  const directory = mkdtempSync(join(tmpdir(), 'sf-diagnostics-')); roots.push(directory);
  const stdout = join(directory, 'task-build-1.stdout'), stderr = join(directory, 'task-build-1.stderr');
  writeFileSync(stdout, 'assertion left != right\n'); writeFileSync(stderr, text);
  const check = { id: 'build', attempt: 1, passed: false, sourceBefore: 'source', sourceAfter: 'source',
    stdout, stderr, stdoutDigest: digest('assertion left != right\n'), stderrDigest: digest(text) };
  const run = { task: { id: 'task', checks: [{ id: 'build' }] }, checks: [check] };
  const harness = { directory, read: () => run } as unknown as DeliveryHarness;
  const request = { taskId: 'task', stage: 'implementation', repair: true, sourceDigest: 'source', prerequisiteDigests: [hash(check)] } as NativeStageRequest;
  const context = () => repairDiagnosticContext(harness, request, {}) as { diagnosticData: {
    stderr: { status: string; text?: string; truncated?: boolean }; stdout: { text?: string } } }[];
  return { directory, check, run, harness, request, context, text };
}

it('adds verified diagnostic bytes without modifying historical logs or requests', () => {
  const f = fixture(), before = JSON.stringify([f.request, f.run]);
  const data = f.context()[0]!.diagnosticData;
  expect(data.stderr.status).toBe('verified');
  expect(data.stderr.text).toBe(f.text);
  expect(data.stdout.text).toContain('assertion left != right');
  expect(JSON.stringify([f.request, f.run])).toBe(before);
  expect(readFileSync(f.check.stderr, 'utf8')).toBe(f.text);
});

it.each(['review', 'architecture', 'ordinary', 'stale', 'changed-source', 'passed', 'superseded'])('omits diagnostics for %s', mode => {
  const f = fixture();
  if (mode === 'review' || mode === 'architecture') f.request.stage = mode;
  if (mode === 'ordinary') f.request.repair = false;
  if (mode === 'stale') f.request.sourceDigest = 'other';
  if (mode === 'changed-source') f.check.sourceAfter = 'other';
  if (mode === 'passed') f.check.passed = true;
  if (mode === 'superseded') f.run.checks.push({ ...f.check, attempt: 2, passed: true });
  expect(f.context()).toEqual([]);
});

it.each(['digest', 'external', 'missing', 'symlink', 'hardlink', 'directory', 'oversized', 'parent-symlink'])('withholds %s without disclosing bytes or paths', mode => {
  const f = fixture('private log content');
  if (mode === 'digest') f.check.stderrDigest = '0'.repeat(64);
  if (mode === 'external') f.check.stderr = join(f.directory, '..', 'external-secret');
  if (mode === 'oversized') { writeFileSync(f.check.stderr, 'x'.repeat(10_000_001)); f.check.stderrDigest = digest('x'.repeat(10_000_001)); }
  if (['missing', 'symlink', 'hardlink', 'directory'].includes(mode)) {
    rmSync(f.check.stderr);
    const other = join(f.directory, 'other'); writeFileSync(other, f.text);
    if (mode === 'symlink') symlinkSync(other, f.check.stderr);
    if (mode === 'hardlink') linkSync(other, f.check.stderr);
    if (mode === 'directory') mkdirSync(f.check.stderr);
  }
  if (mode === 'parent-symlink') {
    const alias = join(f.directory, 'alias'); symlinkSync(f.directory, alias);
    (f.harness as unknown as { directory: string }).directory = alias;
    f.check.stderr = join(alias, 'task-build-1.stderr');
  }
  f.request.prerequisiteDigests = [hash(f.check)];
  const data = f.context()[0]!.diagnosticData.stderr;
  expect(data.status).toBe('withheld');
  expect(data).not.toHaveProperty('text');
  expect(JSON.stringify(data)).not.toContain(f.directory);
});

it('redacts before bounded head/tail extraction, preserves useful compiler and assertion lines', () => {
  const secret = 'known-environment-secret';
  const f = fixture('head error[E0308]\n' + 'x'.repeat(4096 - 'head error[E0308]\n'.length - 10) + secret + '\n'
    + 'y'.repeat(15000) + '\nAuthorization: Bearer unseen\nPASSWORD=unseen-password\n'
    + 'https://user:pass@example.invalid\n-----BEGIN PRIVATE KEY-----\nkey material\n-----END PRIVATE KEY-----\n'
    + '\u001b[31massertion tail\u001b[0m\n');
  const all = repairDiagnosticContext(f.harness, f.request, { API_KEY: secret });
  const text = JSON.stringify(all);
  for (const forbidden of [secret, 'unseen', 'user:pass', 'key material', '\\u001b']) expect(text).not.toContain(forbidden);
  expect(text).toContain('head error[E0308]'); expect(text).toContain('assertion tail');
  expect(text).toContain('diagnostic middle omitted'); expect(text.length).toBeLessThan(13500);
  expect(text).not.toContain('known-envi');
});

it('binds diagnostic selection to exact failed-check prerequisite', () => {
  const f = fixture();
  f.run.task.checks.push({ id: 'public' });
  f.run.checks.push({ ...f.check, id: 'public' });
  expect(f.context()[0]!.diagnosticData.stderr.status).toBe('verified');
  f.request.prerequisiteDigests = ['unrelated'];
  expect(f.context()).toEqual([]);
});

it('redacts an environment secret crossing the tail boundary', () => {
  const secret = 'tail-boundary-secret-value';
  const f = fixture('x'.repeat(16000) + secret + 'z'.repeat(8192 - 10));
  const text = JSON.stringify(repairDiagnosticContext(f.harness, f.request, { AUTH_TOKEN: secret }));
  expect(text).not.toContain('cret-value');
  expect(text).not.toContain(secret);
});

it('handles adversarial long tokens and URL punctuation without quadratic scanning', () => {
  const f = fixture('a-'.repeat(500000) + '\nhttp://' + ':'.repeat(1000000) + '\n' + 'token'.repeat(200000));
  const start = performance.now();
  expect(f.context()[0]!.diagnosticData.stderr.status).toBe('verified');
  expect(performance.now() - start).toBeLessThan(2000);
}, 5000);

it.each(['\x1b[0m', '\x00'])('normalizes controls before all secret redaction (%j)', control => {
  const secret = 'environment-secret-value';
  const f = fixture(`environment-${control}secret-value\ntok${control}en=assignment-value\n`
    + `Authorization${control}: Bearer header-value\n-----BEGIN PRIV${control}ATE KEY-----\nkey-value\n-----END PRIVATE KEY-----`);
  const text = JSON.stringify(repairDiagnosticContext(f.harness, f.request, { AUTH_TOKEN: secret }));
  for (const value of [secret, 'assignment-value', 'header-value', 'key-value']) expect(text).not.toContain(value);
});

it('redacts empty usernames and entire userinfo through its last at-sign', () => {
  const f = fixture(['https', 'postgresql', 'redis', 'rediss', 'amqp', 'amqps', 'mongodb', 'mongodb+srv', 'ftp', 'ssh']
    .flatMap(scheme => [`${scheme}://:empty-user-password@host/path`, `${scheme}://user:pass@word@host/path`]).join('\n'));
  const text = JSON.stringify(f.context());
  expect(text).not.toContain('empty-user-password'); expect(text).not.toContain('word@host');
  expect(text).not.toContain('user:pass');
});
