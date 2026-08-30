// SPDX-License-Identifier: MIT

import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync,
  linkSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  truncateSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1 } from '../src/programme-capture-config-v1.js';
import {
  programmeCaptureRunClaimPathV1,
  readProgrammeCaptureRunClaimV1,
  reserveProgrammeCaptureRunClaimV1,
  type ProgrammeCaptureRunClaimAuthorityInputV1,
} from '../src/programme-capture-claim-io-v1.js';
import {
  rejectProgrammeCaptureHostPreflightV1,
  verifyProgrammeCaptureHostNonAdmissionV1,
} from '../src/programme-capture-host-authority-v1.js';
import { collectProgrammeCaptureHostObservationV1 } from '../src/programme-capture-host-preflight-v1.js';
import {
  PROGRAMME_CAPTURE_OUTPUT_PATH,
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
} from '../src/programme-capture-task-v1.js';

const roots: string[] = [];
const RUN_ID = 'capture_claim_io_20260828_0001';
const PROJECT_AUTHORITY = '1'.repeat(64);
const RUNNER_IDENTITY = '2'.repeat(64);
const digest = (character: string): string => character.repeat(64);
const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const TASK_PATH = 'coding-harness/config/programme-v5-acceptance.json';
let controller: Readonly<{ root: string; commit: string }>;

