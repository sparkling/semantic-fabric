// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { parseHarnessManifest } from '../src/manifest.js';
import {
  PROGRAMME_CAPTURE_HARNESS_CONFIG_V1,
  PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1,
} from '../src/programme-capture-config-v1.js';
import {
  attestProgrammeCaptureInputsV1,
  parseProgrammeCaptureInputAttestationBlobV1,
  parseProgrammeCaptureInputAttestationV1,
} from '../src/programme-capture-input-attestation-v1.js';
import {
  PROGRAMME_CAPTURE_OUTPUT_PATH,
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
} from '../src/programme-capture-task-v1.js';
import { digestValue } from '../src/receipts.js';

const TASK_PATH = 'coding-harness/config/programme-v5-acceptance.json';
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme capture V1 scoped configuration', () => {
  it('is non-overridable, runtime-frozen, exact, and separate from global protection', () => {
    const expected = [...new Set([
      PROGRAMME_CAPTURE_PROFILE_PATH,
      PROGRAMME_CAPTURE_SCENARIOS_PATH,
      'Cargo.lock',
      ...PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
    ])].sort(compareUtf8);
    expect(PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1).toEqual(expected);
    expect(PROGRAMME_CAPTURE_HARNESS_CONFIG_V1.requiredProtectedPaths).toEqual(expected);
    expect(PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1).not.toContain(PROGRAMME_CAPTURE_OUTPUT_PATH);
    expect(Object.isFrozen(PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS)).toBe(true);
    expect(Object.isFrozen(PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1)).toBe(true);
    expect(Object.isFrozen(PROGRAMME_CAPTURE_HARNESS_CONFIG_V1)).toBe(true);
    for (const mutate of [
      (paths: string[]) => paths.push('attacker.rs'),
      (paths: string[]) => paths.splice(0, 1),
      (paths: string[]) => paths.reverse(),
      (paths: string[]) => paths.sort(),
    ]) expect(() => mutate(PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS as unknown as string[]))
      .toThrow();

    const manifest = JSON.parse(readFileSync(
      new URL('../.harness/manifest.json', import.meta.url), 'utf8',
    ));
    expect(parseHarnessManifest(manifest, SECURE_HARNESS_CONFIG).name)
      .toBe('semantic-fabric-coding-harness');
    expect(() => parseHarnessManifest(manifest, PROGRAMME_CAPTURE_HARNESS_CONFIG_V1))
      .toThrow('HARNESS_MANIFEST_PROTECTED_PATHS_MISMATCH');
  });
});

