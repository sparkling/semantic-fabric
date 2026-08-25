// SPDX-License-Identifier: MIT

import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import type { StructuredCommand } from '../src/contracts.js';
import type { OfflineProcessIsolator } from '../src/network.js';
import { runRepositoryCommandBatch } from '../src/repository-command-runner.js';
import { createTestConfig, TEST_RESOURCE_SCOPE } from './helpers.js';

const roots: string[] = [];
const fixtureScript = join(
  dirname(fileURLToPath(import.meta.url)),
  'fixtures/process-fixture.mjs',
);

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('repository generated-output command runner', () => {
  it('requires and binds fresh output from each configured producer', async () => {
    const fixture = makeFixture();
    const commands = [command('one'), command('two')];
    const batch = await runRepositoryCommandBatch({
      commands,
      workspaceRoot: fixture.workspace,
      controlledRoot: fixture.controlled,
      writablePaths: [fixture.output],
      outputRoot: fixture.output,
      config: createTestConfig(),
      declaredTools: ['node'],
      offlineIsolator: generatingIsolator(),
      offlineEnvironment: { PATH: process.env.PATH },
      trackedPaths: ['reports/earl.ttl'],
      generatedOutputs: commands.map((producer, index) => ({
        evidenceId: `producer-${index + 1}`,
        command: producer,
        workspacePaths: ['reports/earl.ttl'],
      })),
      candidateTree: 'a'.repeat(40),
    });

    expect(batch.commands).toHaveLength(2);
    expect(Object.keys(batch.generatedOutputDigests)).toEqual(['producer-1', 'producer-2']);
    expect(batch.generatedOutputDigests['producer-1'])
      .not.toBe(batch.generatedOutputDigests['producer-2']);
    expect(readFileSync(fixture.destination, 'utf8')).toBe('tracked report\n');
  });

  it('fails closed when the declared producer does not overwrite its poison file', async () => {
    const fixture = makeFixture();
    const producer = command('one');
    await expect(runRepositoryCommandBatch({
      commands: [producer],
      workspaceRoot: fixture.workspace,
      controlledRoot: fixture.controlled,
      writablePaths: [fixture.output],
      outputRoot: fixture.output,
      config: createTestConfig(),
      declaredTools: ['node'],
      offlineIsolator: passthroughIsolator(),
      offlineEnvironment: { PATH: process.env.PATH },
      trackedPaths: ['reports/earl.ttl'],
      generatedOutputs: [{
        evidenceId: 'producer-one', command: producer,
        workspacePaths: ['reports/earl.ttl'],
      }],
      candidateTree: 'a'.repeat(40),
    })).rejects.toThrow('HARNESS_OVERLAY_NOT_GENERATED');
  });
});

function command(label: string): StructuredCommand {
  return {
    tool: 'node', executable: process.execPath,
    argv: [fixtureScript, 'success', label], cwd: '.', env: {},
    timeoutMs: 2_000, maxOutputBytes: 10_000,
  };
}

function generatingIsolator(): OfflineProcessIsolator {
  return {
    assertStable() {},
    async terminateAndVerify() {},
    isolate(source) {
      for (const overlay of source.writableOverlays ?? []) {
        writeFileSync(overlay.source, validEarl(source.args.at(-1) ?? 'unknown'));
      }
      return wrapped(source);
    },
  };
}

function passthroughIsolator(): OfflineProcessIsolator {
  return { assertStable() {}, async terminateAndVerify() {}, isolate: wrapped };
}

function wrapped(source: Parameters<OfflineProcessIsolator['isolate']>[0]) {
  return {
    enforcement: 'os-network-namespace',
    mechanism: 'test-no-network',
    resourceScope: TEST_RESOURCE_SCOPE,
    command: {
      ...source,
      executable: '/usr/bin/env',
      args: [source.executable, ...source.args],
    },
  };
}

function makeFixture() {
  const controlled = mkdtempSync(join(tmpdir(), 'coding-harness-command-runner-'));
  roots.push(controlled);
  chmodSync(controlled, 0o700);
  const workspace = join(controlled, 'workspace');
  const output = join(controlled, 'output');
  mkdirSync(join(workspace, 'reports'), { recursive: true });
  mkdirSync(output, { mode: 0o700 });
  const destination = join(workspace, 'reports/earl.ttl');
  writeFileSync(destination, 'tracked report\n');
  return { controlled, workspace, output, destination };
}

function validEarl(label: string): string {
  return '@prefix earl: <http://www.w3.org/ns/earl#> .\n'
    + `[] a earl:Assertion ; # ${label}\n`
    + '  earl:subject <https://example.org/tools/semantic-fabric> ;\n'
    + '  earl:result [ earl:outcome earl:passed ] .\n';
}
