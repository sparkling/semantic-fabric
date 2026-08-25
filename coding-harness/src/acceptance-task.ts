// SPDX-License-Identifier: MIT

import type { HarnessConfig, StructuredCommand } from './contracts.js';
import {
  DEVELOPMENT_AUTHORITY,
  asInteger,
  asNonEmptyString,
  asRecord,
  asUniqueStrings,
  assertExactKeys,
  assertStructuredText,
  deepFreeze,
  normalizeWorkspacePath,
  parseStructuredCommand,
  pathsOverlap,
} from './contracts.js';

const GIT_OBJECT_ID = /^[a-f0-9]{40}$/;
const OPAQUE_ID = /^[A-Za-z0-9_-]{8,128}$/;
const TEST_NAME = /^[A-Za-z_][A-Za-z0-9_:]*$/;
const REQUIRED_NATIVE_HOSTS = ['codex', 'claude-code'] as const;

export interface GitObjectIdentity {
  commit: string;
  tree: string;
}

export interface NamedAcceptanceCommand {
  commandId: string;
  command: StructuredCommand;
}

export interface MutationAcceptanceCommand {
  mutationId: string;
  path: string;
  search: string;
  replacement: string;
  command: StructuredCommand;
}

export interface AcceptanceTask {
  schemaVersion: 1;
  taskId: string;
  issue: 8;
  authority: typeof DEVELOPMENT_AUTHORITY;
  baseline: GitObjectIdentity;
  sourceFix: GitObjectIdentity;
  evaluatorPaths: string[];
  implementationPaths: string[];
  artifactPaths: string[];
  tools: string[];
  redBaseline: {
    commands: NamedAcceptanceCommand[];
    expected: {
      exitCode: 101;
      failedTests: string[];
    };
  };
  commands: {
    build: NamedAcceptanceCommand[];
    public: NamedAcceptanceCommand[];
    independent: NamedAcceptanceCommand[];
    regression: NamedAcceptanceCommand[];
    mutation: MutationAcceptanceCommand[];
  };
  policy: {
    candidateNetwork: 'offline';
    modelTransport: 'native-first-party-only';
    nativeHosts: ['codex', 'claude-code'];
  };
  evolutionEligible: false;
}

export function parseAcceptanceTask(value: unknown, config: HarnessConfig): AcceptanceTask {
  const input = asRecord(value, 'acceptance task');
  assertExactKeys(input, [
    'schemaVersion', 'taskId', 'issue', 'authority', 'baseline', 'sourceFix',
    'evaluatorPaths', 'implementationPaths', 'artifactPaths', 'tools', 'redBaseline',
    'commands', 'policy', 'evolutionEligible',
  ], 'acceptance task');
  if (input.schemaVersion !== 1) throw new TypeError('acceptance task.schemaVersion must be 1');
  if (input.issue !== 8) throw new TypeError('acceptance task.issue must be 8');
  if (input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('acceptance task.authority cannot grant promotion');
  }
  if (input.evolutionEligible !== false) {
    throw new TypeError('acceptance task.evolutionEligible must remain false');
  }

  const taskId = parseOpaqueId(input.taskId, 'acceptance task.taskId');
  const baseline = parseGitIdentity(input.baseline, 'acceptance task.baseline');
  const sourceFix = parseGitIdentity(input.sourceFix, 'acceptance task.sourceFix');
  if (baseline.commit === sourceFix.commit || baseline.tree === sourceFix.tree) {
    throw new TypeError('acceptance task baseline and source fix identities must differ');
  }

  const evaluatorPaths = parsePaths(input.evaluatorPaths, 'acceptance task.evaluatorPaths');
  const implementationPaths = parsePaths(input.implementationPaths, 'acceptance task.implementationPaths');
  assertDisjointPaths(evaluatorPaths, implementationPaths, 'evaluator and implementation paths must not overlap');
  const artifactPaths = parsePaths(input.artifactPaths, 'acceptance task.artifactPaths');
  if (artifactPaths.some((path) => !implementationPaths.includes(path))) {
    throw new TypeError('acceptance task artifact must be a tracked implementation path');
  }

  const tools = asUniqueStrings(input.tools, 'acceptance task.tools');
  for (const tool of tools) {
    assertStructuredText(tool, 'acceptance task.tools entry');
    if (!config.allowedTools.includes(tool)) {
      throw new TypeError(`acceptance task tool "${tool}" is not configured`);
    }
  }

  const redBaseline = parseRedBaseline(input.redBaseline, config, tools);
  const commands = parseCommandGroups(input.commands, config, tools, implementationPaths);
  const allCommandIds = [
    ...redBaseline.commands.map(({ commandId }) => commandId),
    ...commands.build.map(({ commandId }) => commandId),
    ...commands.public.map(({ commandId }) => commandId),
    ...commands.independent.map(({ commandId }) => commandId),
    ...commands.regression.map(({ commandId }) => commandId),
  ];
  if (new Set(allCommandIds).size !== allCommandIds.length) {
    throw new TypeError('acceptance task commandId values must be globally unique');
  }
  const mutationIds = commands.mutation.map(({ mutationId }) => mutationId);
  if (new Set(mutationIds).size !== mutationIds.length) {
    throw new TypeError('acceptance task mutationId values must be unique');
  }
  const mutationSearches = commands.mutation.map(({ path, search }) => `${path}\0${search}`);
  if (new Set(mutationSearches).size !== mutationSearches.length) {
    throw new TypeError('acceptance task mutation path/search pairs must be unique');
  }

  const policy = parsePolicy(input.policy);
  return deepFreeze({
    schemaVersion: 1,
    taskId,
    issue: 8,
    authority: DEVELOPMENT_AUTHORITY,
    baseline,
    sourceFix,
    evaluatorPaths,
    implementationPaths,
    artifactPaths,
    tools,
    redBaseline,
    commands,
    policy,
    evolutionEligible: false,
  });
}