describe('programme capture V1 commit-object input attestation', () => {
  it('binds the manifest, parsed task, complete ordered blob closure, and absent output', async () => {
    const fixture = repository();
    const result = await attestProgrammeCaptureInputsV1({
      controllerStore: fixture.root,
      controllerCommit: fixture.commit,
      taskPath: TASK_PATH,
    });

    expect(result.taskBlob).toBe(fixture.taskBlob);
    expect(result.record).toMatchObject({
      schemaVersion: 1,
      transactionKind: 'programme-capture-v1',
      controller: { commit: fixture.commit, tree: fixture.tree },
      output: { path: PROGRAMME_CAPTURE_OUTPUT_PATH, absentFromCommit: true },
    });
    expect(result.record.task.valueDigest).toBe(digestValue(result.task));
    expect(result.record.protectedInputs.map(({ path }) => path)).toEqual([
      PROGRAMME_CAPTURE_PROFILE_PATH,
      PROGRAMME_CAPTURE_SCENARIOS_PATH,
      'Cargo.lock',
      ...PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
    ]);
    expect(result.record.protectedInputs).toHaveLength(124);
    expect(result.record.protectedInputsDigest).toBe(
      digestValue(result.record.protectedInputs),
    );
    expect(result.record.attestationDigest).toMatch(/^[a-f0-9]{64}$/);
    visitObjects(result, (value) => expect(Object.isFrozen(value)).toBe(true));
    expect(parseProgrammeCaptureInputAttestationV1(structuredClone(result.record)))
      .toEqual(result.record);
    expect(parseProgrammeCaptureInputAttestationBlobV1(JSON.stringify(result.record)))
      .toEqual(result.record);

    writeFile(fixture.root, 'unrelated.txt', 'later\n');
    git(fixture.root, ['add', '--', 'unrelated.txt']);
    git(fixture.root, ['commit', '--quiet', '-m', 'advance'], identityEnvironment());
    expect(git(fixture.root, ['rev-parse', 'HEAD']).trim()).not.toBe(fixture.commit);
    expect((await attestProgrammeCaptureInputsV1({
      controllerStore: fixture.root,
      controllerCommit: fixture.commit,
      taskPath: TASK_PATH,
    })).record).toEqual(result.record);

    const bareStore = packedBareRepository(fixture.root);
    expect((await attestProgrammeCaptureInputsV1({
      controllerStore: bareStore,
      controllerCommit: fixture.commit,
      taskPath: TASK_PATH,
    })).record).toEqual(result.record);
  });

  it('supports native SHA-256 object stores', async () => {
    const fixture = repository({ objectFormat: 'sha256' });
    const record = (await attest(fixture)).record;
    expect(record.controller.commit).toHaveLength(64);
    expect(record.controller.tree).toHaveLength(64);
    expect(record.protectedInputs.every(({ gitBlobId }) => gitBlobId.length === 64)).toBe(true);
    expect(parseProgrammeCaptureInputAttestationV1(structuredClone(record))).toEqual(record);
    const bareStore = packedBareRepository(fixture.root);
    expect((await attestProgrammeCaptureInputsV1({
      controllerStore: bareStore, controllerCommit: fixture.commit, taskPath: TASK_PATH,
    })).record).toEqual(record);
  });

  it('rejects inherited or linked controller stores while admitting exact bare roots', async () => {
    const fixture = repository();
    for (const controllerStore of [
      join(fixture.root, 'coding-harness'),
      join(fixture.root, '.git'),
    ]) {
      await expect(attestProgrammeCaptureInputsV1({
        controllerStore, controllerCommit: fixture.commit, taskPath: TASK_PATH,
      })).rejects.toThrow();
    }
    const bareStore = packedBareRepository(fixture.root);
    await expect(attestProgrammeCaptureInputsV1({
      controllerStore: bareStore, controllerCommit: fixture.commit, taskPath: TASK_PATH,
    })).resolves.toBeDefined();
    await expect(attestProgrammeCaptureInputsV1({
      controllerStore: join(bareStore, 'objects'),
      controllerCommit: fixture.commit,
      taskPath: TASK_PATH,
    })).rejects.toThrow();
    const linkedStore = linkedWorktree(fixture.root, fixture.commit);
    await expect(attestProgrammeCaptureInputsV1({
      controllerStore: linkedStore, controllerCommit: fixture.commit, taskPath: TASK_PATH,
    })).rejects.toThrow(/ROOT_MISMATCH/);
  });

  it('rejects abbreviated, symbolic, uppercase, or revision-expression commit authority', async () => {
    const fixture = repository();
    git(fixture.root, ['tag', '-a', 'authority-tag', '-m', 'tag'], identityEnvironment());
    for (const controllerCommit of [
      fixture.commit.slice(0, 12),
      'HEAD',
      fixture.commit.toUpperCase(),
      `${fixture.commit}^`,
      fixture.tree,
      git(fixture.root, ['rev-parse', 'refs/tags/authority-tag']).trim(),
    ]) {
      await expect(attestProgrammeCaptureInputsV1({
        controllerStore: fixture.root,
        controllerCommit,
        taskPath: TASK_PATH,
      })).rejects.toThrow();
    }
  });

  it('fails closed on manifest membership, duplicate JSON keys, or non-UTF-8 task bytes', async () => {
    const unlisted = repository({
      mutateManifest: (manifest) => { manifest.acceptanceTasks = ['coding-harness/config/issue-8-acceptance.json']; },
    });
    await expect(attest(unlisted)).rejects.toThrow('HARNESS_MANIFEST_TASK_NOT_LISTED');

    const duplicateManifest = repository({ duplicateManifestKey: true });
    await expect(attest(duplicateManifest)).rejects.toThrow(/duplicate JSON key/);
    const duplicateTask = repository({ duplicateTaskKey: true });
    await expect(attest(duplicateTask)).rejects.toThrow(/duplicate JSON key/);
    const nonUtf8Task = repository({ nonUtf8Task: true });
    await expect(attest(nonUtf8Task)).rejects.toThrow(/UTF-8/);
  });

  it('rejects missing, non-blob, digest-mismatched, or output-present commit entries', async () => {
    const missing = repository({ omitPath: PROGRAMME_CAPTURE_PROFILE_PATH });
    await expect(attest(missing)).rejects.toThrow(/BLOB_INVALID/);
    const symlink = repository({ symlinkPath: PROGRAMME_CAPTURE_PROFILE_PATH });
    await expect(attest(symlink)).rejects.toThrow(/BLOB_INVALID/);
    const executable = repository({ executablePath: PROGRAMME_CAPTURE_PROFILE_PATH });
    await expect(attest(executable)).rejects.toThrow(/BLOB_INVALID/);
    for (const mismatchPath of [
      PROGRAMME_CAPTURE_PROFILE_PATH,
      PROGRAMME_CAPTURE_SCENARIOS_PATH,
      'Cargo.lock',
      PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS.at(-1)!,
    ]) {
      const mismatch = repository({ mismatchPath });
      await expect(attest(mismatch)).rejects.toThrow(/DIGEST_MISMATCH/);
    }
    for (const outputKind of ['blob', 'symlink', 'tree', 'gitlink'] as const) {
      const outputPresent = repository({ outputKind });
      await expect(attest(outputPresent)).rejects.toThrow(/OUTPUT_PRESENT/);
    }
  }, 15_000);

  it('rejects unsafe Git materialization controls before reading authority blobs', async () => {
    const fixture = repository();
    git(fixture.root, ['config', 'core.autocrlf', 'true']);
    await expect(attest(fixture)).rejects.toThrow(/CONFIG_FORBIDDEN/);
  });

  it('strictly reparses and rehashes every attestation layer', async () => {
    const original = (await attest(repository())).record;
    expect(() => parseProgrammeCaptureInputAttestationBlobV1(
      JSON.stringify(original).replace(
        '"schemaVersion":1', '"schemaVersion":1,"schemaVersion":1',
      ),
    )).toThrow(/duplicate JSON key/);
    const mutants: Array<(record: any) => void> = [
      (record) => { record.extra = true; },
      (record) => { record.controller.commit = 'f'.repeat(40); },
      (record) => { record.manifest.path = 'README.md'; },
      (record) => { record.task.valueDigest = 'f'.repeat(64); },
      (record) => { record.protectedInputs[0].sha256 = 'f'.repeat(64); },
      (record) => { record.protectedInputs[0].byteLength += 1; },
      (record) => { record.protectedInputs.reverse(); },
      (record) => { record.output.absentFromCommit = false; },
      (record) => { record.protectedInputsDigest = 'f'.repeat(64); },
      (record) => { record.attestationDigest = 'f'.repeat(64); },
    ];
    for (const mutate of mutants) {
      const record = structuredClone(original);
      mutate(record);
      expect(() => parseProgrammeCaptureInputAttestationV1(record)).toThrow();
    }
  });

  it('rejects fully rehashed records outside generator byte ceilings', async () => {
    const original = (await attest(repository())).record;
    const mutations: Array<(record: any) => void> = [
      (record) => { record.manifest.byteLength = 1_048_577; },
      (record) => { record.task.byteLength = 1_048_577; },
      (record) => { record.protectedInputs[0].byteLength = 10_000_001; },
      (record) => {
        for (const input of record.protectedInputs) input.byteLength = 1_000_001;
      },
    ];
    for (const mutate of mutations) {
      const record = structuredClone(original);
      mutate(record);
      rehashAttestation(record);
      expect(() => parseProgrammeCaptureInputAttestationV1(record)).toThrow(/byte|large/);
    }
  });
});

