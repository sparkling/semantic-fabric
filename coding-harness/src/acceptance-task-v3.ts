// SPDX-License-Identifier: MIT

import type { AgenticQeProfile } from './evidence.js';
import {
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  asUniqueStrings,
  assertExactKeys,
  normalizeWorkspacePath,
  pathsOverlap,
} from './contracts.js';

const OPAQUE_ID = /^[A-Za-z0-9_-]{8,128}$/;
const EVIDENCE_ID = /^[a-z0-9][a-z0-9-]{0,63}$/;
const CARGO_PACKAGE_NAME = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;
const RUST_TEST_TARGET = /^[A-Za-z_][A-Za-z0-9_]{0,127}$/;
const GENERATED_OUTPUT_STAGES = ['public', 'independent', 'regression'] as const;
const LEGACY_QE_PROFILES = new Set<AgenticQeProfile>(['lcov-gap', 'sast']);

export const RUNTIME_PROTECTED_PATHS = Object.freeze(['Cargo.lock'] as const);

export interface SastQeBinding {
  readonly profile: 'sast';
  readonly collector: 'agentic-qe-sast';
}

export interface LcovGapQeBinding {
  readonly profile: 'lcov-gap';
  readonly collector: 'rust-lcov';
  readonly packageName: string;
  readonly testTarget: string;
}

export type TaskQeBinding = SastQeBinding | LcovGapQeBinding;
export type GeneratedOutputStage = typeof GENERATED_OUTPUT_STAGES[number];

export interface TaskGeneratedOutputBinding {
  readonly stage: GeneratedOutputStage;
  readonly evidenceId: string;
  readonly commandId: string;
  readonly workspacePaths: readonly string[];
}

export interface TaskRustEvidencePolicy {
  readonly frozenLockSha256: string;
}

export interface TaskQePolicy {
  readonly profiles: readonly TaskQeBinding[];
}

export interface TaskGeneratedEvidencePolicy {
  readonly requiredAdmittedPaths: readonly string[];
  readonly generatedOutputs: readonly TaskGeneratedOutputBinding[];
}

export function parseTaskRustPolicy(value: unknown): TaskRustEvidencePolicy {
  const input = asRecord(value, 'acceptance task.rust');
  assertExactKeys(input, ['frozenLockSha256'], 'acceptance task.rust');
  const frozenLockSha256 = asNonEmptyString(
    input.frozenLockSha256,
    'acceptance task.rust.frozenLockSha256',
  );
  if (!SHA256_PATTERN.test(frozenLockSha256)) {
    throw new TypeError('acceptance task.rust.frozenLockSha256 must be a lowercase SHA-256 digest');
  }
  return { frozenLockSha256 };
}

export function parseTaskQePolicy(value: unknown): TaskQePolicy {
  const input = asRecord(value, 'acceptance task.qe');
  assertExactKeys(input, ['profiles'], 'acceptance task.qe');
  if (!Array.isArray(input.profiles) || input.profiles.length === 0) {
    throw new TypeError('acceptance task.qe.profiles must be a non-empty array');
  }
  const profiles = input.profiles.map(parseQeBinding);
  if (new Set(profiles.map(({ profile }) => profile)).size !== profiles.length) {
    throw new TypeError('acceptance task.qe.profiles must not contain duplicate profiles');
  }
  profiles.sort((left, right) => left.profile < right.profile ? -1 : left.profile > right.profile ? 1 : 0);
  return { profiles };
}