function parseGitIdentity(value: unknown, label: string): GitObjectIdentity {
  const input = asRecord(value, label);
  assertExactKeys(input, ['commit', 'tree'], label);
  const commit = asNonEmptyString(input.commit, `${label}.commit`);
  const tree = asNonEmptyString(input.tree, `${label}.tree`);
  if (!GIT_OBJECT_ID.test(commit)) throw new TypeError(`${label}.commit must be a lowercase 40-character Git ID`);
  if (!GIT_OBJECT_ID.test(tree)) throw new TypeError(`${label}.tree must be a lowercase 40-character Git ID`);
  return { commit, tree };
}

function parsePaths(value: unknown, label: string): string[] {
  return asUniqueStrings(value, label)
    .map((path, index) => normalizeWorkspacePath(path, `${label}[${index}]`));
}

function assertDisjointPaths(left: readonly string[], right: readonly string[], message: string): void {
  if (left.some((leftPath) => right.some((rightPath) => pathsOverlap(leftPath, rightPath)))) {
    throw new TypeError(message);
  }
}

function parseRedBaseline(
  value: unknown,
  config: HarnessConfig,
  tools: readonly string[],
): AcceptanceTask['redBaseline'] {
  const input = asRecord(value, 'acceptance task.redBaseline');
  assertExactKeys(input, ['commands', 'expected'], 'acceptance task.redBaseline');
  const commands = parseNamedCommands(input.commands, 'acceptance task.redBaseline.commands', config, tools);
  const expected = asRecord(input.expected, 'acceptance task.redBaseline.expected');
  assertExactKeys(expected, ['exitCode', 'failedTests'], 'acceptance task.redBaseline.expected');
  if (asInteger(expected.exitCode, 'acceptance task.redBaseline.expected.exitCode', 1) !== 101) {
    throw new TypeError('acceptance task.redBaseline.expected.exitCode must be Cargo failure code 101');
  }
  const failedTests = asUniqueStrings(
    expected.failedTests,
    'acceptance task.redBaseline.expected.failedTests',
  );
  for (const testName of failedTests) {
    if (!TEST_NAME.test(testName)) throw new TypeError(`invalid red-baseline test name: ${testName}`);
  }
  return { commands, expected: { exitCode: 101, failedTests } };
}