beforeAll(() => { controller = controllerRepository(); });
afterAll(() => { rmSync(controller.root, { recursive: true, force: true }); });

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme capture V1 local run-claim authority', () => {
  it('durably reserves, re-attests, reads, and admits one non-authorizing claim', async () => {
    const input = claimInput();
    const reserved = await reserveProgrammeCaptureRunClaimV1(input);
    const replayed = await readProgrammeCaptureRunClaimV1(input);

    expect(replayed).toEqual(reserved);
    expect(reserved.path).toBe(programmeCaptureRunClaimPathV1({
      authorityRoot: input.authorityRoot,
      projectAuthorityDigest: input.projectAuthorityDigest,
      runId: input.runId,
    }));
    expect(lstatSync(input.authorityRoot).mode & 0o7777).toBe(0o700);
    expect(lstatSync(reserved.path).mode & 0o7777).toBe(0o600);
    expect(lstatSync(reserved.path).nlink).toBe(1);
    expect(reserved.record).toMatchObject({
      hostAdmission: 'not-evaluated', runnerLeaseAcquired: false,
      attemptStartAuthorized: false, captureAuthorized: false,
    });
    expect(reserved.stateView).toMatchObject({ phase: 'admitted', captureAttempts: 0 });
    expect(reserved.stateView.events[0]).toMatchObject({
      kind: 'admit', evidenceDigest: reserved.admissionView.evidenceDigest,
    });
  });

  it('spends an occupied slot regardless of body and permits no idempotent claim', async () => {
    const input = claimInput();
    const first = await reserveProgrammeCaptureRunClaimV1(input);

    await expect(reserveProgrammeCaptureRunClaimV1(input)).rejects.toThrow(/CLAIM_SPENT/);
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...input,
      expectedRunnerIdentityDigest: digest('3'),
    })).rejects.toThrow(/CLAIM_SPENT/);

    const differentRun = await reserveProgrammeCaptureRunClaimV1({
      ...input, runId: 'capture_claim_io_20260828_0002',
    });
    expect(differentRun.path).not.toBe(first.path);

    rmSync(first.path);
    const recreated = await reserveProgrammeCaptureRunClaimV1(input);
    expect(recreated.record).toEqual(first.record);
    expect(recreated.path).toBe(first.path);
  }, 15_000);

  it.each([
    'regular', 'directory', 'hardlink', 'symlink', 'dangling-symlink', 'fifo',
  ] as const)('treats a pre-existing %s as a spent claim', async (kind) => {
    const input = claimInput();
    const path = programmeCaptureRunClaimPathV1({
      authorityRoot: input.authorityRoot,
      projectAuthorityDigest: input.projectAuthorityDigest,
      runId: input.runId,
    });
    const seed = join(input.authorityRoot, 'seed');
    writeFileSync(seed, 'seed', { mode: 0o600 });
    if (kind === 'regular') writeFileSync(path, 'occupied', { mode: 0o600 });
    if (kind === 'directory') mkdirSync(path, { mode: 0o700 });
    if (kind === 'hardlink') linkSync(seed, path);
    if (kind === 'symlink') symlinkSync(seed, path);
    if (kind === 'dangling-symlink') symlinkSync(join(input.authorityRoot, 'absent'), path);
    if (kind === 'fifo') {
      const result = spawnSync('mkfifo', [path], { encoding: 'utf8' });
      expect(result.status, result.stderr).toBe(0);
    }

    await expect(reserveProgrammeCaptureRunClaimV1(input)).rejects.toThrow(/CLAIM_SPENT/);
  });

  it('rejects relative, noncanonical, symlinked, permissive, and unsafe-parent roots', async () => {
    const valid = claimInput();
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...valid, authorityRoot: 'relative/claim-root',
    })).rejects.toThrow(/AUTHORITY_INVALID/);
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...valid, authorityRoot: join(valid.authorityRoot, '..', 'x'),
    })).rejects.toThrow(/AUTHORITY_INVALID/);

    const permissive = authorityRoot();
    chmodSync(permissive, 0o755);
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...valid, authorityRoot: permissive,
    })).rejects.toThrow(/AUTHORITY_INVALID/);

    const target = authorityRoot();
    const parent = temporary('capture-claim-symlink-parent-');
    const linked = join(parent, 'authority');
    symlinkSync(target, linked);
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...valid, authorityRoot: linked,
    })).rejects.toThrow(/AUTHORITY_INVALID/);

    const unsafeParent = temporary('capture-claim-unsafe-parent-');
    chmodSync(unsafeParent, 0o777);
    const nested = join(unsafeParent, 'authority');
    mkdirSync(nested, { mode: 0o700 });
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...valid, authorityRoot: nested,
    })).rejects.toThrow(/AUTHORITY_INVALID/);
  });

  it('fails replay on metadata, canonical-byte, digest, and UTF-8 corruption', async () => {
    const cases = [
      (path: string) => chmodSync(path, 0o640),
      (path: string) => linkSync(path, `${path}.hardlink`),
      (path: string) => truncateSync(path, 1),
      (path: string) => {
        const value = JSON.parse(readFileSync(path, 'utf8'));
        writeFileSync(path, JSON.stringify(value));
      },
      (path: string) => {
        const value = JSON.parse(readFileSync(path, 'utf8'));
        value.claimDigest = digest('f');
        writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
      },
      (path: string) => writeFileSync(path, Buffer.from([0xff])),
    ];
    for (const corrupt of cases) {
      const input = claimInput();
      const { path } = await reserveProgrammeCaptureRunClaimV1(input);
      corrupt(path);
      await expect(readProgrammeCaptureRunClaimV1(input)).rejects.toThrow();
    }
  }, 15_000);

  it('allows exactly one winner across separate concurrent processes', async () => {
    const input = claimInput();
    const inputPath = join(temporary('capture-claim-race-input-'), 'input.json');
    writeFileSync(inputPath, `${JSON.stringify(input)}\n`, { mode: 0o600 });
    const moduleUrl = pathToFileURL(
      resolve(harnessRoot, 'dist/programme-capture-claim-io-v1.js'),
    ).href;
    const script = [
      "import { readFileSync } from 'node:fs';",
      `import { reserveProgrammeCaptureRunClaimV1 } from ${JSON.stringify(moduleUrl)};`,
      "const input = JSON.parse(readFileSync(process.argv[1], 'utf8'));",
      'try { await reserveProgrammeCaptureRunClaimV1(input); process.stdout.write("won"); }',
      'catch (error) {',
      '  if (String(error?.message).includes("CLAIM_SPENT")) process.stdout.write("spent");',
      '  else { process.stderr.write(String(error?.stack ?? error)); process.exitCode = 1; }',
      '}',
    ].join('\n');

    const outcomes = await Promise.all([
      runChild(script, inputPath), runChild(script, inputPath),
    ]);
    expect(outcomes.map(({ status }) => status)).toEqual([0, 0]);
    expect(outcomes.map(({ stdout }) => stdout).sort()).toEqual(['spent', 'won']);
  });

  it('rejects self-authored attestations and invalid controller authority before claiming', async () => {
    const input = claimInput();
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...input, inputAttestation: {},
    } as any)).rejects.toThrow(/invalid keys/);
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...input, controllerCommit: 'f'.repeat(40),
    })).rejects.toThrow();
    await expect(reserveProgrammeCaptureRunClaimV1({
      ...input, taskPath: 'coding-harness/config/not-authoritative.json',
    })).rejects.toThrow();
    expect(() => lstatSync(programmeCaptureRunClaimPathV1({
      authorityRoot: input.authorityRoot,
      projectAuthorityDigest: input.projectAuthorityDigest,
      runId: input.runId,
    }))).toThrow(/ENOENT/);
  });

  it('roots the first host consumer and authoritative replay in the durable claim', async () => {
    const claimAuthority = claimInput();
    await reserveProgrammeCaptureRunClaimV1(claimAuthority);
    const profileBytes = fixtureProfileBytes();
    const result = await rejectProgrammeCaptureHostPreflightV1({
      claimAuthority, profileBytes, observation: collectProgrammeCaptureHostObservationV1(),
    });
    expect(result.record.captureAuthorized).toBe(false);
    expect(result.state).toMatchObject({ phase: 'failed', captureAttempts: 0 });
    await expect(verifyProgrammeCaptureHostNonAdmissionV1({
      record: result.record, state: result.state, claimAuthority, profileBytes,
    })).resolves.toEqual(result.record);
  });

  it('rejects absent, deleted, replaced, and caller-forged claim authority at host entry', async () => {
    const profileBytes = fixtureProfileBytes();
    const observation = collectProgrammeCaptureHostObservationV1();
    const missing = claimInput();
    await expect(rejectProgrammeCaptureHostPreflightV1({
      claimAuthority: missing, profileBytes, observation,
    })).rejects.toThrow(/CLAIM_MISSING/);

    const deleted = claimInput();
    const saved = await reserveProgrammeCaptureRunClaimV1(deleted);
    rmSync(saved.path);
    await expect(rejectProgrammeCaptureHostPreflightV1({
      claimAuthority: deleted, profileBytes, observation,
    })).rejects.toThrow(/CLAIM_MISSING/);

    const replaced = claimInput();
    const original = await reserveProgrammeCaptureRunClaimV1(replaced);
    rmSync(original.path);
    await reserveProgrammeCaptureRunClaimV1({
      ...replaced, expectedRunnerIdentityDigest: digest('3'),
    });
    await expect(rejectProgrammeCaptureHostPreflightV1({
      claimAuthority: replaced, profileBytes, observation,
    })).rejects.toThrow(/AUTHORITY_MISMATCH/);

    await expect(rejectProgrammeCaptureHostPreflightV1({
      claimAuthority: missing,
      profileBytes,
      observation,
      inputAttestation: saved.inputAttestation,
      admission: saved.admissionView,
      state: saved.stateView,
    } as any)).rejects.toThrow(/invalid keys/);
  });

  it('contains no provider, network, producer, delete, retry, or release capability', () => {
    const source = readFileSync(
      resolve(harnessRoot, 'src/programme-capture-claim-io-v1.ts'), 'utf8',
    );
    expect(source).toContain('constants.O_EXCL');
    expect(source).toContain('constants.O_NOFOLLOW');
    expect(source).toContain('fsyncSync(root.descriptor)');
    expect(source).toContain('attestProgrammeCaptureInputsV1');
    expect(source).not.toMatch(
      /node:(?:child_process|net|http|https|tls|dgram|worker_threads)|native-client|model|provider/,
    );
    expect(source).not.toMatch(/unlink|rmSync|rename|retry|release|capture-baseline/);
    const codec = readFileSync(
      resolve(harnessRoot, 'src/programme-capture-claim-record-v1.ts'), 'utf8',
    );
    expect(codec).not.toMatch(/run-claim-admission-v1|createProgrammeCaptureStateV1/);
  });
});

