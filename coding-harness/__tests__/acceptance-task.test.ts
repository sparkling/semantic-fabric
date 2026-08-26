// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  acceptanceTaskPrompt,
  bindAcceptanceTaskToRustProfile,
  parseAcceptanceTask,
  type AcceptanceTask,
} from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import type { StructuredCommand } from '../src/contracts.js';
import { ISSUE_8_RUST_LIMITS } from '../src/issue-8-system.js';
import { limitsForProcessDeadline } from '../src/resource-boundary.js';
import type { RustOfflineProfile } from '../src/rust-sandbox.js';

function taskInput(): Record<string, unknown> {
  return JSON.parse(readFileSync(
    new URL('../config/issue-8-acceptance.json', import.meta.url),
    'utf8',
  )) as Record<string, unknown>;
}

function cloneTask(): Record<string, any> {
  return structuredClone(taskInput());
}

function verifierOnlyTaskInput(): Record<string, any> {
  const input = cloneTask();
  input.schemaVersion = 3;
  input.taskId = 'verifier_only_task_0001';
  input.workItem = 'completion-programme:reproducibility';
  input.candidateOracle = { mode: 'verifier-only' };
  delete input.qeProfiles;
  input.rust = { frozenLockSha256: 'a'.repeat(64) };
  input.qe = {
    profiles: [
      { profile: 'sast', collector: 'agentic-qe-sast' },
      {
        profile: 'lcov-gap',
        collector: 'rust-lcov',
        packageName: 'sf-conformance',
        testTarget: 'issue_8_binding_pruning',
      },
    ],
  };
  input.evidence = {
    requiredAdmittedPaths: ['crates/sf-sparql/src/unfold.rs'],
    generatedOutputs: [{
      stage: 'regression',
      evidenceId: 'workspace-tests-earl',
      commandId: 'workspace-tests',
      workspacePaths: ['tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl'],
    }],
  };
  return input;
}

const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url));

const rustProfile: RustOfflineProfile = Object.freeze({
  cargoExecutable: '/toolchain/bin/cargo',
  environment: Object.freeze({
    PATH: '/toolchain/bin:/usr/bin',
    HOME: '/home/harness',
    CARGO_HOME: '/cargo-home',
    CARGO_NET_OFFLINE: 'true',
    CARGO_INCREMENTAL: '0',
  }),
  readOnlyMounts: Object.freeze([]),
  isolator: Object.freeze({
    isolate() {},
    assertStable() {},
  }),
});

function acceptanceCommands(task: AcceptanceTask): Array<[string, StructuredCommand]> {
  return [
    ...task.redBaseline.commands.map(({ commandId, command }) => [`red:${commandId}`, command] as const),
    ...task.commands.build.map(({ commandId, command }) => [`build:${commandId}`, command] as const),
    ...task.commands.public.map(({ commandId, command }) => [`public:${commandId}`, command] as const),
    ...task.commands.independent.map(({ commandId, command }) => [`independent:${commandId}`, command] as const),
    ...task.commands.regression.map(({ commandId, command }) => [`regression:${commandId}`, command] as const),
    ...task.commands.mutation.map(({ mutationId, command }) => [`mutation:${mutationId}`, command] as const),
  ];
}

function visitObjects(value: unknown, visit: (object: object) => void): void {
  if (value === null || typeof value !== 'object') return;
  visit(value);
  for (const child of Object.values(value)) visitObjects(child, visit);
}

