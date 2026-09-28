import { mkdtempSync, rmSync, readdirSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { hash } from '@metaharness/harness';
import { makeSigner } from '@metaharness/flywheel';
import { createDeliveryCandidate } from '../src/delivery-candidate.js';
import { evaluateDeliveryPolicies } from '../src/delivery-evolution.js';
import { activateDeliveryPolicy, deliveryPolicyInputDigest, deliveryPromotionRule, loadDeliveryPolicy, verifyDeliveryPolicyActivation } from '../src/delivery-policy.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';
import { runDeliveryOutcome } from '../src/delivery-runner.js';

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
it('rejects malformed quality/anchor evidence and ignores subscription cost', () => {
  const baseline = { primary: 0, noopRate: 1, regressed: false, costPerWin: 0 }, candidate = { ...baseline, primary: 1, noopRate: 0, costPerWin: Infinity };
  expect(deliveryPromotionRule({ baseline, candidate }).promote).toBe(true);
  for (const value of [NaN, Infinity, undefined, -1, 2]) expect(deliveryPromotionRule({ baseline, candidate,
    anchor: { baseline: 0, candidate: value as number } }).promote).toBe(false);
  expect(deliveryPromotionRule({ baseline, candidate: { ...candidate, regressed: undefined as unknown as boolean } }).promote).toBe(false);
});

it('evaluates actual isolated runner receipts, binds policy/suite/runtime, and rejects tampering', async () => {
  const f = workflowFixture(roots), parentDirectory = mkdtempSync(join(tmpdir(), 'fabric-policy-')); roots.push(parentDirectory);
  const signer = makeSigner();
  mkdirSync(join(f.root, 'config')); writeFileSync(join(f.root, 'config/delivery-policy.json'), JSON.stringify({ schemaVersion: 1, publicKey: signer.publicKey(), rootId: 'frozen-root' }));
  const candidate = createDeliveryCandidate(f.harness, { parentDirectory, scope: f.task.scope });
  await candidate.harness.begin(f.task); await candidate.harness.bind(f.task.id, f.task.owner, native);
  const target = candidate.harness.read(f.task.id), before = f.harness.snapshot().digest;
  const activation = await evaluateDeliveryPolicies({ canonical: f.harness, target, parentDirectory, rootId: 'frozen-root', signer,
    holdout: Array.from({ length: 5 }, (_, index) => ({ task: { ...f.task, id: index ? `held-${index}` : f.task.id }, handoff: native })),
    anchor: Array.from({ length: 5 }, (_, index) => ({ task: { ...f.task, id: `anchor-${index}` }, handoff: native })), maxGenerations: 1,
    proposer: async () => 'Improved concrete policy guidance', execute: async (request, _files, context) => {
      const improved = JSON.stringify(context).includes('Improved concrete policy guidance');
      return { response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
        native: { ...request.route, executorId: request.executorId ?? 'fresh-reviewer', authentication: 'native-subscription', observation: 'Injected policy evaluator' },
        outcome: request.stage === 'architecture' && !improved ? 'changes-requested' : 'completed', summary: 'Functional evaluator',
        issues: request.stage === 'architecture' && !improved ? ['Planner needs concrete guidance'] : [],
        metering: { costUsd: 0, latencyMs: 1, evidenceDigest: hash(request) } },
        changes: request.stage === 'implementation' ? [{ path: 'product.txt', content: 'fixed\n' }] : [],
        ...(request.stage === 'architecture' ? { plan: { summary: 'Fix admitted source', files: ['product.txt'], tests: ['build', 'public'] } } : {}) };
    } });
  expect(activation.binding).not.toBeNull();
  const input = { ...activation, binding: activation.binding!, publicKey: signer.publicKey(), rootId: 'frozen-root', inputDigest: deliveryPolicyInputDigest(target) };
  expect(verifyDeliveryPolicyActivation(input)).toEqual(activation.policy);
  expect(() => verifyDeliveryPolicyActivation({ ...input, policy: { ...input.policy, planner: 'Different untested bytes' } })).toThrow();
  expect(() => verifyDeliveryPolicyActivation({ ...input, inputDigest: '0'.repeat(64) })).toThrow();
  expect(() => verifyDeliveryPolicyActivation({ ...input, publicKey: makeSigner().publicKey() })).toThrow();
  expect(() => verifyDeliveryPolicyActivation({ ...input, evidence: { ...activation.evidence, sourceDigest: 'tampered' } })).toThrow();
  expect(f.harness.snapshot().digest).toBe(before);
  expect(readdirSync(f.harness.directory)).not.toContain('learning');
  const installed = await activateDeliveryPolicy(f.root, target, activation, null);
  expect(loadDeliveryPolicy(f.root, target).activation).toBe(installed.digest);
  expect(loadDeliveryPolicy(f.root, { ...target, task: { ...target.task, id: 'unrelated' } }).nonapplicability).toBe('different-evaluated-task');
  await expect(activateDeliveryPolicy(f.root, target, activation, null)).rejects.toThrow('DELIVERY_POLICY_CAS_MISMATCH');
  expect((await activateDeliveryPolicy(f.root, target, activation, installed.digest)).previous).toBe(installed.digest);
  const ordinary = await runDeliveryOutcome(candidate.harness, f.task.id, f.task.owner, { execute: async (request, _files, context) => {
    if (request.stage === 'architecture') expect(JSON.stringify(context)).toContain('Improved concrete policy guidance');
    return { response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, ...request.route, executorId: request.executorId ?? 'fresh-reviewer' }, outcome: 'completed', summary: 'Active policy ordinary run', issues: [] },
      changes: request.stage === 'implementation' ? [{ path: 'product.txt', content: 'fixed\n' }] : [],
      ...(request.stage === 'architecture' ? { plan: { summary: 'Ordinary plan', files: ['product.txt'], tests: ['build', 'public'] } } : {}) };
  } });
  expect(ordinary.success).toBe(true); expect(ordinary.policyEvaluation).toBe(false); expect(ordinary.policyActivation).toBe(installed.digest);
  expect(ordinary.policyDigest).toBe(hash(activation.policy));
  writeFileSync(join(f.root, '.metaharness/delivery/policy/active.json'), JSON.stringify({ ...activation, policy: { ...activation.policy, planner: 'Tampered artifact guidance' } }));
  expect(() => loadDeliveryPolicy(f.root, { ...target, task: { ...target.task, id: 'unrelated' } })).toThrow();
}, 60000);
