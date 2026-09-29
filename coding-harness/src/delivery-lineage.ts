// SPDX-License-Identifier: MIT
import type { DeliveryRun } from './delivery-runtime.js';

// Harness code and policy execute from the canonical checkout beside private candidates.
const LIVE_RUNTIME = /^(coding-harness|scripts|config)\//;

/** Explicit read closure never drops evaluator/runtime/package dependencies. */
export function requiredDeliveryInputs(files: Record<string, string>, checks: DeliveryRun['task']['checks']): string[] {
  // Cargo's implicit source inputs are rechecked on combined source, not frozen canonical read locks.
  const scripts = checks.filter(c => c.argv[0] === 'node').map(c => `${c.cwd}/${c.argv[1]}`);
  return Object.keys(files).filter(path => scripts.includes(path) || LIVE_RUNTIME.test(path) || /^tests\//.test(path)
    || /(^|\/)(?:\.cargo|tests|benches)\//.test(path)
    || /(^|\/)(?:[^/]*lock[^/]*|package\.json|Cargo\.toml|build\.rs|rust-toolchain(?:\.toml)?|tsconfig[^/]*|\.gitignore|\.gitattributes)$/.test(path));
}
/** Private candidate checks read their own snapshot; integration re-pins those inputs via requiredDeliveryInputs. */
export function liveDeliveryRuntimeInputs(paths: string[]): string[] {
  return paths.filter(path => LIVE_RUNTIME.test(path));
}
