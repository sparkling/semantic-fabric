// SPDX-License-Identifier: MIT
import { randomUUID } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { AgentPool, AlgorithmRouter, HarnessKernel, PolicyGate, VerifierRegistry, hash } from '@metaharness/harness';
import { DeliveryApiFailure, parseDeliveryChanges, parseDeliveryPlan, type DeliveryPlan } from './delivery-api.js';
import { createDeliveryExecutor, type DeliveryExecution, type DeliveryExecutor } from './delivery-executor.js';
import { bindAppliedProposal } from './delivery-proposal.js';
import { deliveryReadPaths } from './delivery-candidate.js';
import { resolveWorkspacePath } from './workspace.js';
import { atomicJson, withOperationLock } from './delivery-workspace.js';
import type { DeliveryHarness } from './delivery-runtime.js';
import type { DeliveryRoute, NativeHandoff } from './delivery-contracts.js';
import type { NativeStageRequest } from './delivery-workflow-contracts.js';
import { verifyNativeStage } from './delivery-stage.js';
import { openDeliveryLearning } from './delivery-learning.js';
import { parseStageResponse } from './delivery-workflow-contracts.js';
import { route } from './delivery-contracts.js';
import { captureDeliveryPolicy, DELIVERY_ROOT_POLICY, loadDeliveryPolicy } from './delivery-policy.js';
import type { Policy } from '@metaharness/flywheel';

const repairRoute = { host: 'claude-code' as const, model: 'cc/claude-opus-5-5[1m]', effort: 'high' as const };
const fresh = (selected: DeliveryRoute): NativeHandoff => ({ ...selected,
  executorId: `runner-${randomUUID()}`, authentication: 'native-subscription', observation: 'Runner selected native subscription executor; actual invocation recorded in response' });