interface RepositoryOptions {
  readonly objectFormat?: 'sha1' | 'sha256';
  readonly mismatchPath?: string;
  readonly omitPath?: string;
  readonly symlinkPath?: string;
  readonly executablePath?: string;
  readonly outputKind?: 'blob' | 'symlink' | 'tree' | 'gitlink';
  readonly duplicateManifestKey?: boolean;
  readonly duplicateTaskKey?: boolean;
  readonly nonUtf8Task?: boolean;
  readonly mutateManifest?: (manifest: any) => void;
}

interface RepositoryFixture {
  readonly root: string;
  readonly commit: string;
  readonly tree: string;
  readonly taskBlob: string;
}

function repository(options: RepositoryOptions = {}): RepositoryFixture {
  const root = mkdtempSync(join(tmpdir(), 'programme-capture-inputs-'));
  roots.push(root);
  const values = new Map<string, Buffer>();
  for (const path of PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1) {
    values.set(path, Buffer.from(`capture fixture: ${path}\n`, 'utf8'));
  }
  const task = taskInput(values, options.mismatchPath);
  let taskBlob = `${JSON.stringify(task)}\n`;
  if (options.duplicateTaskKey) {
    taskBlob = taskBlob.replace('"schemaVersion":1', '"schemaVersion":1,"schemaVersion":1');
  }
  const manifest = JSON.parse(readFileSync(
    new URL('../.harness/manifest.json', import.meta.url), 'utf8',
  ));
  options.mutateManifest?.(manifest);
  let manifestBlob = `${JSON.stringify(manifest, null, 2)}\n`;
  if (options.duplicateManifestKey) {
    manifestBlob = manifestBlob.replace(
      '"schemaVersion": 1', '"schemaVersion": 1,\n  "schemaVersion": 1',
    );
  }
  writeFile(root, 'coding-harness/.harness/manifest.json', manifestBlob);
  writeFile(root, TASK_PATH, options.nonUtf8Task ? Buffer.from([0xff]) : taskBlob);
  for (const [path, bytes] of values) {
    if (path === options.omitPath || path === options.symlinkPath) continue;
    writeFile(root, path, bytes);
  }
  if (options.symlinkPath !== undefined) {
    const absolute = join(root, options.symlinkPath);
    mkdirSync(dirname(absolute), { recursive: true });
    symlinkSync('/nonexistent/capture-input', absolute);
  }
  if (options.executablePath !== undefined) chmodSync(join(root, options.executablePath), 0o755);
  if (options.outputKind === 'blob') writeFile(root, PROGRAMME_CAPTURE_OUTPUT_PATH, Buffer.alloc(0));
  if (options.outputKind === 'symlink') {
    const output = join(root, PROGRAMME_CAPTURE_OUTPUT_PATH);
    mkdirSync(dirname(output), { recursive: true });
    symlinkSync('/nonexistent/capture-output', output);
  }
  if (options.outputKind === 'tree') {
    writeFile(root, `${PROGRAMME_CAPTURE_OUTPUT_PATH}/entry`, 'forbidden\n');
  }
  git(root, [
    'init', '--quiet',
    ...(options.objectFormat === 'sha256' ? ['--object-format=sha256'] : []),
  ]);
  git(root, ['add', '--all']);
  if (options.outputKind === 'gitlink') {
    const objectLength = options.objectFormat === 'sha256' ? 64 : 40;
    git(root, [
      'update-index', '--add', '--cacheinfo',
      `160000,${'1'.repeat(objectLength)},${PROGRAMME_CAPTURE_OUTPUT_PATH}`,
    ]);
  }
  git(root, ['commit', '--quiet', '-m', 'capture fixture'], identityEnvironment());
  chmodSync(join(root, '.git'), 0o755);
  chmodSync(join(root, '.git', 'objects'), 0o755);
  const commit = git(root, ['rev-parse', 'HEAD']).trim();
  const tree = git(root, ['rev-parse', `${commit}^{tree}`]).trim();
  return { root, commit, tree, taskBlob };
}

