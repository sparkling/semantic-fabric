// SPDX-License-Identifier: MIT

import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { issue8MaskedWorkspacePaths } from '../src/issue-8-native-session.js';

const TASK = 'coding-harness/config/issue-8-acceptance.json';
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('issue #8 native task masking', () => {
  it('masks the entire task configuration directory when it is present', () => {
    const candidate = temporary('native-task-candidate-');
    mkdirSync(join(candidate, 'coding-harness', 'config'), { recursive: true });

    expect(issue8MaskedWorkspacePaths(candidate, ['tests/secret.rs'], TASK)).toEqual([
      'tests/secret.rs',
      'coding-harness/config',
    ]);
  });

  it('keeps evaluator masks when a historical candidate has no task configuration', () => {
    const candidate = temporary('native-task-legacy-');

    expect(issue8MaskedWorkspacePaths(
      candidate,
      ['tests/secret.rs', 'tests/secret.rs'],
      TASK,
    )).toEqual(['tests/secret.rs']);
  });

  it('rejects malformed task paths before inspecting the workspace', () => {
    const candidate = temporary('native-task-invalid-');

    expect(() => issue8MaskedWorkspacePaths(candidate, [], '../task.json')).toThrow();
  });

  it('rejects a symlinked task configuration boundary', () => {
    const candidate = temporary('native-task-symlink-');
    const target = temporary('native-task-target-');
    mkdirSync(join(candidate, 'coding-harness'), { recursive: true });
    symlinkSync(target, join(candidate, 'coding-harness', 'config'));

    expect(() => issue8MaskedWorkspacePaths(candidate, [], TASK))
      .toThrow('HARNESS_NATIVE_TASK_CONFIG_MASK_INVALID');
  });

  it('rejects a non-directory task boundary and malformed evaluator masks', () => {
    const candidate = temporary('native-task-file-');
    const legacy = temporary('native-task-mask-invalid-');
    mkdirSync(join(candidate, 'coding-harness'), { recursive: true });
    writeFileSync(join(candidate, 'coding-harness', 'config'), 'not a directory');

    expect(() => issue8MaskedWorkspacePaths(candidate, [], TASK))
      .toThrow('HARNESS_NATIVE_TASK_CONFIG_MASK_INVALID');
    for (const evaluatorPath of ['/absolute.rs', '../escape.rs', 'tests\\secret.rs']) {
      expect(() => issue8MaskedWorkspacePaths(legacy, [evaluatorPath], TASK)).toThrow();
    }
  });
});

function temporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  roots.push(root);
  return root;
}
