// SPDX-License-Identifier: MIT
import { isAbsolute } from 'node:path';
import { asRecord, assertExactKeys, normalizeWorkspacePath } from './contracts.js';
import { identifier, nonempty, parseDeliveryHandoff, parseDeliveryTask } from './delivery-contracts.js';
import { assertAcceptedDeliverySource, createDeliveryCandidate } from './delivery-candidate.js';
import { runDeliveryPool, type DeliveryPoolProgress } from './delivery-pool.js';
import type { DeliveryHarness } from './delivery-runtime.js';
import { withSynchronousOperationLock } from './delivery-workspace.js';
import { requiredDeliveryInputs } from './delivery-lineage.js';

/** Explicit ready cohort only: accepted main outcomes, never candidate success, release dependencies. */
export async function dispatchDeliveryReady(canonical: DeliveryHarness, input: unknown,
  execute: (candidate: DeliveryHarness, command: 'packet' | 'propose' | 'run', id: string, owner: string, signal: AbortSignal) => Promise<boolean>,
  signal?: AbortSignal, observe?: (event: DeliveryPoolProgress) => void) {
  const manifest = asRecord(input, 'ready manifest');
  assertExactKeys(manifest, ['schemaVersion', 'parentDirectory', 'maxConcurrency', 'mode', 'outcomes'], 'ready manifest');
  if (manifest.schemaVersion !== 1 || !Number.isSafeInteger(manifest.maxConcurrency) || (manifest.maxConcurrency as number) < 1
    || !['packet', 'propose', 'run'].includes(String(manifest.mode)) || !Array.isArray(manifest.outcomes) || !manifest.outcomes.length) {
    throw new Error('DELIVERY_INVALID_READY_MANIFEST');
  }
  const parentDirectory = nonempty(manifest.parentDirectory, 'parentDirectory');
  if (!isAbsolute(parentDirectory)) throw new Error('DELIVERY_ABSOLUTE_CANDIDATE_PARENT_REQUIRED');
  const mode = manifest.mode as 'packet' | 'propose' | 'run';
  const outcomes = manifest.outcomes.map(value => {
    const row = asRecord(value, 'ready outcome');
    assertExactKeys({ acceptedParent: undefined, acceptedInputs: undefined, ...row },
      ['task', 'handoff', 'resources', 'acceptedParent', 'acceptedInputs'], 'ready outcome');
    const task = parseDeliveryTask(row.task), handoff = parseDeliveryHandoff(row.handoff);
    if (mode === 'propose' && task.host !== 'openrouter') throw new Error('DELIVERY_PENDING_API_REQUEST_REQUIRED');
    if (!Array.isArray(row.resources)) throw new Error('DELIVERY_INVALID_READY_RESOURCES');
    const resources = row.resources.map(value => nonempty(value, 'resource'));
    const acceptedParent = row.acceptedParent === undefined ? undefined : identifier(row.acceptedParent);
    if (row.acceptedInputs !== undefined && (!Array.isArray(row.acceptedInputs) || !row.acceptedInputs.length)) throw new Error('DELIVERY_ACCEPTED_INPUT_SCOPE');
    const acceptedInputs = (row.acceptedInputs as unknown[] | undefined)?.map(value => normalizeWorkspacePath(nonempty(value, 'accepted input'), 'accepted input'));
    const acceptedReadPaths = acceptedParent ? Object.keys(assertAcceptedDeliverySource(canonical, acceptedParent, acceptedInputs).inputs) : [];
    return { task, handoff, resources, acceptedParent, acceptedInputs, acceptedReadPaths };
  });
  return runDeliveryPool(canonical, outcomes.map(({ task, handoff, resources, acceptedParent, acceptedInputs, acceptedReadPaths }) => ({
    id: task.id, mutationPaths: task.scope, resources,
    readPaths: task.readPaths === undefined || task.checks.some(check => check.argv[0] === 'cargo') ? undefined : [...new Set([...task.readPaths, ...acceptedReadPaths,
      ...requiredDeliveryInputs(canonical.snapshot().files, task.checks)])],
    run: async (signal: AbortSignal, record: Parameters<Parameters<typeof runDeliveryPool>[1][number]['run']>[1]) => {
      const candidate = withSynchronousOperationLock(canonical.directory, () => {
        if (canonical.inspect().active !== null) throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
        return createDeliveryCandidate(canonical, { parentDirectory, scope: task.scope, acceptedParent, acceptedInputs, readPaths: task.readPaths, resources });
      });
      record(candidate);
      await candidate.harness.begin(task);
      await candidate.harness.bind(task.id, task.owner, handoff);
      if (signal.aborted) throw signal.reason ?? new Error('DELIVERY_POOL_CANCELLED');
      if (!await execute(candidate.harness, mode, task.id, task.owner, signal)) throw new Error('DELIVERY_READY_EXECUTION_FAILED');
      // Preserve source and evidence for the sole integrator, including failed callbacks.
      return { status: mode === 'packet' ? 'awaiting-executor' : mode === 'run' ? 'candidate-awaiting-integration' : 'proposal-awaiting-root-application',
        sourceDigest: candidate.harness.snapshot().digest, acceptedSource: candidate.acceptedSource };
    },
  })), { maxConcurrency: manifest.maxConcurrency as number, signal, observe });
}