function claimInput(): ProgrammeCaptureRunClaimAuthorityInputV1 {
  return {
    authorityRoot: authorityRoot(),
    projectAuthorityDigest: PROJECT_AUTHORITY,
    runId: RUN_ID,
    controllerStore: controller.root,
    controllerCommit: controller.commit,
    taskPath: TASK_PATH,
    expectedRunnerIdentityDigest: RUNNER_IDENTITY,
  };
}

function fixtureProfileBytes(): Buffer {
  return Buffer.from(`capture claim fixture: ${PROGRAMME_CAPTURE_PROFILE_PATH}\n`, 'utf8');
}

function controllerRepository(): Readonly<{ root: string; commit: string }> {
  const root = mkdtempSync(join(tmpdir(), 'capture-claim-controller-'));
  const values = new Map<string, Buffer>();
  for (const path of PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1) {
    values.set(path, Buffer.from(`capture claim fixture: ${path}\n`, 'utf8'));
  }
  const binding = (path: string) => ({ path, sha256: sha256(values.get(path)!) });
  const task = {
    schemaVersion: 1,
    taskKind: 'controlled-performance-baseline',
    taskId: 'capture_claim_authority_20260828',
    workItem: 'completion-programme:m0-performance-baseline',
    objective: 'Exercise one controller-attested local claim reservation.',
    invariants: ['Claim inputs come only from the pinned controller commit.'],
    exclusions: ['No measurement or runner admission is authorized.'],
    authority: 'development-only-no-promotion',
    inputs: {
      runnerProfile: binding(PROGRAMME_CAPTURE_PROFILE_PATH),
      scenarios: binding(PROGRAMME_CAPTURE_SCENARIOS_PATH),
      cargoLock: binding('Cargo.lock'),
      workloadSha256: digest('d'),
      sources: PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS.map(binding),
    },
    commands: {
      capture: captureCommand('capture-baseline', 1_800_000),
      verify: captureCommand('check-baseline', 60_000),
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
    routing: {
      tags: ['controlled-capture', 'performance'], difficulty: 1, evolutionEligible: false,
    },
  };
  const manifest = readFileSync(
    new URL('../.harness/manifest.json', import.meta.url), 'utf8',
  );
  writeFixture(root, 'coding-harness/.harness/manifest.json', manifest);
  writeFixture(root, TASK_PATH, `${JSON.stringify(task)}\n`);
  for (const [path, bytes] of values) writeFixture(root, path, bytes);
  git(root, ['init', '--quiet']);
  git(root, ['add', '--all']);
  git(root, ['commit', '--quiet', '-m', 'capture claim fixture'], identityEnvironment());
  chmodSync(join(root, '.git'), 0o755);
  chmodSync(join(root, '.git', 'objects'), 0o755);
  return { root, commit: git(root, ['rev-parse', 'HEAD']).trim() };
}

function captureCommand(argument: string, timeoutMs: number) {
  return {
    commandId: `${argument}_0001`,
    command: {
      tool: 'sf-performance-receipt',
      executable: 'target/release/sf-performance-receipt',
      argv: [argument], cwd: '.', env: {}, timeoutMs, maxOutputBytes: 1_048_576,
    },
  };
}

function writeFixture(root: string, path: string, value: string | Buffer): void {
  const absolute = join(root, path);
  mkdirSync(dirname(absolute), { recursive: true });
  writeFileSync(absolute, value);
}

function git(cwd: string, args: readonly string[], environment = process.env): string {
  const result = spawnSync('/usr/bin/git', args, { cwd, env: environment, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout;
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

function authorityRoot(): string {
  const root = temporary('capture-claim-authority-');
  chmodSync(root, 0o700);
  return root;
}

function temporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  roots.push(root);
  return root;
}

function runChild(script: string, inputPath: string): Promise<{
  status: number | null;
  stdout: string;
  stderr: string;
}> {
  return new Promise((resolveResult) => {
    const child = spawn(process.execPath, ['--input-type=module', '--eval', script, inputPath], {
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8').on('data', (chunk) => { stdout += chunk; });
    child.stderr.setEncoding('utf8').on('data', (chunk) => { stderr += chunk; });
    child.on('close', (status) => resolveResult({ status, stdout, stderr }));
  });
}
