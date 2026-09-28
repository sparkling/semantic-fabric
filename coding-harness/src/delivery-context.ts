// SPDX-License-Identifier: MIT
import { evidenceDirectory, git, mainRoot, sourceSnapshot, type SourceSnapshot } from './delivery-workspace.js';

/** Candidate contexts use the same lifecycle, never canonical commit authority. */
export interface DeliveryContext {
  readonly kind: 'main' | 'candidate';
  readonly root: string;
  readonly directory: string;
  assert(): void;
  head(): string;
  snapshot(): SourceSnapshot;
  dirty(scope: string[]): string[];
}

export function mainDeliveryContext(root: string): DeliveryContext {
  const actual = mainRoot(root);
  return {
    kind: 'main', root: actual, directory: evidenceDirectory(actual),
    assert: () => { mainRoot(actual); },
    head: () => git(actual, 'rev-parse', 'HEAD'),
    snapshot: () => sourceSnapshot(actual),
    dirty: scope => [...new Set([
      ...git(actual, 'diff', '--name-only', '-z', 'HEAD').split('\0'),
      ...git(actual, 'ls-files', '--others', '--exclude-standard', '-z').split('\0'),
    ].filter(path => scope.includes(path)))],
  };
}
