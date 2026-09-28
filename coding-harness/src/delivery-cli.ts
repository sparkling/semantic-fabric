// SPDX-License-Identifier: MIT
import { resolve, join } from 'node:path';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { hash } from '@metaharness/harness';
import { fileURLToPath } from 'node:url';
import { DeliveryHarness } from './delivery-runtime.js';
import { readJson } from './delivery-workspace.js';
import { resolveWorkspacePath } from './workspace.js';
import { createDeliveryApi, renderDeliveryPrompt } from './delivery-api.js';
import type { NativeHandoff } from './delivery-contracts.js';
import { bindAppliedProposal } from './delivery-proposal.js';
import { dispatchDeliveryReady } from './delivery-ready.js';
import { reopenDeliveryCandidate } from './delivery-candidate.js';

/** API proposals never edit source or imply acceptance. Native handoffs and
 * optional Ruflo mirroring remain owned by the active integration host. */
export async function deliveryCli(args: string[], signal?: AbortSignal, candidate?: DeliveryHarness): Promise<boolean> {
  const [root, command, ...rest] = args;
  const counts: Record<string, number> = { begin: 1, status: 1, inspect: 0, bind: 3, check: 3, verify: 2, finish: 3,
    pause: 3, resume: 2, supersede: 4, reconcile: 4, next: 2, advance: 2, submit: 3, propose: 2, packet: 2, fallback: 5, repair: 3, ready: 1 };
  if (!root || !(command in counts) || rest.length !== counts[command]) {
    throw new Error('usage: delivery <repository-root> ready <manifest.json> | begin <task.json> | status <id> | inspect | bind|repair <id> <owner> <native.json> | next|advance|propose|packet <id> <owner> | fallback <id> <owner> <request-id> <api-evidence.json> <native.json> | submit <id> <owner> <response-or-proposal.json> | check <id> <owner> <check-id> | verify <id> <owner> | finish <id> <owner> <commit> | pause <id> <owner> <reason> | resume <id> <owner> | supersede <id> <owner> <successor-id> <reason> | reconcile <id> <owner> <nonce-or-none> <reason>');
  }
  if (candidate && (candidate.context.kind !== 'candidate' || candidate.root !== resolve(root))) throw new Error('DELIVERY_CANDIDATE_IDENTITY');
  const reopened = !candidate && existsSync(join(resolve(root), '.metaharness/delivery/candidate.json')) ? reopenDeliveryCandidate(resolve(root)) : undefined;
  const harness = candidate ?? reopened?.harness ?? new DeliveryHarness(resolve(root));
  if (command === 'ready') {
    const result = await dispatchDeliveryReady(harness, readJson(resolve(rest[0])),
      (selected, mode, id, owner, signal) => deliveryCli([selected.root, mode, id, owner], signal, selected), signal);
    console.log(JSON.stringify(result, null, 2));
    return result.results.every(row => row.status === 'fulfilled');
  }
  if (command === 'inspect') { console.log(JSON.stringify(harness.inspect(), null, 2)); return true; }
  const [id, owner, extra] = rest;
  if (command === 'submit') {
    const input = readJson(resolve(extra));
    const response = input !== null && typeof input === 'object' && 'changes' in input ? bindAppliedProposal(harness, id, input) : input;
    const run = await harness.submit(id, owner, response);
    console.log(JSON.stringify(run, null, 2)); return run.workflow?.results.at(-1)?.accepted === true;
  }
  if (command === 'fallback') {
    const run = await harness.fallback(id, owner, extra, rest[3], readJson(resolve(rest[4])) as NativeHandoff);
    console.log(JSON.stringify({ taskId: run.task.id, status: run.status, request: run.workflow?.requests.at(-1) }));
    return true;
  }
  if (command === 'propose' || command === 'packet') {
    const action = await harness.next(id, owner);
    if (action.kind !== 'native' || (command === 'propose' && action.request.route.host !== 'openrouter')) throw new Error('DELIVERY_PENDING_API_REQUEST_REQUIRED');
    const sourceBefore = harness.snapshot();
    if (sourceBefore.digest !== action.request.sourceDigest) throw new Error('DELIVERY_API_SOURCE_CHANGED');
    const run = harness.read(id);
    const files = action.request.scope.map(path => {
      const absolute = resolveWorkspacePath(harness.root, path, { allowMissingLeaf: true, requireRegularFile: true });
      try { return { path, content: readFileSync(absolute, 'utf8') }; }
      catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { path, content: null }; throw error; }
    });
    const checks = run.checks.map(({ id, attempt, passed, exitCode, sourceBefore, sourceAfter, stdoutDigest, stderrDigest }) =>
      ({ id, attempt, passed, exitCode, sourceBefore, sourceAfter, stdoutDigest, stderrDigest }));
    if (harness.snapshot().digest !== action.request.sourceDigest) throw new Error('DELIVERY_API_SOURCE_CHANGED');
    if (command === 'packet') {
      const path = join(harness.directory, `packet-${action.request.id}.json`);
      writeFileSync(path, JSON.stringify({ request: action.request, prompt: renderDeliveryPrompt(action.request, files, checks) }), { flag: 'wx', mode: 0o600 });
      console.log(JSON.stringify({ packetPath: path, status: 'awaiting-executor' })); return true;
    }
    const proposal = await createDeliveryApi({ directory: harness.context.apiDirectory ?? join(harness.directory, 'api') })(action.request, files, checks, hash(run.task), signal);
    if (harness.snapshot().digest !== action.request.sourceDigest) throw new Error('DELIVERY_API_SOURCE_CHANGED');
    const path = join(harness.directory, `proposal-${proposal.evidence.requestId}.json`);
    writeFileSync(path, JSON.stringify({ ...proposal, sourceBefore }), { flag: 'wx', mode: 0o600 });
    console.log(JSON.stringify({ proposalPath: path, actualUsd: proposal.evidence.actualUsd, status: 'proposal-awaiting-root-application' }));
    return true;
  }
  if (command === 'next' || command === 'advance') {
    const action = command === 'next' ? await harness.next(id, owner) : await harness.advance(id, owner, signal);
    console.log(JSON.stringify(action, null, 2)); return action.kind !== 'paused';
  }
  const run = command === 'begin' ? await harness.begin(readJson(resolve(id)))
    : command === 'status' ? harness.read(id)
    : command === 'bind' ? await harness.bind(id, owner, readJson(resolve(extra)) as NativeHandoff)
    : command === 'repair' ? await harness.repair(id, owner, readJson(resolve(extra)) as NativeHandoff)
    : command === 'check' ? await harness.check(id, owner, extra, signal)
    : command === 'verify' ? await harness.verify(id, owner)
    : command === 'finish' ? await harness.finish(id, owner, extra)
    : command === 'pause' ? await harness.pause(id, owner, extra)
    : command === 'supersede' ? await harness.supersede(id, owner, extra, rest[3])
    : command === 'reconcile' ? await harness.reconcile(id, owner, extra, rest[3])
    : await harness.resume(id, owner);
  console.log(JSON.stringify(run, null, 2));
  if (command === 'check') return run.checks.at(-1)?.passed === true;
  if (command === 'verify') return run.verdict?.pass === true;
  return true;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const controller = new AbortController();
  process.once('SIGINT', () => controller.abort());
  process.once('SIGTERM', () => controller.abort());
  deliveryCli(process.argv.slice(2), controller.signal).then(ok => { process.exitCode = ok ? 0 : 1; })
    .catch(error => { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; });
}