function taskInput(values: ReadonlyMap<string, Buffer>, mismatchPath?: string): Record<string, any> {
  const binding = (path: string) => ({
    path,
    sha256: path === mismatchPath ? 'f'.repeat(64) : sha256(values.get(path)!),
  });
  return {
    schemaVersion: 1,
    taskKind: 'controlled-performance-baseline',
    taskId: 'capture_m0_20260828',
    workItem: 'completion-programme:m0-performance-baseline',
    objective: 'Capture the frozen M0 performance baseline exactly once.',
    invariants: ['Measurement bytes come only from the attested producer.'],
    exclusions: ['No model process overlaps the measured interval.'],
    authority: 'development-only-no-promotion',
    inputs: {
      runnerProfile: binding(PROGRAMME_CAPTURE_PROFILE_PATH),
      scenarios: binding(PROGRAMME_CAPTURE_SCENARIOS_PATH),
      cargoLock: binding('Cargo.lock'),
      workloadSha256: 'd'.repeat(64),
      sources: PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS.map(binding),
    },
    commands: {
      capture: { commandId: 'capture_once_0001', command: captureCommand('capture-baseline', 1_800_000) },
      verify: { commandId: 'verify_capture_0001', command: captureCommand('check-baseline', 60_000) },
    },
    output: {
      path: PROGRAMME_CAPTURE_OUTPUT_PATH,
      mode: 'create-new',
      mediaType: 'text/tab-separated-values; charset=utf-8',
      maximumBytes: 1_048_576,
    },
    policy: {
      measurementNetwork: 'offline',
      modelTransport: 'native-first-party-only',
      nativeHosts: ['codex', 'claude-code'],
      dualReview: { preCapture: true, postCapture: true },
      maximumMeasurementAttempts: 1,
      automaticMeasurementRetries: 0,
      automaticRepairs: 0,
      modelMeasurementOverlap: 'forbidden',
      coreEvidence: 'fail-closed',
    },
    routing: { tags: ['controlled-capture', 'performance'], difficulty: 1, evolutionEligible: false },
  };
}

