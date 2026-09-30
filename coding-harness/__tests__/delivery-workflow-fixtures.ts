// SPDX-License-Identifier: MIT
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DeliveryHarness } from '../src/delivery-runtime.js';
import type { DeliveryTask } from '../src/delivery-contracts.js';
import type { NativeStageRequest, NativeStageResponse } from '../src/delivery-workflow-contracts.js';
import { git, sourceSnapshot } from '../src/delivery-workspace.js';

export const native = { host: 'codex' as const, model: 'gpt-6.1-sol', effort: 'high' as const,
  executorId: 'native-sol', authentication: 'native-subscription' as const, observation: 'fixture native host metadata' };
export function workflowFixture(roots: string[], script = 'process.exit(0);\n') {
  const root = mkdtempSync(join(tmpdir(), 'sf-delivery-workflow-')); roots.push(root);
  execFileSync('git', ['init', '-b', 'main', root], { stdio: 'ignore' });
  git(root, 'config', 'user.name', 'Harness test'); git(root, 'config', 'user.email', 'test@example.invalid');
  mkdirSync(join(root, 'coding-harness')); writeFileSync(join(root, '.gitignore'), '.metaharness/\n');
  writeFileSync(join(root, 'coding-harness/check.mjs'), script); writeFileSync(join(root, 'product.txt'), 'before\n');
  git(root, 'add', '.'); git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'initial');
  const task: DeliveryTask = { schemaVersion: 1, id: 'task-1', owner: 'root', thread: 'fixture-thread',
    requirement: 'An exact scoped native task', taskClass: 'implementation', host: 'codex', scope: ['product.txt'],
    checks: [{ id: 'build', kind: 'build', argv: ['node', 'check.mjs'], cwd: 'coding-harness' },
      { id: 'public', kind: 'acceptance', argv: ['node', 'check.mjs'], cwd: 'coding-harness' }] };
  return { root, task, harness: new DeliveryHarness(root) };
}
export function responseFor(harness: DeliveryHarness, request: NativeStageRequest,
  extra: Partial<NativeStageResponse> = {}): NativeStageResponse {
  return { schemaVersion: 1, requestId: request.id, sourceDigest: sourceSnapshot(harness.root).digest,
    native: { ...native, ...request.route, executorId: request.executorId ?? 'independent-reviewer' },
    outcome: 'completed', summary: `Fixture ${request.stage} result`, issues: [], ...extra };
}
export async function nextRequest(harness: DeliveryHarness): Promise<NativeStageRequest> {
  const action = await harness.advance('task-1', 'root');
  if (action.kind !== 'native') throw new Error(`fixture expected request: ${action.kind}`);
  return action.request;
}
