// SPDX-License-Identifier: MIT
import type { DeliveryRun } from './delivery-runtime.js';

/** Explicit read closure never drops evaluator/runtime/package dependencies. */
export function requiredDeliveryInputs(files: Record<string, string>, checks: DeliveryRun['task']['checks']): string[] {
  // Cargo's implicit source inputs are rechecked on combined source, not frozen canonical read locks.
  const scripts = checks.filter(c => c.argv[0] === 'node').map(c => `${c.cwd}/${c.argv[1]}`);
  return Object.keys(files).filter(path => scripts.includes(path) || /^(coding-harness|scripts|config|tests)\//.test(path)
    || /(^|\/)(?:\.cargo|tests|benches)\//.test(path)
    || /(^|\/)(?:[^/]*lock[^/]*|package\.json|Cargo\.toml|build\.rs|rust-toolchain(?:\.toml)?|tsconfig[^/]*|\.gitignore|\.gitattributes)$/.test(path));
}
