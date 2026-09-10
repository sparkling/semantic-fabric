// SPDX-License-Identifier: MIT
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DeliveryHarness } from './delivery-runtime.js';
import { readJson } from './delivery-workspace.js';
import type { NativeHandoff } from './delivery-contracts.js';

/** Does not launch/resume a model, edit source, commit, push, or access Ruflo stores.
 * The active native host performs handoffs and mirrors outcomes through Ruflo MCP. */
export async function deliveryCli(args: string[], signal?: AbortSignal): Promise<boolean> {
  const [root, command, ...rest] = args;
  const counts: Record<string, number> = { begin: 1, status: 1, inspect: 0, bind: 3, check: 3, verify: 2, finish: 3, pause: 3, resume: 2, reconcile: 4 };
  if (!root || !(command in counts) || rest.length !== counts[command]) {
    throw new Error('usage: delivery <repository-root> begin <task.json> | status <id> | inspect | bind <id> <owner> <native.json> | check <id> <owner> <check-id> | verify <id> <owner> | finish <id> <owner> <commit> | pause <id> <owner> <reason> | resume <id> <owner> | reconcile <id> <owner> <nonce-or-none> <reason>');
  }
  const harness = new DeliveryHarness(resolve(root));
  if (command === 'inspect') { console.log(JSON.stringify(harness.inspect(), null, 2)); return true; }
  const [id, owner, extra] = rest;
  const run = command === 'begin' ? await harness.begin(readJson(resolve(id)))
    : command === 'status' ? harness.read(id)
    : command === 'bind' ? await harness.bind(id, owner, readJson(resolve(extra)) as NativeHandoff)
    : command === 'check' ? await harness.check(id, owner, extra, signal)
    : command === 'verify' ? await harness.verify(id, owner)
    : command === 'finish' ? await harness.finish(id, owner, extra)
    : command === 'pause' ? await harness.pause(id, owner, extra)
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
