// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { parseAcceptanceTask } from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { parseHarnessConfig } from '../src/contracts.js';
import { assertProgrammeV5ControllerTask } from '../src/programme-v5-driver-support.js';
import {
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  parseProgrammeCaptureTaskBlobV1,
  parseProgrammeCaptureTaskV1,
} from '../src/programme-capture-task-v1.js';

const digest = (character: string): string => character.repeat(64);
const CAPTURE_HARNESS_CONFIG = parseHarnessConfig({
  ...structuredClone(SECURE_HARNESS_CONFIG),
  requiredProtectedPaths: [...new Set([
    ...SECURE_HARNESS_CONFIG.requiredProtectedPaths,
    PROGRAMME_CAPTURE_PROFILE_PATH,
    ...PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  ])].sort(),
});

function taskInput(): Record<string, any> {
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
      runnerProfile: {
        path: 'crates/sf-bench/config/performance-runner-profile-v1.tsv',
        sha256: digest('a'),
      },
      scenarios: {
        path: 'crates/sf-bench/config/performance-scenarios-v1.tsv',
        sha256: digest('b'),
      },
      cargoLock: { path: 'Cargo.lock', sha256: digest('c') },
      workloadSha256: digest('d'),
      sources: PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS.map((path) => ({
        path,
        sha256: digest('e'),
      })),
    },
    commands: {
      capture: {
        commandId: 'capture_once_0001',
        command: {
          tool: 'sf-performance-receipt',
          executable: 'target/release/sf-performance-receipt',
          argv: ['capture-baseline'],
          cwd: '.',
          env: {},
          timeoutMs: 1_800_000,
          maxOutputBytes: 1_048_576,
        },
      },
      verify: {
        commandId: 'verify_capture_0001',
        command: {
          tool: 'sf-performance-receipt',
          executable: 'target/release/sf-performance-receipt',
          argv: ['check-baseline'],
          cwd: '.',
          env: {},
          timeoutMs: 60_000,
          maxOutputBytes: 1_048_576,
        },
      },
    },
    output: {
      path: 'crates/sf-bench/config/performance-baseline-v1.tsv',
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
      tags: ['controlled-capture', 'performance'],
      difficulty: 1,
      evolutionEligible: false,
    },
  };
}

function cloneTask(): Record<string, any> {
  return structuredClone(taskInput());
}

function visitObjects(value: unknown, visit: (value: object) => void): void {
  if (value === null || typeof value !== 'object') return;
  visit(value);
  for (const child of Object.values(value)) visitObjects(child, visit);
}

interface CargoMetadataPackage {
  readonly name: string;
  readonly manifest_path: string;
  readonly dependencies: readonly Readonly<{ path?: string }>[];
}

function discoverCaptureLocalSourceClosure(): string[] {
  const repositoryRoot = fileURLToPath(new URL('../..', import.meta.url));
  const metadata = JSON.parse(execFileSync('cargo', [
    'metadata', '--locked', '--offline', '--format-version', '1', '--no-deps',
  ], { cwd: repositoryRoot, encoding: 'utf8' })) as {
    readonly packages: readonly CargoMetadataPackage[];
  };
  const packageByDirectory = new Map(metadata.packages.map((pkg) => [
    dirname(pkg.manifest_path),
    pkg,
  ]));
  const root = metadata.packages.find((pkg) => pkg.name === 'sf-bench');
  if (root === undefined) throw new Error('sf-bench is absent from Cargo metadata');

  const queue = [root];
  const reachable = new Map<string, CargoMetadataPackage>();
  while (queue.length > 0) {
    const pkg = queue.pop();
    if (pkg === undefined || reachable.has(pkg.manifest_path)) continue;
    reachable.set(pkg.manifest_path, pkg);
    for (const dependency of pkg.dependencies) {
      if (dependency.path === undefined) continue;
      const localPackage = packageByDirectory.get(dependency.path);
      if (localPackage === undefined) {
        throw new Error(`local Cargo dependency has no package metadata: ${dependency.path}`);
      }
      queue.push(localPackage);
    }
  }

  expect([...reachable.values()].map((pkg) => pkg.name).sort()).toEqual([
    'sf-bench', 'sf-core', 'sf-mapping', 'sf-sparql', 'sf-sql',
  ]);
  const packagePaths = [...reachable.values()].flatMap((pkg) => {
    const packageDirectory = dirname(relative(repositoryRoot, pkg.manifest_path));
    return execFileSync('git', ['ls-files', '--',
      `${packageDirectory}/Cargo.toml`,
      `${packageDirectory}/build.rs`,
      `${packageDirectory}/src`,
    ], { cwd: repositoryRoot, encoding: 'utf8' }).trim().split('\n').filter(Boolean);
  });
  return ['Cargo.toml', ...packagePaths, 'rust-toolchain.toml'].sort();
}

