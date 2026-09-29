// SPDX-License-Identifier: MIT
import type { DeliveryRun } from './delivery-runtime.js';

/** Explicit read closure never drops evaluator/runtime/package dependencies. */
export function requiredDeliveryInputs(files: Record<string, string>, checks: DeliveryRun['task']['checks']): string[] {
  if (checks.some(c => c.argv[0] === 'cargo')) return Object.keys(files);
  const scripts = checks.filter(c => c.argv[0] === 'node').map(c => `${c.cwd}/${c.argv[1]}`);
  return Object.keys(files).filter(path => scripts.includes(path) || /^(coding-harness|scripts|config)\//.test(path)
    || /(^|\/)(?:[^/]*lock[^/]*|package\.json|Cargo\.toml|tsconfig[^/]*|\.gitignore|\.gitattributes)$/.test(path));
}
