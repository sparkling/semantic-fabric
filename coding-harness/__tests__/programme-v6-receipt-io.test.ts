// SPDX-License-Identifier: MIT

import {
  chmodSync, existsSync, linkSync, lstatSync, mkdirSync, mkdtempSync, rmSync,
  symlinkSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { programmeV5AuthorityClaimPath } from '../src/programme-v5-receipt-io.js';
import {
  programmeV6ArtifactPath,
  programmeV6AuthorityClaimPath,
  readProgrammeV6PrivateArtifact,
  requireProgrammeV6ArtifactPath,
  writeProgrammeV6PrivateArtifact,
} from '../src/programme-v6-receipt-io.js';

const roots: string[] = [];
const RUN_ID = 'programme_v6_receipt_io';

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme-v6 private receipt IO', () => {
  it('writes stable private bytes once and rejects hard-linked artifacts', () => {
    const repository = fixtureRepository();
    const path = programmeV6ArtifactPath(repository, RUN_ID, 'execution');
    writeProgrammeV6PrivateArtifact(repository, path, '{"schemaVersion":6}\n');

    expect(readProgrammeV6PrivateArtifact(repository, path)).toBe('{"schemaVersion":6}\n');
    expect(lstatSync(path).mode & 0o777).toBe(0o600);
    expect(() => writeProgrammeV6PrivateArtifact(repository, path, '{}\n'))
      .toThrow('HARNESS_PROGRAMME_V6_RECEIPT_EXISTS');

    const linked = programmeV6ArtifactPath(repository, RUN_ID, 'replay');
    linkSync(path, linked);
    expect(() => readProgrammeV6PrivateArtifact(repository, path))
      .toThrow('HARNESS_PROGRAMME_V6_ARTIFACT_INVALID');
  });

  it('requires the exact run-scoped path and a V6-separated claim namespace', () => {
    const repository = fixtureRepository();
    const path = programmeV6ArtifactPath(repository, RUN_ID, 'policy-review');
    expect(requireProgrammeV6ArtifactPath(repository, RUN_ID, 'policy-review', path)).toBe(path);
    expect(() => requireProgrammeV6ArtifactPath(
      repository, RUN_ID, 'replay', path,
    )).toThrow('HARNESS_PROGRAMME_V6_ARTIFACT_PATH_INVALID');

    const authority = temporary('programme-v6-io-authority-');
    const key = 'a'.repeat(64);
    expect(programmeV6AuthorityClaimPath(key, authority))
      .toBe(join(authority, 'programme-v6-claims', `${key}.json`));
    expect(programmeV6AuthorityClaimPath(key, authority))
      .not.toBe(programmeV5AuthorityClaimPath(key, authority));
  });

  it('rejects a symlinked result ancestor without writing outside the repository', () => {
    const repository = fixtureRepository();
    const outside = temporary('programme-v6-io-outside-');
    symlinkSync(outside, join(repository, 'coding-harness', '.metaharness'));
    const path = join(
      repository, 'coding-harness', '.metaharness', 'runs', `${RUN_ID}.json`,
    );

    expect(() => writeProgrammeV6PrivateArtifact(repository, path, '{}\n'))
      .toThrow('HARNESS_PROGRAMME_V6_RESULT_ROOT_INVALID');
    expect(existsSync(join(outside, 'runs'))).toBe(false);
  });
});

function fixtureRepository(): string {
  const repository = temporary('programme-v6-io-repository-');
  mkdirSync(join(repository, 'coding-harness'), { mode: 0o755 });
  return repository;
}

function temporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  chmodSync(root, 0o700);
  roots.push(root);
  return root;
}
