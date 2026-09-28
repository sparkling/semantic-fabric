// SPDX-License-Identifier: MIT
import { resolve, join } from 'node:path';
import { readFileSync, writeFileSync } from 'node:fs';
import { hash } from '@metaharness/harness';
import { fileURLToPath } from 'node:url';
import { DeliveryHarness } from './delivery-runtime.js';
import { readJson, sourceSnapshot } from './delivery-workspace.js';
import { resolveWorkspacePath } from './workspace.js';
import { createDeliveryApi } from './delivery-api.js';
import type { NativeHandoff } from './delivery-contracts.js';

/** API proposals never edit source or imply acceptance. Native handoffs and
 * optional Ruflo mirroring remain owned by the active integration host. */
export async function deliveryCli(args: string[], signal?: AbortSignal): Promise<boolean> {
  const [root, command, ...rest] = args;
  const counts: Record<string, number> = { begin: 1, status: 1, inspect: 0, bind: 3, check: 3, verify: 2, finish: 3,
    pause: 3, resume: 2, supersede: 4, reconcile: 4, next: 2, advance: 2, submit: 3, propose: 2 };
  if (!root || !(command in counts) || rest.length !== counts[command]) {
    throw new Error('usage: delivery <repository-root> begin <task.json> | status <id> | inspect | bind <id> <owner> <native.json> | next|advance|propose <id> <owner> | submit <id> <owner> <response.json> | check <id> <owner> <check-id> | verify <id> <owner> | finish <id> <owner> <commit> | pause <id> <owner> <reason> | resume <id> <owner> | supersede <id> <owner> <successor-id> <reason> | reconcile <id> <owner> <nonce-or-none> <reason>');
  }
  const harness = new DeliveryHarness(resolve(root));
  if (command === 'inspect') { console.log(JSON.stringify(harness.inspect(), null, 2)); return true; }
  const [id, owner, extra] = rest;
  if (command === 'propose') {
    const action = await harness.next(id, owner);
    if (action.kind !== 'native' || action.request.route.host !== 'openrouter') throw new Error('DELIVERY_PENDING_API_REQUEST_REQUIRED');
    const run = harness.read(id);
    const files = action.request.scope.map(path => {
      const absolute = resolveWorkspacePath(harness.root, path, { allowMissingLeaf: true, requireRegularFile: true });
      try { return { path, content: readFileSync(absolute, 'utf8') }; }
      catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { path, content: null }; throw error; }
    });
    const checks = run.checks.map(({ id, attempt, passed, exitCode, sourceBefore, sourceAfter, stdoutDigest, stderrDigest }) =>
      ({ id, attempt, passed, exitCode, sourceBefore, sourceAfter, stdoutDigest, stderrDigest }));
    const proposal = await createDeliveryApi({ directory: join(harness.directory, 'api') })(action.request, files, checks, hash(run.task), signal);
    if (sourceSnapshot(harness.root).digest !== action.request.sourceDigest) throw new Error('DELIVERY_API_SOURCE_CHANGED');
    const path = join(harness.directory, `proposal-${proposal.evidence.requestId}.json`);
    writeFileSync(path, JSON.stringify(proposal), { flag: 'wx', mode: 0o600 });
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
    : command === 'submit' ? await harness.submit(id, owner, readJson(resolve(extra)))
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
  if (command === 'submit') return run.workflow?.results.at(-1)?.accepted === true;
  return true;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const controller = new AbortController();
  process.once('SIGINT', () => controller.abort());
  process.once('SIGTERM', () => controller.abort());
  deliveryCli(process.argv.slice(2), controller.signal).then(ok => { process.exitCode = ok ? 0 : 1; })
    .catch(error => { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; });
}
