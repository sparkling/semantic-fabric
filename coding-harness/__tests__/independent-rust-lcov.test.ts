// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import {
  CARGO_LLVM_COV_VERSION,
  IndependentRustLcovGenerator,
} from '../src/independent-rust-lcov.js';
import type { OfflineProcessIsolator } from '../src/network.js';

const roots: string[] = [];
const isolator: OfflineProcessIsolator = Object.freeze({
  assertStable() {},
  isolate: (command) => ({
    enforcement: 'os-network-namespace',
    mechanism: 'test-offline-wrapper',
    command: {
      ...command,
      executable: '/usr/bin/env',
      args: [command.executable, ...command.args],
    },
  }),
});

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('independent Rust LCOV generation', () => {
  it('binds a real direct coverage command, exact tool version, tree, and LCOV bytes', async () => {
    const fixture = repository();
    const generator = createGenerator(fixture.executable, 'issue_8_binding_pruning');

    const artifact = await generator.capture(fixture.input);

    expect(artifact).toMatchObject({
      provenance: 'independent-direct-coverage',
      generatorVersion: CARGO_LLVM_COV_VERSION,
    });
    expect(artifact.sha256).toMatch(/^[a-f0-9]{64}$/);
    expect(artifact.coverageCommandDigest).toMatch(/^[a-f0-9]{64}$/);
    expect(readFileSync(artifact.path, 'utf8')).toContain('SF:src/lib.rs');
    expect(lstatSync(artifact.path).mode & 0o222).toBe(0);
    expect(git(fixture.candidateRoot, ['write-tree'])).toBe(fixture.candidateTree);
  });

  it('fails closed if direct coverage mutates candidate source', async () => {
    const fixture = repository();
    const generator = createGenerator(fixture.executable, 'mutating_target');

    await expect(generator.capture(fixture.input)).rejects.toThrow(
      'HARNESS_INDEPENDENT_LCOV_CANDIDATE_CHANGED',
    );
  });

  it('rejects any coverage generator version other than the frozen release', async () => {
    const fixture = repository('cargo-llvm-cov 0.8.6');
    const generator = createGenerator(fixture.executable, 'issue_8_binding_pruning');

    await expect(generator.capture(fixture.input)).rejects.toThrow(
      'HARNESS_INDEPENDENT_LCOV_VERSION_MISMATCH',
    );
  });
});

function createGenerator(executable: string, testTarget: string): IndependentRustLcovGenerator {
  return new IndependentRustLcovGenerator({
    config: SECURE_HARNESS_CONFIG,
    rustProfile: {
      cargoExecutable: executable,
      environment: {
        PATH: '/usr/bin',
        HOME: '/home/harness',
        CARGO_HOME: '/cargo-home',
        CARGO_NET_OFFLINE: 'true',
        CARGO_INCREMENTAL: '0',
      },
      isolator,
    },
    packageName: 'sf-conformance',
    testTarget,
    timeoutMs: 5_000,
    maxOutputBytes: 100_000,
  });
}

function repository(version = CARGO_LLVM_COV_VERSION): Readonly<{
  controlledRoot: string;
  candidateRoot: string;
  outputRoot: string;
  candidateTree: string;
  executable: string;
  input: {
    controlledRoot: string;
    candidateRoot: string;
    outputRoot: string;
    candidateTree: string;
  };
}> {
  const controlledRoot = mkdtempSync(join(tmpdir(), 'semantic-fabric-direct-lcov-'));
  roots.push(controlledRoot);
  const candidateRoot = join(controlledRoot, 'candidate');
  const outputRoot = join(controlledRoot, 'outputs', 'independent');
  const binRoot = join(controlledRoot, 'bin');
  mkdirSync(join(candidateRoot, 'src'), { recursive: true, mode: 0o700 });
  mkdirSync(outputRoot, { recursive: true, mode: 0o700 });
  mkdirSync(binRoot, { mode: 0o700 });
  writeFileSync(join(candidateRoot, 'src/lib.rs'), 'pub fn candidate() -> bool { true }\n');
  git(candidateRoot, ['init', '--quiet']);
  git(candidateRoot, ['add', '--', 'src/lib.rs']);
  git(candidateRoot, ['commit', '--quiet', '-m', 'candidate'], identityEnvironment());
  const candidateTree = git(candidateRoot, ['write-tree']);
  const executable = join(binRoot, 'cargo');
  writeFileSync(executable, fakeCargo(version));
  chmodSync(executable, 0o500);
  return Object.freeze({
    controlledRoot,
    candidateRoot,
    outputRoot,
    candidateTree,
    executable,
    input: { controlledRoot, candidateRoot, outputRoot, candidateTree },
  });
}

function fakeCargo(version: string): string {
  return `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
if (args[0] !== 'llvm-cov') process.exit(91);
if (args[1] === '--version') {
  process.stdout.write(${JSON.stringify(`${version}\n`)});
  process.exit(0);
}
const outputIndex = args.indexOf('--output-path');
const testIndex = args.indexOf('--test');
if (outputIndex < 0 || testIndex < 0 || !args.includes('--offline') || !args.includes('--locked')) {
  process.exit(92);
}
fs.writeFileSync(args[outputIndex + 1], 'TN:\\nSF:src/lib.rs\\nDA:1,1\\nend_of_record\\n');
if (args[testIndex + 1] === 'mutating_target') {
  fs.writeFileSync('src/lib.rs', 'pub fn mutated() -> bool { false }\\n');
}
`;
}

function git(
  cwd: string,
  args: readonly string[],
  environment: NodeJS.ProcessEnv = process.env,
): string {
  const result = spawnSync('/usr/bin/git', args, {
    cwd,
    env: environment,
    encoding: 'utf8',
  });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
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
