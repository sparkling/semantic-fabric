// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  bindAcceptanceTaskToRustProfile,
  parseAcceptanceTask,
  type AcceptanceTask,
} from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import type { StructuredCommand } from '../src/contracts.js';
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

describe('issue #8 acceptance task', () => {
  it('parses the canonical shell-free task and freezes every nested value', () => {
    const task = parseAcceptanceTask(taskInput(), SECURE_HARNESS_CONFIG);

    expect(task).toMatchObject({
      schemaVersion: 1,
      issue: 8,
      authority: 'development-only-no-promotion',
      baseline: {
        commit: 'd510fc952a8dc701d65b1a4f3ad25a8109b98669',
        tree: 'b5d67e0fdb107e6502959fd2ff36831170d093b1',
      },
      sourceFix: {
        commit: '10dedd40bda63d3acef18b8d34f61a32214e98d4',
        tree: 'a3f637f6b14fff73e5209e539b7a19f0b6b73ffa',
      },
      policy: {
        candidateNetwork: 'offline',
        modelTransport: 'native-first-party-only',
        nativeHosts: ['codex', 'claude-code'],
      },
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
    expect(Object.isFrozen(task)).toBe(true);
    expect(Object.isFrozen(task.commands.mutation[0].command.argv)).toBe(true);
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

  it('binds each mutation to one exact source-fix transform back to baseline code', () => {
    const task = parseAcceptanceTask(taskInput(), SECURE_HARNESS_CONFIG);
    for (const mutation of task.commands.mutation) {
      const sourceFix = execFileSync(
        'git',
        ['-C', repositoryRoot, 'show', `${task.sourceFix.commit}:${mutation.path}`],
        { encoding: 'utf8' },
      );
      const baseline = execFileSync(
        'git',
        ['-C', repositoryRoot, 'show', `${task.baseline.commit}:${mutation.path}`],
        { encoding: 'utf8' },
      );
      expect(sourceFix.split(mutation.search)).toHaveLength(2);
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
    ]) {
      const input = cloneTask();
      mutate(input);
      expect(() => parseAcceptanceTask(input, SECURE_HARNESS_CONFIG)).toThrow();
    }
  });
});
