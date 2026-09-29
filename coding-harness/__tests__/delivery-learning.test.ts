import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { openDeliveryLearning } from '../src/delivery-learning.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
it('persists native evidence, pins assigned author, and refuses hybrid labels and tampering', async () => {
  const { task } = workflowFixture(roots), directory = mkdtempSync(join(tmpdir(), 'fabric-learning-')); roots.push(directory);
  const first = await openDeliveryLearning(directory, 'run-one', task, native);
  await first.record(native, 'implementation', 10, 'a'.repeat(64));
  const next = await openDeliveryLearning(directory, 'run-two', task, native);
  expect(next.summary().observations).toBe(1);
  const sonnet = { host: 'claude-code' as const, model: 'cc/claude-sonnet-5-5[1m]', effort: 'medium' as const };
  expect(next.selectCreditFallback()).toEqual(sonnet);
  await next.record(sonnet, 'implementation', 100, 'b'.repeat(64));
  const learned = await openDeliveryLearning(directory, 'run-three', task, native);
  expect(learned.selectCreditFallback()).toMatchObject({ host: 'codex', model: 'gpt-5.6-sol', effort: 'medium' });
  const hybrid = await openDeliveryLearning(directory, 'hybrid', { ...task, host: 'openrouter' }, native);
  await expect(hybrid.record(native, 'implementation', 1, 'b'.repeat(64))).rejects.toThrow('DELIVERY_HYBRID_LEARNING_REFUSED');
  const file = join(directory, 'deltas', readdirSync(join(directory, 'deltas'))[0]);
  const row = JSON.parse(readFileSync(file, 'utf8')); row.receiptDigest = 'c'.repeat(64); writeFileSync(file, JSON.stringify(row));
  await expect(openDeliveryLearning(directory, 'tamper', task, native)).rejects.toThrow('DELIVERY_LEARNING_DELTA_INVALID');
});