function captureCommand(argument: string, timeoutMs: number): Record<string, any> {
  return {
    tool: 'sf-performance-receipt',
    executable: 'target/release/sf-performance-receipt',
    argv: [argument],
    cwd: '.',
    env: {},
    timeoutMs,
    maxOutputBytes: 1_048_576,
  };
}

async function attest(fixture: RepositoryFixture) {
  return await attestProgrammeCaptureInputsV1({
    controllerStore: fixture.root,
    controllerCommit: fixture.commit,
    taskPath: TASK_PATH,
  });
}

function writeFile(root: string, path: string, value: string | Buffer): void {
  const absolute = join(root, path);
  mkdirSync(dirname(absolute), { recursive: true });
  writeFileSync(absolute, value);
}

function git(cwd: string, args: readonly string[], environment = process.env): string {
  const result = spawnSync('/usr/bin/git', args, { cwd, env: environment, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout;
}

function packedBareRepository(source: string): string {
  const parent = mkdtempSync(join(tmpdir(), 'programme-capture-bare-'));
  roots.push(parent);
  const store = join(parent, 'controller.git');
  git(parent, ['clone', '--quiet', '--bare', '--no-hardlinks', source, store]);
  git(store, ['repack', '-a', '-d']);
  chmodSync(store, 0o755);
  chmodSync(join(store, 'objects'), 0o755);
  return store;
}

function linkedWorktree(source: string, commit: string): string {
  const parent = mkdtempSync(join(tmpdir(), 'programme-capture-linked-'));
  roots.push(parent);
  const worktree = join(parent, 'worktree');
  git(source, ['worktree', 'add', '--quiet', '--detach', worktree, commit]);
  chmodSync(worktree, 0o755);
  const gitDirectory = git(worktree, ['rev-parse', '--absolute-git-dir']).trim();
  chmodSync(dirname(gitDirectory), 0o755);
  chmodSync(gitDirectory, 0o755);
  return worktree;
}

function identityEnvironment(): NodeJS.ProcessEnv {
  return {
    ...process.env,
    GIT_AUTHOR_NAME: 'Harness Test',
    GIT_AUTHOR_EMAIL: 'harness@example.invalid',
    GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z',
    GIT_COMMITTER_NAME: 'Harness Test',
    GIT_COMMITTER_EMAIL: 'harness@example.invalid',
    GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z',
  };
}

function sha256(value: Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}

function rehashAttestation(record: any): void {
  record.protectedInputsDigest = digestValue(record.protectedInputs);
  const { attestationDigest: _discarded, ...body } = record;
  record.attestationDigest = digestValue(body);
}

function compareUtf8(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

function visitObjects(value: unknown, visit: (value: object) => void): void {
  if (value === null || typeof value !== 'object') return;
  visit(value);
  for (const child of Object.values(value)) visitObjects(child, visit);
}
