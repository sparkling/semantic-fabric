// SPDX-License-Identifier: MIT
import { readFileSync } from 'node:fs';
import { hash, ReceiptLog } from '@metaharness/harness';
import { runFlywheelGenerations, type Evaluator, type Proposer, type Signer } from '@metaharness/flywheel';
import { createDeliveryCandidate } from './delivery-candidate.js';
import { parseDeliveryHandoff, parseDeliveryTask, type DeliveryTask, type NativeHandoff } from './delivery-contracts.js';
import type { DeliveryHarness, DeliveryRun } from './delivery-runtime.js';
import type { DeliveryExecutor } from './delivery-executor.js';
import { runDeliveryOutcome } from './delivery-runner.js';
import { captureDeliveryPolicy, DELIVERY_ROOT_POLICY, deliveryPolicyInputDigest, deliveryPromotionRule } from './delivery-policy.js';

export interface DeliveryEvaluationCase { task: DeliveryTask; handoff: NativeHandoff }

/** Explicit evolution only. Every score comes from the ordinary candidate runner, never supplied scores. */
export async function evaluateDeliveryPolicies(input: {
  canonical: DeliveryHarness; target: DeliveryRun; parentDirectory: string; rootId: string;
  holdout: DeliveryEvaluationCase[]; anchor: DeliveryEvaluationCase[];
  proposer: Proposer; signer: Signer; maxGenerations: number;
  execute?: DeliveryExecutor; signal?: AbortSignal;
}) {
  const capture = (rows: DeliveryEvaluationCase[]) => rows.map(row => {
    const task = parseDeliveryTask(row.task), handoff = parseDeliveryHandoff(row.handoff);
    if (task.host === 'openrouter' || handoff.host === 'openrouter' || task.host !== handoff.host) throw new Error('DELIVERY_NATIVE_EVOLUTION_REQUIRED');
    return { task, handoff };
  });
  const holdout = capture(input.holdout), anchor = capture(input.anchor);
  if (!holdout.length || !anchor.length || !holdout.some(row => hash(row.task) === hash(input.target.task))) throw new Error('DELIVERY_EVOLUTION_SUITE_INVALID');
  if (new Set([...holdout, ...anchor].map(row => row.task.id)).size !== holdout.length + anchor.length) throw new Error('DELIVERY_EVOLUTION_SUITE_OVERLAP');
  const source = input.canonical.snapshot().digest, inputDigest = deliveryPolicyInputDigest(input.target);
  const suites = { holdout, anchor }, evaluations: unknown[] = [];
  const baseline = new Map<string, boolean>();
  const evaluator: Evaluator = async (value, suite) => {
      const policy = captureDeliveryPolicy(value), receipts = [];
      for (const { task, handoff } of suite.items as DeliveryEvaluationCase[]) {
        if (input.signal?.aborted) throw input.signal.reason ?? new Error('DELIVERY_EVOLUTION_CANCELLED');
        if (input.canonical.snapshot().digest !== source || deliveryPolicyInputDigest(input.target) !== inputDigest) throw new Error('DELIVERY_EVOLUTION_INPUT_DRIFT');
        const candidate = createDeliveryCandidate(input.canonical, { parentDirectory: input.parentDirectory, scope: task.scope });
        await candidate.harness.begin(task); await candidate.harness.bind(task.id, task.owner, handoff);
        const outcome = await runDeliveryOutcome(candidate.harness, task.id, task.owner,
          { execute: input.execute, signal: input.signal, evaluationPolicy: policy });
        const saved = JSON.parse(readFileSync(outcome.receiptPath, 'utf8'));
        const { digest, ...body } = saved;
        if (digest !== hash(body) || body.policyDigest !== hash(policy) || !body.policyEvaluation
          || body.taskDigest !== hash(task) || !ReceiptLog.fromJSON({ receipts: body.kernel.receipts }).verify().ok) throw new Error('DELIVERY_EVOLUTION_RECEIPT_INVALID');
        // Infrastructure interruption is not a policy quality label.
        if (body.failure && !['DELIVERY_REPAIR_LIMIT_REACHED', 'DELIVERY_VERIFICATION_FAILED', 'DELIVERY_PLAN_REJECTED'].includes(body.failure)) throw new Error(`DELIVERY_EVOLUTION_STOP:${body.failure}`);
        receipts.push(saved);
        if (hash(policy) === hash(DELIVERY_ROOT_POLICY)) baseline.set(hash(task), saved.success);
      }
      const primary = receipts.filter(receipt => receipt.success).length / receipts.length;
      const score = { primary, noopRate: receipts.filter(receipt => receipt.sourceBefore === receipt.sourceAfter).length / receipts.length,
        costPerWin: 0, regressed: receipts.some(receipt => baseline.get(receipt.taskDigest) === true && !receipt.success) };
      evaluations.push({ policy, policyDigest: hash(policy), suite: suite.id, score, receipts });
      return score;
  };
  const result = await runFlywheelGenerations({ rootId: input.rootId, rootPolicy: DELIVERY_ROOT_POLICY,
    proposer: async (base, target) => {
      const proposed = await input.proposer(base, target);
      captureDeliveryPolicy({ ...base.policy, [target]: typeof proposed === 'string' ? proposed : proposed.value });
      return proposed;
    }, signer: input.signer, maxGenerations: input.maxGenerations, promotionRule: deliveryPromotionRule,
    holdout: { id: 'holdout', items: holdout }, anchor: { id: 'anchor', items: anchor }, evaluator,
    dataSource: input.execute ? 'INJECTED_FUNCTIONAL' : 'NATIVE_ORDINARY_OUTCOMES',
  });
  if (input.canonical.snapshot().digest !== source || deliveryPolicyInputDigest(input.target) !== inputDigest) throw new Error('DELIVERY_EVOLUTION_INPUT_DRIFT');
  const head = result.replayBundle.chain[0];
  const replayHoldout = head?.verdict === 'PROMOTED' ? await evaluator(result.finalPolicy, { id: 'replay-holdout', items: holdout }) : null;
  const replayAnchor = head?.verdict === 'PROMOTED' ? await evaluator(result.finalPolicy, { id: 'replay-anchor', items: anchor }) : null;
  if (input.canonical.snapshot().digest !== source || deliveryPolicyInputDigest(input.target) !== inputDigest) throw new Error('DELIVERY_EVOLUTION_INPUT_DRIFT');
  const evidence = { suites, evaluations, inputDigest, targetTaskDigest: hash(input.target.task), sourceDigest: source };
  const binding = head?.verdict === 'PROMOTED' && holdout.length >= 5 && anchor.length >= 5
    && !replayHoldout?.regressed && !replayAnchor?.regressed && hash(replayHoldout) === hash(head.candidateScore)
    && replayAnchor?.primary === head.anchorScore ? input.signer.sign({ kind: 'fabric-evaluated-policy-v1', rootId: input.rootId, headId: head.id,
    policyDigest: hash(result.finalPolicy), bundleDigest: hash(result.replayBundle), inputDigest, evidenceDigest: hash(evidence) }) : null;
  // Returned evidence is not installed or published. Operator trust pins remain separately controlled.
  return { policy: result.finalPolicy, bundle: result.replayBundle, binding, evidence };
}
