// SPDX-License-Identifier: MIT
import { AgentPool, AlgorithmRouter, HarnessKernel, PolicyGate, VerifierRegistry, hash } from '@metaharness/harness';
import type { NativeStageRequest, NativeStageResponse, NativeStageResult } from './delivery-workflow-contracts.js';

/** A ready stage only. The durable outer controller supplies prerequisites and repair feedback.
 * The worker consumes a response from the existing native host; it launches no second host.
 * Kernel receipts bind the full request/response, not just an unbound role name. */
export async function verifyNativeStage(request: NativeStageRequest, response: NativeStageResponse,
  reasons: string[]): Promise<NativeStageResult> {
  const accepted = response.outcome === 'completed' && reasons.length === 0;
  const pool = new AgentPool();
  pool.register({ id: response.native.executorId, model: response.native.model, handles: [request.stage],
    run: async () => ({ output: { request, response }, quality: accepted ? 1 : 0,
      confidence: 1, risk: 0, costUsd: 0, latencyMs: 0 }) });
  const verifiers = new VerifierRegistry();
  verifiers.register({ id: 'source-bound-native-stage', kind: request.stage, check: async value => ({
    pass: accepted && hash(value) === hash({ request, response }), score: accepted ? 1 : 0,
    reasons: accepted ? [] : [...reasons, response.summary, ...response.issues],
  }) });
  const kernel = await new HarnessKernel({ pool, verifiers,
    router: new AlgorithmRouter({ 'native-delivery': { intent: 'native-delivery', steps: [{ kind: request.stage }] } }),
    policy: new PolicyGate([{ id: 'only-declared-stage', effect: 'allow',
      match: action => action.tool === request.stage, risk: 0 }], 0),
    // No subscription quota or spend gate. Disable upstream's internal replay of
    // the SAME submitted response; every repair instead gets fresh failure feedback.
    // This is not a limit on native model requests (this worker invokes no model).
    budget: { costUsd: Infinity, retries: 0, risk: 0, confidence: 1 }, breakerThreshold: 1,
    actionFor: () => ({ tool: request.stage, args: { requestDigest: hash(request) } }),
  }).run({ text: request.requirement, intent: 'native-delivery', context: { request } }, request.id);
  return { request, response, kernel, accepted: accepted && kernel.success && kernel.receiptsValid, reasons };
}