describe('programme acceptance task', () => {
  it('parses the canonical shell-free task and freezes every nested value', () => {
    const task = parseAcceptanceTask(taskInput(), SECURE_HARNESS_CONFIG);

    expect(task).toMatchObject({
      schemaVersion: 2,
      workItem: 'github-issue:8',
      authority: 'development-only-no-promotion',
      baseline: {
        commit: 'd510fc952a8dc701d65b1a4f3ad25a8109b98669',
        tree: 'b5d67e0fdb107e6502959fd2ff36831170d093b1',
      },
      candidateOracle: {
        mode: 'exact-reference',
        candidate: {
          commit: '10dedd40bda63d3acef18b8d34f61a32214e98d4',
          tree: 'a3f637f6b14fff73e5209e539b7a19f0b6b73ffa',
        },
      },
      policy: {
        candidateNetwork: 'offline',
        modelTransport: 'native-first-party-only',
        nativeHosts: ['codex', 'claude-code'],
      },
      routing: {
        tags: ['rust', 'sparql', 'binding-pruning', 'sealed-evaluator'],
        difficulty: 0.8,
      },
      qeProfiles: ['lcov-gap', 'sast'],
      evolutionEligible: false,
    });
    expect(task.evaluatorPaths).toEqual(['crates/sf-conformance/tests/issue_8_binding_pruning.rs']);
    expect(task.implementationPaths).toEqual(['crates/sf-sparql/src/unfold.rs']);
    expect(task.artifactPaths).toEqual(task.implementationPaths);
    expect(task.redBaseline.expected.exitCode).toBe(101);
    expect(task.redBaseline.expected.failedTests).toHaveLength(4);
    expect(Object.values(task.commands).every((commands) => commands.length > 0)).toBe(true);
    expect(task.commands.mutation.map(({ mutationId }) => mutationId)).toEqual([
      'ordinary-predicate-bind-prune',
      'ordinary-subject-bind-prune',
      'class-object-bind-prune',
      'class-predicate-bind-prune',
    ]);
    expect(task.commands.mutation.every(({ path, search, replacement }) => (
      task.implementationPaths.includes(path) && search.length > 0 && replacement.length > 0
    ))).toBe(true);
    expect(task.commands.regression.every(({ command }) => Array.isArray(command.argv))).toBe(true);
    const allCommands = [
      ...task.redBaseline.commands,
      ...task.commands.build,
      ...task.commands.public,
      ...task.commands.independent,
      ...task.commands.regression,
      ...task.commands.mutation,
    ].map(({ command }) => command);
    expect(allCommands.every(({ argv }) => argv[0] === '--offline')).toBe(true);
    expect(allCommands.every(({ argv }) => argv[1] === 'fmt'
      ? !argv.includes('--locked')
      : argv.includes('--locked'))).toBe(true);
    for (const command of allCommands) {
      expect(limitsForProcessDeadline(
        ISSUE_8_RUST_LIMITS,
        command.timeoutMs,
        SECURE_HARNESS_CONFIG.limits.terminationGraceMs,
      ).runtimeSeconds).toBe(1_801);
    }
    expect(Object.isFrozen(task)).toBe(true);
    expect(Object.isFrozen(task.commands.mutation[0].command.argv)).toBe(true);
    expect(acceptanceTaskPrompt(task)).toContain('Objective: Prune an unfolding branch');
  });

  it('accepts a non-issue work item while keeping the exact-reference oracle explicit', () => {
    const input = cloneTask();
    input.taskId = 'bounded_operator_task_0001';
    input.workItem = 'completion-programme:bounded-operators';
    input.objective = 'Make one declared global operator bounded without changing its semantics.';
    input.invariants = ['Direct repository oracles remain authoritative.'];
    input.exclusions = ['Do not widen the declared mutable path set.'];
    input.routing = { tags: ['rust', 'boundedness'], difficulty: 0.9 };
    input.qeProfiles = ['sast'];

    const task = parseAcceptanceTask(input, SECURE_HARNESS_CONFIG);
    expect(task.workItem).toBe('completion-programme:bounded-operators');
    expect(task.routing).toEqual({ tags: ['rust', 'boundedness'], difficulty: 0.9 });
    expect(task.candidateOracle.mode).toBe('exact-reference');
  });

  it('accepts a verifier-only oracle without a pre-known candidate identity', () => {
    const input = verifierOnlyTaskInput();

    const task = parseAcceptanceTask(input, SECURE_HARNESS_CONFIG);

    expect(task.schemaVersion).toBe(3);
    if (task.schemaVersion !== 3) throw new Error('expected v3 task');
    expect(task.candidateOracle).toEqual({ mode: 'verifier-only' });
    expect(task.rust.frozenLockSha256).toBe('a'.repeat(64));
    expect(task.qe.profiles.map(({ profile }) => profile)).toEqual(['lcov-gap', 'sast']);
    expect(task.evidence.requiredAdmittedPaths).toEqual(['crates/sf-sparql/src/unfold.rs']);
    expect(task.evidence.generatedOutputs).toEqual([{
      stage: 'regression',
      evidenceId: 'workspace-tests-earl',
      commandId: 'workspace-tests',
      workspacePaths: ['tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl'],
    }]);
  });

  it('deep-clones, binds, and freezes every acceptance command for the Rust profile', () => {
    const parsedTask = parseAcceptanceTask(taskInput(), SECURE_HARNESS_CONFIG);
    const parsedSnapshot = structuredClone(parsedTask);
    const parsedObjects = new Set<object>();
    visitObjects(parsedTask, (object) => parsedObjects.add(object));

    const boundTask = bindAcceptanceTaskToRustProfile(parsedTask, rustProfile);
    const parsedCommands = acceptanceCommands(parsedTask);
    const boundCommands = acceptanceCommands(boundTask);

    expect(boundCommands.map(([label]) => label)).toEqual(parsedCommands.map(([label]) => label));
    expect(boundCommands).toHaveLength(13);
    for (let index = 0; index < parsedCommands.length; index += 1) {
      const original = parsedCommands[index][1];
      const bound = boundCommands[index][1];
      expect(bound.tool).toBe('cargo');
      expect(bound.executable).toBe('/toolchain/bin/cargo');
      expect(bound.argv).toEqual(original.argv);
      expect(bound.argv).not.toBe(original.argv);
      expect(bound.env).toEqual({ ...original.env, ...rustProfile.environment });
      expect(bound.env).not.toBe(original.env);
    }

    visitObjects(boundTask, (object) => {
      expect(parsedObjects.has(object)).toBe(false);
      expect(Object.isFrozen(object)).toBe(true);
    });
    expect(parsedTask).toEqual(parsedSnapshot);
    expect(acceptanceCommands(parsedTask).every(([, command]) => command.executable === 'cargo')).toBe(true);
  });

  it('rejects shell command strings and metacharacters in argv', () => {
    const shellString = cloneTask();
    shellString.commands.public[0] = 'cargo test -p sf-conformance';
    expect(() => parseAcceptanceTask(shellString, SECURE_HARNESS_CONFIG)).toThrow(/must be an object/);

    const injected = cloneTask();
    injected.commands.public[0].command.argv.push('bad;touch');
    expect(() => parseAcceptanceTask(injected, SECURE_HARNESS_CONFIG)).toThrow(/shell metacharacter/);
  });

  it('rejects unknown keys, non-exact identities, and overlapping source paths', () => {
    const unknown = cloneTask();
    unknown.unreviewed = true;
    expect(() => parseAcceptanceTask(unknown, SECURE_HARNESS_CONFIG)).toThrow(/invalid keys/);

    const shortIdentity = cloneTask();
    shortIdentity.baseline.commit = 'abc123';
    expect(() => parseAcceptanceTask(shortIdentity, SECURE_HARNESS_CONFIG)).toThrow(/40-character/);

    const overlap = cloneTask();
    overlap.implementationPaths = [...overlap.evaluatorPaths];
    expect(() => parseAcceptanceTask(overlap, SECURE_HARNESS_CONFIG)).toThrow(/must not overlap/);

    for (const protectedPath of ['.github/workflows/ci.yml', 'Cargo.lock']) {
      const governed = cloneTask();
      governed.implementationPaths = [protectedPath];
      governed.artifactPaths = [protectedPath];
      governed.commands.mutation.forEach((mutation: Record<string, unknown>) => {
        mutation.path = protectedPath;
      });
      expect(() => parseAcceptanceTask(governed, SECURE_HARNESS_CONFIG)).toThrow(/protected paths/);
    }
  });

  it('rejects weak red signatures, duplicate mutations, and empty gate groups', () => {
    const weakRed = cloneTask();
    weakRed.redBaseline.expected.failedTests = [];
    expect(() => parseAcceptanceTask(weakRed, SECURE_HARNESS_CONFIG)).toThrow(/non-empty array/);

    const duplicateMutation = cloneTask();
    duplicateMutation.commands.mutation[1].mutationId =
      duplicateMutation.commands.mutation[0].mutationId;
    expect(() => parseAcceptanceTask(duplicateMutation, SECURE_HARNESS_CONFIG)).toThrow(/mutationId.*unique/);

    const emptyGate = cloneTask();
    emptyGate.commands.independent = [];
    expect(() => parseAcceptanceTask(emptyGate, SECURE_HARNESS_CONFIG)).toThrow(/non-empty array/);
  });

  it('binds each mutation to one exact reference-candidate transform back to baseline code', () => {
    const task = parseAcceptanceTask(taskInput(), SECURE_HARNESS_CONFIG);
    expect(task.candidateOracle.mode).toBe('exact-reference');
    if (task.candidateOracle.mode !== 'exact-reference') throw new Error('expected exact reference');
    for (const mutation of task.commands.mutation) {
      const referenceCandidate = execFileSync(
        'git',
        ['-C', repositoryRoot, 'show', `${task.candidateOracle.candidate.commit}:${mutation.path}`],
        { encoding: 'utf8' },
      );
      const baseline = execFileSync(
        'git',
        ['-C', repositoryRoot, 'show', `${task.baseline.commit}:${mutation.path}`],
        { encoding: 'utf8' },
      );
      expect(referenceCandidate.split(mutation.search)).toHaveLength(2);
      expect(baseline).toContain(mutation.replacement);
    }
  });

  it('rejects fake artifacts, unbound transforms, and weak Cargo isolation flags', () => {
    const fakeArtifact = cloneTask();
    fakeArtifact.artifactPaths = ['.metaharness/runs/bprune_8_20260825/build.json'];
    expect(() => parseAcceptanceTask(fakeArtifact, SECURE_HARNESS_CONFIG)).toThrow(/implementation path/);

    const unboundTransform = cloneTask();
    unboundTransform.commands.mutation[0].path = 'crates/sf-sparql/src/not-the-fix.rs';
    expect(() => parseAcceptanceTask(unboundTransform, SECURE_HARNESS_CONFIG)).toThrow(/implementation path/);

    const emptyReplacement = cloneTask();
    emptyReplacement.commands.mutation[0].replacement = '';
    expect(() => parseAcceptanceTask(emptyReplacement, SECURE_HARNESS_CONFIG)).toThrow(/non-empty/);

    const online = cloneTask();
    online.commands.public[0].command.argv = online.commands.public[0].command.argv
      .filter((arg: string) => arg !== '--offline');
    expect(() => parseAcceptanceTask(online, SECURE_HARNESS_CONFIG)).toThrow(/--offline/);

    const unlocked = cloneTask();
    unlocked.commands.public[0].command.argv = unlocked.commands.public[0].command.argv
      .filter((arg: string) => arg !== '--locked');
    expect(() => parseAcceptanceTask(unlocked, SECURE_HARNESS_CONFIG)).toThrow(/--locked/);
  });

  it('fails closed when offline, native-host, authority, or evolution policy changes', () => {
    for (const mutate of [
      (task: Record<string, any>) => { task.policy.candidateNetwork = 'online'; },
      (task: Record<string, any>) => { task.policy.nativeHosts = ['codex']; },
      (task: Record<string, any>) => { task.authority = 'promotion-authority'; },
      (task: Record<string, any>) => { task.evolutionEligible = true; },
      (task: Record<string, any>) => { task.routing.difficulty = 1.1; },
      (task: Record<string, any>) => { task.qeProfiles = ['unknown-qe']; },
      (task: Record<string, any>) => { task.qeProfiles = ['quality-contract']; },
    ]) {
      const input = cloneTask();
      mutate(input);
      expect(() => parseAcceptanceTask(input, SECURE_HARNESS_CONFIG)).toThrow();
    }

    for (const candidateOracle of [
      { mode: 'verifier-only' },
      { mode: 'verifier-only', evaluatorSource: 'reference-candidate' },
      { mode: 'verifier-only', evaluatorSource: 'controller-commit', candidate: {} },
      { mode: 'exact-reference' },
      { mode: 'unknown', evaluatorSource: 'controller-commit' },
    ]) {
      const input = cloneTask();
      input.candidateOracle = candidateOracle;
      expect(() => parseAcceptanceTask(input, SECURE_HARNESS_CONFIG)).toThrow();
    }

    const widenedV2 = cloneTask();
    widenedV2.candidateOracle = { mode: 'verifier-only' };
    expect(() => parseAcceptanceTask(widenedV2, SECURE_HARNESS_CONFIG))
      .toThrow(/schemaVersion 3/);
  });

  it('rejects task-controlled QE policy and generated-output boundary escapes', () => {
    const invalidVariants = [
      (task: Record<string, any>) => { task.qe.profiles[0].collector = 'task-selected-command'; },
      (task: Record<string, any>) => { task.qe.profiles[1].testTarget = '../unsafe'; },
      (task: Record<string, any>) => { task.qe.profiles.push({ profile: 'sast', collector: 'agentic-qe-sast' }); },
      (task: Record<string, any>) => { task.evidence.generatedOutputs[0].commandId = 'issue8-public'; },
      (task: Record<string, any>) => { task.evidence.generatedOutputs[0].evidenceId = 'Invalid_ID'; },
      (task: Record<string, any>) => {
        task.evidence.generatedOutputs.push({
          ...task.evidence.generatedOutputs[0],
          evidenceId: 'duplicate-producer',
        });
      },
      (task: Record<string, any>) => {
        task.evidence.requiredAdmittedPaths = ['crates/sf-core/src/lib.rs'];
      },
      (task: Record<string, any>) => {
        task.implementationPaths.push('crates/sf-core/src/lib.rs');
        task.evidence.requiredAdmittedPaths.push('crates/sf-core/src/lib.rs');
      },
      (task: Record<string, any>) => {
        task.evidence.generatedOutputs[0].workspacePaths = ['crates/sf-sparql/src/unfold.rs'];
      },
      (task: Record<string, any>) => {
        task.evidence.generatedOutputs[0].workspacePaths = ['.github/workflows/ci.yml'];
      },
      (task: Record<string, any>) => { task.rust.frozenLockSha256 = 'not-a-digest'; },
      (task: Record<string, any>) => { task.qeProfiles = ['sast']; },
    ];
    for (const mutate of invalidVariants) {
      const input = verifierOnlyTaskInput();
      mutate(input);
      expect(() => parseAcceptanceTask(input, SECURE_HARNESS_CONFIG)).toThrow();
    }
  });
});