function parseCommandGroups(
  value: unknown,
  config: HarnessConfig,
  tools: readonly string[],
  implementationPaths: readonly string[],
): AcceptanceTask['commands'] {
  const input = asRecord(value, 'acceptance task.commands');
  assertExactKeys(input, ['build', 'public', 'independent', 'regression', 'mutation'], 'acceptance task.commands');
  const mutationInput = requireNonEmptyArray(input.mutation, 'acceptance task.commands.mutation');
  return {
    build: parseNamedCommands(input.build, 'acceptance task.commands.build', config, tools),
    public: parseNamedCommands(input.public, 'acceptance task.commands.public', config, tools),
    independent: parseNamedCommands(input.independent, 'acceptance task.commands.independent', config, tools),
    regression: parseNamedCommands(input.regression, 'acceptance task.commands.regression', config, tools),
    mutation: mutationInput.map((entry, index) => {
      const label = `acceptance task.commands.mutation[${index}]`;
      const item = asRecord(entry, label);
      assertExactKeys(item, ['mutationId', 'path', 'search', 'replacement', 'command'], label);
      const path = normalizeWorkspacePath(item.path, `${label}.path`);
      if (!implementationPaths.includes(path)) {
        throw new TypeError(`${label}.path must be an implementation path`);
      }
      const search = parseTransformText(item.search, `${label}.search`);
      const replacement = parseTransformText(item.replacement, `${label}.replacement`);
      if (search === replacement) throw new TypeError(`${label} transform must change the source`);
      return {
        mutationId: parseOpaqueId(item.mutationId, `${label}.mutationId`),
        path,
        search,
        replacement,
        command: parseOfflineCommand(item.command, config, tools),
      };
    }),
  };
}

function parseNamedCommands(
  value: unknown,
  label: string,
  config: HarnessConfig,
  tools: readonly string[],
): NamedAcceptanceCommand[] {
  return requireNonEmptyArray(value, label).map((entry, index) => {
    const itemLabel = `${label}[${index}]`;
    const item = asRecord(entry, itemLabel);
    assertExactKeys(item, ['commandId', 'command'], itemLabel);
    return {
      commandId: parseOpaqueId(item.commandId, `${itemLabel}.commandId`),
      command: parseOfflineCommand(item.command, config, tools),
    };
  });
}

function parseOfflineCommand(
  value: unknown,
  config: HarnessConfig,
  tools: readonly string[],
): StructuredCommand {
  const command = parseStructuredCommand(value, config, tools);
  if (command.tool !== 'cargo' || command.executable !== 'cargo') {
    throw new TypeError('acceptance task commands must invoke Cargo directly');
  }
  if (command.cwd !== '.') throw new TypeError('acceptance task commands must execute from the candidate root');
  if (command.env.CARGO_NET_OFFLINE !== 'true') {
    throw new TypeError('acceptance task commands must set CARGO_NET_OFFLINE=true');
  }
  if (command.argv[0] !== '--offline') {
    throw new TypeError('acceptance task Cargo commands must begin with --offline');
  }
  const cargoSubcommand = command.argv[1];
  if (cargoSubcommand === undefined) throw new TypeError('acceptance task Cargo command needs a subcommand');
  if (cargoSubcommand === 'fmt') {
    if (command.argv.includes('--locked')) throw new TypeError('cargo fmt must not use --locked');
  } else if (!command.argv.includes('--locked')) {
    throw new TypeError('acceptance task Cargo commands must use --locked');
  }
  return command;
}

function parsePolicy(value: unknown): AcceptanceTask['policy'] {
  const input = asRecord(value, 'acceptance task.policy');
  assertExactKeys(input, ['candidateNetwork', 'modelTransport', 'nativeHosts'], 'acceptance task.policy');
  if (input.candidateNetwork !== 'offline') throw new TypeError('candidate network policy must be offline');
  if (input.modelTransport !== 'native-first-party-only') {
    throw new TypeError('model transport must be native-first-party-only');
  }
  const nativeHosts = asUniqueStrings(input.nativeHosts, 'acceptance task.policy.nativeHosts');
  if (nativeHosts.length !== REQUIRED_NATIVE_HOSTS.length
    || REQUIRED_NATIVE_HOSTS.some((host) => !nativeHosts.includes(host))) {
    throw new TypeError('acceptance task requires both codex and claude-code native hosts');
  }
  return { candidateNetwork: 'offline', modelTransport: 'native-first-party-only', nativeHosts: [...REQUIRED_NATIVE_HOSTS] };
}

function parseOpaqueId(value: unknown, label: string): string {
  const id = asNonEmptyString(value, label);
  if (!OPAQUE_ID.test(id)) throw new TypeError(`${label} must be an opaque 8-128 character ID`);
  return id;
}

function parseTransformText(value: unknown, label: string): string {
  const text = asNonEmptyString(value, label);
  if (text.includes('\0') || text.includes('\r')) {
    throw new TypeError(`${label} must use normalized LF source text`);
  }
  return text;
}

function requireNonEmptyArray(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value) || value.length === 0) throw new TypeError(`${label} must be a non-empty array`);
  return value;
}
