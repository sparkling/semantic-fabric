// SPDX-License-Identifier: MIT

import { chmodSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { ISSUE_8_SYSTEM_PATHS } from '../src/issue-8-system.js';
import {
  PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY,
  PROGRAMME_V5_AGENTIC_QE_VERSION,
  attestProgrammeV5SystemTools,
  stableProgrammeV5SystemFileDigest,
} from '../src/programme-v5-system.js';
import { assertTaskQeRunnerIdentity } from '../src/task-qe.js';

let evidence: Readonly<Record<string, string>>;
const roots: string[] = [];

beforeAll(() => {
  evidence = attestProgrammeV5SystemTools();
});

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme-v5 system evidence', () => {
  it('attests the exact live system key set and v5 digests', () => {
    expect(evidence).toEqual({
      cargo: `${ISSUE_8_SYSTEM_PATHS.cargo}#sha256:f30f9fd1b1d0b8fd10dc33219eb4cd4bec3543f40e434ac71f5a03fd0359063f`,
      cargoLlvmCov: `${ISSUE_8_SYSTEM_PATHS.cargoLlvmCov}#sha256:c59831d34b46a3e3a3dc5b357fa12f75eb0af3172f8e9e81a6fc1412cdbcaa1a`,
      node: `${ISSUE_8_SYSTEM_PATHS.node}#sha256:53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc`,
      codexExecutable: `${ISSUE_8_SYSTEM_PATHS.codex}#sha256:73dc5888888f411c1f0fa7b81d866e721dcc86b527ce8e3b2cf4708661e823ba`,
      claudeExecutable: `${ISSUE_8_SYSTEM_PATHS.claude}#sha256:3473601ea695d5bf769c5b202844d4cb4fbf723ae995450fcb6973204775c84a`,
      bwrap: `${ISSUE_8_SYSTEM_PATHS.bwrap}#sha256:52231e1caf55bcbc667b269f49c63599a6f7db4767ae6a039580d0ff853db712`,
      systemdRun: `${ISSUE_8_SYSTEM_PATHS.systemdRun}#sha256:dbc8b988a849d5c9d7ef2de7068a6f107021bc6c11e0d7864c73f373eef726a7`,
      systemctl: `${ISSUE_8_SYSTEM_PATHS.systemctl}#sha256:e0d3d0e9444da1b2b58c792c3f5028b69f049b77d5ca17b3ec0d09f89117225b`,
      caBundle: `${ISSUE_8_SYSTEM_PATHS.caBundle}#sha256:6602a85a36afc2e51c66a0df5ae3d383c5b7c2fed93339ccef7d37e01faf09e8`,
      agenticQeMcp: `${ISSUE_8_SYSTEM_PATHS.agenticQeMcp}#sha256:a07f22e29ff2dd074e05b30ccdaf76ce042418e6a879d83807e9fdd722dfa483`,
      agenticQe: '3.13.12#sast-only-flat-v1+lcov-gap',
      agenticQePackageTreeDigest:
        '0e7497a02997c9c43c2dbe9c200ce016c8a6c345b8fdb5d5ee99d61ff8722884',
    });
    expect(PROGRAMME_V5_AGENTIC_QE_VERSION)
      .toBe('3.13.12#sast-only-flat-v1+lcov-gap');
    expect(Object.isFrozen(evidence)).toBe(true);
    expect(evidence).not.toHaveProperty('codex');
    expect(evidence).not.toHaveProperty('claude');
  });

  it('accepts the live v5 identity while the unchanged v4 default rejects it', () => {
    const live = {
      package: {
        version: evidence.agenticQe.split('#')[0],
        treeSha256: evidence.agenticQePackageTreeDigest,
      },
    };
    expect(() => assertTaskQeRunnerIdentity(
      live,
      PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY,
    )).not.toThrow();
    expect(() => assertTaskQeRunnerIdentity(live))
      .toThrow('HARNESS_TASK_QE_IDENTITY_MISMATCH');
  });

  it('fails closed when the selected v5 package identity drifts', () => {
    expect(() => assertTaskQeRunnerIdentity({
      package: {
        version: '3.13.13',
        treeSha256: PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY.treeSha256,
      },
    }, PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY))
      .toThrow('HARNESS_TASK_QE_IDENTITY_MISMATCH');
    expect(() => assertTaskQeRunnerIdentity({
      package: {
        version: PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY.version,
        treeSha256: 'f'.repeat(64),
      },
    }, PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY))
      .toThrow('HARNESS_TASK_QE_IDENTITY_MISMATCH');
  });

  it.each([
    ['group', 0o775],
    ['world', 0o757],
  ])('rejects a %s-writable system executable', (_scope, mode) => {
    const root = mkdtempSync(join(tmpdir(), 'programme-v5-system-'));
    roots.push(root);
    const executable = join(root, 'tool');
    writeFileSync(executable, '#!/bin/sh\nexit 0\n');
    chmodSync(executable, mode);

    expect(() => stableProgrammeV5SystemFileDigest(executable, true))
      .toThrow('HARNESS_PROGRAMME_V5_SYSTEM_TOOL_UNTRUSTED');
  });
});