export function parseTaskGeneratedEvidencePolicy(inputValue: Readonly<{
  value: unknown;
  commands: Readonly<Record<GeneratedOutputStage, readonly { commandId: string }[]>>;
  implementationPaths: readonly string[];
  mutationPaths: readonly string[];
  forbiddenPaths: readonly string[];
}>): TaskGeneratedEvidencePolicy {
  const input = asRecord(inputValue.value, 'acceptance task.evidence');
  assertExactKeys(
    input,
    ['requiredAdmittedPaths', 'generatedOutputs'],
    'acceptance task.evidence',
  );
  const requiredAdmittedPaths = asUniqueStrings(
    input.requiredAdmittedPaths,
    'acceptance task.evidence.requiredAdmittedPaths',
  ).map((path, index) => normalizeWorkspacePath(
    path,
    `acceptance task.evidence.requiredAdmittedPaths[${index}]`,
  ));
  if (requiredAdmittedPaths.some((path) => !inputValue.implementationPaths.includes(path))) {
    throw new TypeError('acceptance task required admitted path must be an implementation path');
  }
  const mutationPaths = [...new Set(inputValue.mutationPaths)].sort();
  if (JSON.stringify(mutationPaths) !== JSON.stringify([...requiredAdmittedPaths].sort())) {
    throw new TypeError('acceptance task mutations must exactly cover required admitted paths');
  }
  if (!Array.isArray(input.generatedOutputs)) {
    throw new TypeError('acceptance task.evidence.generatedOutputs must be an array');
  }
  const generatedOutputs = input.generatedOutputs.map((entry, index) => {
    const label = `acceptance task.evidence.generatedOutputs[${index}]`;
    const item = asRecord(entry, label);
    assertExactKeys(item, ['stage', 'evidenceId', 'commandId', 'workspacePaths'], label);
    if (!GENERATED_OUTPUT_STAGES.includes(item.stage as GeneratedOutputStage)) {
      throw new TypeError(`${label}.stage is invalid`);
    }
    const stage = item.stage as GeneratedOutputStage;
    const evidenceId = asNonEmptyString(item.evidenceId, `${label}.evidenceId`);
    if (!EVIDENCE_ID.test(evidenceId)) throw new TypeError(`${label}.evidenceId is invalid`);
    const commandId = parseOpaqueId(item.commandId, `${label}.commandId`);
    const matches = inputValue.commands[stage].filter((command) => command.commandId === commandId);
    if (matches.length !== 1) {
      throw new TypeError(`${label}.commandId must resolve exactly once in ${stage} commands`);
    }
    const workspacePaths = asUniqueStrings(item.workspacePaths, `${label}.workspacePaths`)
      .map((path, pathIndex) => normalizeWorkspacePath(path, `${label}.workspacePaths[${pathIndex}]`));
    if (workspacePaths.some((path) => inputValue.forbiddenPaths.some(
      (forbidden) => pathsOverlap(path, forbidden),
    ))) {
      throw new TypeError(`${label}.workspacePaths must not overlap governed input paths`);
    }
    return { stage, evidenceId, commandId, workspacePaths };
  });
  if (new Set(generatedOutputs.map(({ evidenceId }) => evidenceId)).size !== generatedOutputs.length) {
    throw new TypeError('acceptance task generated-output evidenceId values must be unique');
  }
  const producerBindings = generatedOutputs.map(({ stage, commandId }) => `${stage}\0${commandId}`);
  if (new Set(producerBindings).size !== producerBindings.length) {
    throw new TypeError('acceptance task generated-output command binding must be unique');
  }
  return { requiredAdmittedPaths, generatedOutputs };
}

export function qeProfilesFromBindings(bindings: readonly TaskQeBinding[]): AgenticQeProfile[] {
  return bindings.map(({ profile }) => profile);
}

export function assertTaskQeBindings(bindings: readonly TaskQeBinding[]): void {
  if (bindings.length === 0
    || new Set(bindings.map(({ profile }) => profile)).size !== bindings.length) {
    throw new Error('HARNESS_TASK_QE_BINDINGS_INVALID');
  }
  for (const binding of bindings) {
    if (binding.profile === 'sast' && binding.collector === 'agentic-qe-sast') continue;
    if (binding.profile === 'lcov-gap' && binding.collector === 'rust-lcov'
      && CARGO_PACKAGE_NAME.test(binding.packageName)
      && RUST_TEST_TARGET.test(binding.testTarget)) continue;
    throw new Error('HARNESS_TASK_QE_BINDING_INVALID');
  }
}

export function parseLegacyQeProfiles(value: unknown): AgenticQeProfile[] {
  const profiles = asUniqueStrings(value, 'acceptance task.qeProfiles');
  for (const profile of profiles) {
    if (!LEGACY_QE_PROFILES.has(profile as AgenticQeProfile)) {
      throw new TypeError(`acceptance task QE profile is invalid: ${profile}`);
    }
  }
  return profiles as AgenticQeProfile[];
}

function parseQeBinding(value: unknown, index: number): TaskQeBinding {
  const label = `acceptance task.qe.profiles[${index}]`;
  const input = asRecord(value, label);
  if (input.profile === 'sast') {
    assertExactKeys(input, ['profile', 'collector'], label);
    if (input.collector !== 'agentic-qe-sast') {
      throw new TypeError(`${label}.collector must be agentic-qe-sast`);
    }
    return { profile: 'sast', collector: 'agentic-qe-sast' };
  }
  if (input.profile === 'lcov-gap') {
    assertExactKeys(input, ['profile', 'collector', 'packageName', 'testTarget'], label);
    if (input.collector !== 'rust-lcov') {
      throw new TypeError(`${label}.collector must be rust-lcov`);
    }
    const packageName = asNonEmptyString(input.packageName, `${label}.packageName`);
    const testTarget = asNonEmptyString(input.testTarget, `${label}.testTarget`);
    if (!CARGO_PACKAGE_NAME.test(packageName)) {
      throw new TypeError(`${label}.packageName is invalid`);
    }
    if (!RUST_TEST_TARGET.test(testTarget)) {
      throw new TypeError(`${label}.testTarget is invalid`);
    }
    return { profile: 'lcov-gap', collector: 'rust-lcov', packageName, testTarget };
  }
  throw new TypeError(`${label}.profile is invalid`);
}

function parseOpaqueId(value: unknown, label: string): string {
  const id = asNonEmptyString(value, label);
  if (!OPAQUE_ID.test(id)) throw new TypeError(`${label} must be an opaque 8-128 character ID`);
  return id;
}