describe('programme capture V1 task', () => {
  it('binds every tracked source in the reachable local Cargo package closure', () => {
    expect([...PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS])
      .toEqual(discoverCaptureLocalSourceClosure());
  });

  it('parses the exact capture contract and deeply freezes it', () => {
    expect(() => parseProgrammeCaptureTaskV1(taskInput(), SECURE_HARNESS_CONFIG))
      .toThrow(/protected controller inputs/);
    const task = parseProgrammeCaptureTaskV1(taskInput(), CAPTURE_HARNESS_CONFIG);

    expect(task).toMatchObject({
      schemaVersion: 1,
      taskKind: 'controlled-performance-baseline',
      authority: 'development-only-no-promotion',
      output: { mode: 'create-new', maximumBytes: 1_048_576 },
      policy: {
        measurementNetwork: 'offline',
        maximumMeasurementAttempts: 1,
        automaticMeasurementRetries: 0,
        automaticRepairs: 0,
      },
      routing: { evolutionEligible: false },
    });
    visitObjects(task, (value) => expect(Object.isFrozen(value)).toBe(true));
  });

  it('rejects unknown, missing, or substituted identity and authority fields', () => {
    for (const mutate of [
      (task: any) => { task.extra = true; },
      (task: any) => { delete task.objective; },
      (task: any) => { task.schemaVersion = '1'; },
      (task: any) => { task.taskKind = 'candidate-patch'; },
      (task: any) => { task.authority = 'promotion'; },
      (task: any) => { task.taskId = 'short'; },
      (task: any) => { task.workItem = 'bad\0work'; },
      (task: any) => { task.objective = 'bad\rtext'; },
    ]) {
      const task = cloneTask();
      mutate(task);
      expect(() => parseProgrammeCaptureTaskV1(task, CAPTURE_HARNESS_CONFIG)).toThrow();
    }
    expect(() => parseProgrammeCaptureTaskV1(Object.create(taskInput()), CAPTURE_HARNESS_CONFIG))
      .toThrow(/plain own-key object/);
  });

  it('requires exact, non-overlapping, byte-sorted input bindings and real digests', () => {
    const mutants: Array<(task: any) => void> = [
      (task) => { task.inputs.runnerProfile.path = '../profile.tsv'; },
      (task) => { task.inputs.scenarios.path = '/tmp/scenarios.tsv'; },
      (task) => { task.inputs.cargoLock.path = 'nested/Cargo.lock'; },
      (task) => { task.inputs.workloadSha256 = digest('0'); },
      (task) => { task.inputs.sources[0].sha256 = digest('A'); },
      (task) => { task.inputs.sources[0].sha256 = 'a'.repeat(63); },
      (task) => { task.inputs.sources.reverse(); },
      (task) => { task.inputs.sources.push(structuredClone(task.inputs.sources[0])); },
      (task) => { task.inputs.sources[0].path = task.output.path; },
      (task) => { task.inputs.sources[0].path = 'crates/sf-bench'; },
      (task) => { task.inputs.sources[0].path = 'crates\\sf-bench\\src\\lib.rs'; },
      (task) => { task.inputs.sources = [{
        path: 'README.md',
        sha256: digest('e'),
      }]; },
      (task) => { task.inputs.sources.pop(); },
      (task) => { task.inputs.sources = []; },
      (task) => { task.inputs.sources = new Array(1); },
      (task) => { task.inputs.sources = Array.from({ length: 257 }, (_, index) => ({
        path: `crates/sf-bench/src/generated-${String(index).padStart(3, '0')}.rs`,
        sha256: digest('e'),
      })); },
      (task) => { task.inputs.extra = true; },
    ];
    for (const mutate of mutants) {
      const task = cloneTask();
      mutate(task);
      expect(() => parseProgrammeCaptureTaskV1(task, CAPTURE_HARNESS_CONFIG)).toThrow();
    }
  });

  it('permits only the two exact shell-free logical producer commands', () => {
    const mutants: Array<(task: any) => void> = [
      (task) => { task.commands.capture.command.tool = 'cargo'; },
      (task) => { task.commands.capture.command.executable = '/tmp/sf-performance-receipt'; },
      (task) => { task.commands.capture.command.argv = ['check-baseline']; },
      (task) => { task.commands.verify.command.argv = ['capture-baseline']; },
      (task) => { task.commands.capture.command.argv.push(';touch'); },
      (task) => { task.commands.capture.command.cwd = 'crates/sf-bench'; },
      (task) => { task.commands.capture.command.env.OPENROUTER_API_KEY = 'secret'; },
      (task) => { task.commands.capture.command.timeoutMs = 1_800_001; },
      (task) => { task.commands.capture.command.timeoutMs = 1; },
      (task) => { task.commands.capture.command.maxOutputBytes = 1_048_577; },
      (task) => { task.commands.verify.command.maxOutputBytes = 1; },
      (task) => { task.commands.verify.commandId = task.commands.capture.commandId; },
      (task) => { task.commands.capture.command.extra = true; },
    ];
    for (const mutate of mutants) {
      const task = cloneTask();
      mutate(task);
      expect(() => parseProgrammeCaptureTaskV1(task, CAPTURE_HARNESS_CONFIG)).toThrow();
    }
  });

  it('fixes create-new output and terminal offline review policy', () => {
    const mutants: Array<(task: any) => void> = [
      (task) => { task.output.path = 'target/baseline.tsv'; },
      (task) => { task.output.mode = 'overwrite'; },
      (task) => { task.output.mediaType = 'text/plain'; },
      (task) => { task.output.maximumBytes = 1_048_577; },
      (task) => { task.policy.measurementNetwork = 'first-party-model'; },
      (task) => { task.policy.modelTransport = 'openrouter'; },
      (task) => { task.policy.nativeHosts.reverse(); },
      (task) => { task.policy.dualReview.preCapture = false; },
      (task) => { task.policy.dualReview.postCapture = false; },
      (task) => { task.policy.maximumMeasurementAttempts = 2; },
      (task) => { task.policy.automaticMeasurementRetries = 1; },
      (task) => { task.policy.automaticRepairs = 1; },
      (task) => { task.policy.modelMeasurementOverlap = 'allowed'; },
      (task) => { task.policy.coreEvidence = 'best-effort'; },
      (task) => { task.routing.evolutionEligible = true; },
      (task) => { task.routing.difficulty = 1.1; },
    ];
    for (const mutate of mutants) {
      const task = cloneTask();
      mutate(task);
      expect(() => parseProgrammeCaptureTaskV1(task, CAPTURE_HARNESS_CONFIG)).toThrow();
    }
  });

  it('rejects duplicate JSON keys and schema confusion in both directions', () => {
    const blob = JSON.stringify(taskInput()).replace(
      '"schemaVersion":1',
      '"schemaVersion":1,"schemaVersion":1',
    );
    expect(() => parseProgrammeCaptureTaskBlobV1(blob, CAPTURE_HARNESS_CONFIG))
      .toThrow(/duplicate JSON key/);
    const nestedBlob = JSON.stringify(taskInput()).replace(
      '"mode":"create-new"',
      '"mode":"create-new","m\\u006fde":"create-new"',
    );
    expect(() => parseProgrammeCaptureTaskBlobV1(nestedBlob, CAPTURE_HARNESS_CONFIG))
      .toThrow(/duplicate JSON key/);
    expect(() => parseAcceptanceTask(taskInput(), SECURE_HARNESS_CONFIG))
      .toThrow(/schemaVersion must be 2 or 3/);

    const patchTask = JSON.parse(readFileSync(
      new URL('../config/programme-v5-acceptance.json', import.meta.url),
      'utf8',
    ));
    expect(() => parseProgrammeCaptureTaskV1(patchTask, CAPTURE_HARNESS_CONFIG)).toThrow();

    const captureTask = parseProgrammeCaptureTaskV1(taskInput(), CAPTURE_HARNESS_CONFIG);
    const capturePath = 'coding-harness/config/m0-performance-baseline-acceptance.json';
    expect(() => assertProgrammeV5ControllerTask({
      taskPath: capturePath,
      task: captureTask,
    } as any, {
      taskPath: capturePath,
      controllerCommit: 'a'.repeat(40),
    }, {} as any)).toThrow('HARNESS_PROGRAMME_V5_VERIFIER_ONLY_TASK_REQUIRED');
  });
});