/** Candidate execution never integrates main. Existing lifecycle remains acceptance authority. */
export async function runDeliveryOutcome(harness: DeliveryHarness, id: string, owner: string,
  options: { execute?: DeliveryExecutor; signal?: AbortSignal; maximumRepairs?: number; evaluationPolicy?: Policy } = {}) {
  if (harness.context.kind !== 'candidate') throw new Error('DELIVERY_RUN_REQUIRES_CANDIDATE');
  const maximumRepairs = options.maximumRepairs ?? 2;
  if (!Number.isSafeInteger(maximumRepairs) || maximumRepairs < 0) throw new Error('DELIVERY_REPAIR_LIMIT_INVALID');
  const run = harness.read(id), runId = `outcome-${randomUUID()}`, sourceBefore = harness.snapshot();
  if (run.task.owner !== owner) throw new Error('DELIVERY_WRITER_MISMATCH');
  const directory = join(harness.directory, 'runner'); mkdirSync(directory, { recursive: true, mode: 0o700 });
  const execute = options.execute ?? createDeliveryExecutor(harness, hash(run.task));
  return withOperationLock(directory, async () => {
    if (options.evaluationPolicy && run.task.host === 'openrouter') throw new Error('DELIVERY_HYBRID_EVOLUTION_REFUSED');
    const policy = options.evaluationPolicy ? { policy: captureDeliveryPolicy(options.evaluationPolicy), digest: hash(options.evaluationPolicy), activation: null }
      : run.task.host === 'openrouter' ? { policy: DELIVERY_ROOT_POLICY, digest: hash(DELIVERY_ROOT_POLICY), activation: null }
      : loadDeliveryPolicy(harness.context.canonicalRoot ?? harness.root, run);
    const learning = await openDeliveryLearning(join(options.evaluationPolicy ? harness.directory : dirname(harness.context.apiDirectory ?? join(harness.directory, 'api')),
      'learning', policy.digest), runId, run.task, run.route);
    let plan: DeliveryPlan | undefined, failure: string | null = null, repairs = 0;
    const evidence: unknown[] = [];
    let knownActualUsd = 0;
    const learningFailures: string[] = [];
    const invoke = async (request: NativeStageRequest): Promise<DeliveryExecution> => {
      if (options.signal?.aborted) throw options.signal.reason ?? new Error('DELIVERY_RUN_CANCELLED');
      const before = harness.snapshot();
      if (before.digest !== request.sourceDigest) throw new Error('DELIVERY_EXECUTION_SOURCE_CHANGED');
      const files = deliveryReadPaths(harness, request.scope, run.task.readPaths).map(path => {
        const absolute = resolveWorkspacePath(harness.root, path, { allowMissingLeaf: true, requireRegularFile: true });
        try { return { path, content: readFileSync(absolute, 'utf8') }; }
        catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { path, content: null }; throw error; }
      });
      const checks = harness.read(id).checks.map(({ id, passed, exitCode, sourceBefore, sourceAfter, stdoutDigest, stderrDigest }) =>
        ({ id, passed, exitCode, sourceBefore, sourceAfter, stdoutDigest, stderrDigest }));
      const guidance = policy.policy[request.stage === 'architecture' ? 'planner' : request.stage === 'review' ? 'reviewer' : 'implementation'];
      const context = request.stage === 'review' ? [...checks, { policyGuidance: guidance }] : [{ declaredChecks: run.task.checks, ...(plan ? { plan } : {}), policyGuidance: guidance,
        ...(request.route.host !== 'openrouter' ? { modelRouting: learning.summary() } : {}) }, ...checks];
      let result: DeliveryExecution;
      try { result = await execute(request, files, context, options.signal); }
      catch (error) {
        if (error instanceof DeliveryApiFailure) knownActualUsd += error.evidence.actualUsd ?? 0;
        const failure = { request, error: error instanceof Error ? error.message : String(error),
          ...(error instanceof DeliveryApiFailure ? { evidence: error.evidence } : {}) };
        evidence.push(failure); atomicJson(join(directory, `${runId}-${evidence.length}.json`), failure); throw error;
      }
      knownActualUsd += result.response.metering?.costUsd ?? 0;
      if (harness.snapshot().digest !== before.digest) throw new Error('DELIVERY_EXECUTOR_MUTATED_SOURCE');
      evidence.push({ request, ...result });
      atomicJson(join(directory, `${runId}-${evidence.length}.json`), { request, ...result });
      const response = parseStageResponse(result.response);
      if (response.requestId !== request.id || response.sourceDigest !== request.sourceDigest
        || hash(route({ host: response.native.host, model: response.native.model, effort: response.native.effort })) !== hash(request.route)
        || (request.executorId && response.native.executorId !== request.executorId)) throw new Error('DELIVERY_EXECUTOR_IDENTITY_MISMATCH');
      if (response.outcome === 'unavailable' || response.outcome === 'cancelled') throw new Error(`${response.native.host}:${response.native.model}:${response.summary}`);
      if (request.stage === 'architecture' && response.outcome !== 'completed') throw new Error('DELIVERY_PLAN_REJECTED');
      return result;
    };
    const pool = new AgentPool();
    pool.register({ id: 'ordinary-delivery', model: 'configured-delivery-lifecycle', handles: ['delivery'], run: async () => {
      try {
        const body: Omit<NativeStageRequest, 'id'> = { schemaVersion: 1, taskId: id, thread: run.task.thread, baseCommit: run.baseCommit,
          stage: 'architecture', attempt: 1, sourceDigest: sourceBefore.digest, route: run.route,
          executorId: run.handoffs.at(-1)?.executorId, prerequisiteDigests: [], evidenceDigest: hash(run.task),
          requirement: run.task.requirement, scope: run.task.scope, feedback: [], repair: false };
        let request = { ...body, id: hash(body) };
        let planned: DeliveryExecution;
        try { planned = await invoke(request); }
        catch (error) {
          if (!(error instanceof DeliveryApiFailure) || !['completed-invalid-output', 'confirmed-credit-rejection'].includes(error.evidence.status)) throw error;
          const handoff = fresh(error.evidence.status === 'confirmed-credit-rejection' ? learning.selectCreditFallback() : repairRoute);
          const retry = { ...body, route: { host: handoff.host, model: handoff.model, effort: handoff.effort },
            executorId: handoff.executorId, failedApi: error.evidence, attempt: 2 };
          request = { ...retry, id: hash(retry) }; planned = await invoke(request);
        }
        plan = parseDeliveryPlan(planned.plan, run.task.scope);
        if (planned.changes.length || plan.tests.some(check => !run.task.checks.some(declared => declared.id === check))
          || !(await verifyNativeStage(request, planned.response, [])).accepted) throw new Error('DELIVERY_PLAN_REJECTED');
        for (;;) {
          const action = await harness.advance(id, owner, options.signal);
          if (action.kind === 'paused') throw new Error(action.reason);
          if (action.kind === 'ready-to-commit') {
            if (!(await harness.verify(id, owner)).verdict?.pass) throw new Error('DELIVERY_VERIFICATION_FAILED');
            break;
          }
          if (action.kind !== 'native') throw new Error('DELIVERY_RUN_ACTION_INVALID');
          let request = action.request;
          if (request.repair) {
            if (++repairs > maximumRepairs) throw new Error('DELIVERY_REPAIR_LIMIT_REACHED');
            if (request.route.host !== repairRoute.host || request.route.model !== repairRoute.model || request.route.effort !== repairRoute.effort) {
              await harness.repair(id, owner, fresh(repairRoute));
              const repaired = await harness.next(id, owner); if (repaired.kind !== 'native') throw new Error('DELIVERY_REPAIR_REQUEST_REQUIRED');
              request = repaired.request;
            }
          }
          let result: DeliveryExecution;
          try { result = await invoke(request); }
          catch (error) {
            if (!(error instanceof DeliveryApiFailure) || !['completed-invalid-output', 'confirmed-credit-rejection'].includes(error.evidence.status)) throw error;
            const handoff = fresh(error.evidence.status === 'confirmed-credit-rejection' ? learning.selectCreditFallback() : repairRoute);
            const apiDirectory = harness.context.apiDirectory ?? join(harness.directory, 'api');
            const { createHash } = await import('node:crypto');
            const digest = createHash('sha256').update(JSON.stringify({ taskDigest: hash(run.task), requestId: request.id })).digest('hex');
            await harness.fallback(id, owner, request.id, join(apiDirectory, `request-${digest}.json`), handoff);
            continue;
          }
          const before = harness.snapshot();
          if (request.stage !== 'implementation' && result.changes.length) throw new Error('DELIVERY_REVIEW_MUTATION_REFUSED');
          const changes = parseDeliveryChanges(result.changes, request.scope, request.stage !== 'implementation');
          if (result.response.outcome !== 'completed' && changes.length) throw new Error('DELIVERY_REJECTED_OUTPUT_MUTATION');
          if (changes.some(change => change.content.split('\n').length > 500)) throw new Error('DELIVERY_FILE_LINE_LIMIT');
          for (const change of changes) {
            if (!request.scope.includes(change.path)) throw new Error('DELIVERY_OUT_OF_SCOPE_CHANGE');
            const path = resolveWorkspacePath(harness.root, change.path, { allowMissingLeaf: true, requireRegularFile: true });
            mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, change.content);
          }
          const response = request.route.host === 'openrouter'
            ? bindAppliedProposal(harness, id, { response: result.response, changes: result.changes,
              evidence: result.evidence, evidencePath: result.evidencePath, sourceBefore: before })
            : { ...result.response, sourceDigest: harness.snapshot().digest };
          await harness.submit(id, owner, response);
        }
      } catch (error) {
        failure = error instanceof Error ? error.message : String(error);
        const current = harness.read(id);
        if (current.status === 'active' || current.status === 'awaiting-native') await harness.pause(id, owner, failure);
      }
      return { output: { accepted: failure === null, failure, sourceDigest: harness.snapshot().digest }, quality: failure === null ? 1 : 0,
        confidence: 1, risk: 0, costUsd: knownActualUsd, latencyMs: 0 };
    } });
    const kernel = await new HarnessKernel({ pool,
      router: new AlgorithmRouter({ delivery: { intent: 'delivery', steps: [{ kind: 'delivery' }] } }),
      policy: new PolicyGate([{ id: 'candidate-only', effect: 'allow', match: action => action.tool === 'delivery', risk: 0 }], 0),
      verifiers: new VerifierRegistry().register({ id: 'ordinary-outcome', kind: 'delivery', check: value => ({
        pass: (value as { accepted: boolean }).accepted, score: (value as { accepted: boolean }).accepted ? 1 : 0, reasons: failure ? [failure] : [] }) }),
      actionFor: () => ({ tool: 'delivery', args: { taskDigest: hash(run.task) } }),
      retrieveMemory: () => ({ modelRouting: learning.summary() }),
      updateMemory: async kernel => {
        const completed = harness.read(id);
        if (options.evaluationPolicy || !kernel.success || !kernel.receiptsValid || !completed.verdict?.pass || run.task.host === 'openrouter'
          || completed.handoffs.some(handoff => handoff.host === 'openrouter')) return;
        const author = completed.workflow?.results.filter(result => result.request.stage === 'implementation').at(-1);
        if (!author?.accepted || !author.response.metering || author.response.native.host === 'openrouter') return;
        try { await learning.record(author.request.route, author.request.repair ? 'repair' : 'implementation',
          author.response.metering.latencyMs, hash({ kernel, taskDigest: hash(run.task), sourceDigest: harness.snapshot().digest })); }
        catch (error) { learningFailures.push(error instanceof Error ? error.message : String(error)); }
      },
      budget: { costUsd: Infinity, retries: 0, risk: 0, confidence: 1 }, breakerThreshold: 1,
    }).run({ text: run.task.requirement, intent: 'delivery' }, runId);
    const receipt = { schemaVersion: 1, runId, taskDigest: hash(run.task), sourceBefore: sourceBefore.digest,
      sourceAfter: harness.snapshot().digest, success: kernel.success && kernel.receiptsValid && failure === null,
      status: failure === null ? 'candidate-awaiting-integration' : 'paused', failure, repairs, plan, kernel,
      evidenceDigests: evidence.map(hash), knownActualUsd, learningFailures, modelRouting: learning.summary(), policyDigest: policy.digest,
      policyActivation: policy.activation, policyNonapplicability: 'nonapplicability' in policy ? policy.nonapplicability : null,
      policyEvaluation: Boolean(options.evaluationPolicy), candidateRoot: harness.root };
    const receiptPath = join(directory, `${runId}.json`); atomicJson(receiptPath, { ...receipt, digest: hash(receipt) });
    return { ...receipt, receiptPath };
  });
}
