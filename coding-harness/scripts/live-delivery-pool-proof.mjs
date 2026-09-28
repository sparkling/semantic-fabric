// SPDX-License-Identifier: MIT
// Explicit local harness proof; paid read-only reviews, never application work.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { hash } from '@metaharness/harness';
import { DeliveryHarness } from '../dist/delivery-runtime.js';
import { createDeliveryCandidate } from '../dist/delivery-candidate.js';
import { deliveryPoolIdentity, runDeliveryPool } from '../dist/delivery-pool.js';
import { createDeliveryApi } from '../dist/delivery-api.js';
import { verifyNativeStage } from '../dist/delivery-stage.js';
import { atomicJson } from '../dist/delivery-workspace.js';
import { buildCheckEnvironment } from '../dist/delivery-process.js';

const directory = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const sourceOnly = process.argv[2] === '--source-review-only';
if (process.argv.length > (sourceOnly ? 3 : 2)) throw new Error('usage: live-delivery-pool-proof.mjs [--source-review-only]');
const harness = new DeliveryHarness(resolve(directory, '..'));
if (harness.inspect().active !== null || harness.inspect().operation !== null) throw new Error('ACTIVE_WRITER');
const source = harness.snapshot();
const proofDirectory = join(harness.directory, `pool-proof-${source.digest}`);
mkdirSync(proofDirectory, { mode: 0o700 });
const checks = [];
for (const [id, argv] of [['build', ['run', 'build']], ['focused', ['test', '--', '--run',
  '__tests__/delivery-candidate.test.ts', '__tests__/delivery-api.test.ts',
  '__tests__/delivery-runtime.test.ts', '__tests__/delivery-workflow.test.ts']]]) {
  const began = Date.now();
  const stdout = execFileSync('npm', argv, { cwd: directory, env: buildCheckEnvironment(), maxBuffer: 10000000 });
  writeFileSync(join(proofDirectory, `${id}.stdout`), stdout, { flag: 'wx', mode: 0o600 });
  if (harness.snapshot().digest !== source.digest) throw new Error('CHECK_SOURCE_CHANGED');
  checks.push({ id, argv: ['npm', ...argv], passed: true, exitCode: 0, durationMs: Date.now() - began,
    sourceBefore: source.digest, sourceAfter: source.digest,
    stdoutDigest: createHash('sha256').update(stdout).digest('hex') });
}
const inputPath = 'coding-harness/src/delivery-api.ts';
const candidate = createDeliveryCandidate(harness, { parentDirectory: mkdtempSync(join(tmpdir(), 'fabric-pool-proof-')), scope: [inputPath],
  acceptedParent: 'known-http-classification-20260928', acceptedInputs: [inputPath] });
const bytes = readFileSync(join(candidate.harness.root, inputPath));
const readDigest = createHash('sha256').update(bytes).digest('hex');
if (candidate.acceptedSource.inputs[inputPath].split(':')[1] !== readDigest) throw new Error('ACCEPTED_INPUT_NOT_CONSUMED');
const parentProof = { ...candidate.acceptedSource, childRoot: candidate.harness.root,
  childSourceDigest: candidate.harness.snapshot().digest, readDigest, readBytes: bytes.length };
atomicJson(join(proofDirectory, 'accepted-parent.json'), parentProof);

const partitions = [
  { id: 'fabric-pool-source-review-20260928',
    question: 'Independent adversarial review of non-Git candidate isolation, exact accepted-parent file input consumption, source drift refusal and ordinary lifecycle checks. Scopes name exact files, not directories; pool resource exclusions may use ancestor paths. Existing canonical single-writer authority must stay intact. Candidate directories are cooperative and writable; source digests, not OS sandboxing, enforce mutation scope. Review concrete regressions in this pool/candidate change; delivery-api.ts is unchanged accepted-parent input.',
    paths: ['delivery-candidate.ts', 'delivery-context.ts', 'delivery-runtime.ts', 'delivery-proposal.ts', 'delivery-workspace.ts', 'immutable-private-runtime.ts'] },
  { id: 'fabric-pool-custody-review-20260928',
    question: 'Independent adversarial review of actual upstream pool integration, path/resource exclusions, pre-abort and noncooperative cancellation drain, failed evidence retention, and host-only package boundary. Exact-file mutation scopes are inherited from ordinary lifecycle; pool conflict checks also allow ancestor paths. This optional development host adapter requires a full development install; it is not reachable from the frozen production controller. No application restart. Review regressions in this pool/candidate change; unchanged delivery-api.ts is accepted-parent input, with a strict proven-nonexecution HTTP policy.',
    paths: ['delivery-pool.ts', 'delivery-candidate.ts', 'delivery-context.ts', 'delivery-process.ts'] },
];
const events = [];
const selected = sourceOnly ? partitions.slice(0, 1) : partitions;
const charges = new Map();
const controller = new AbortController();
process.once('SIGINT', () => controller.abort()); process.once('SIGTERM', () => controller.abort());
const result = await runDeliveryPool(harness, selected.map(partition => ({
  id: partition.id, mutationPaths: [], resources: [], run: async signal => {
    const scope = [inputPath, ...partition.paths.map(path => `coding-harness/src/${path}`),
      'coding-harness/__tests__/delivery-candidate.test.ts', 'coding-harness/package.json'];
    const taskDigest = hash({ id: partition.id, requirement: partition.question, scope });
    const body = { schemaVersion: 1, taskId: partition.id, thread: 'fabric-harness-implementation',
      baseCommit: harness.context.head(), stage: 'review', attempt: 1, sourceDigest: source.digest,
      evidenceDigest: hash(checks), route: { host: 'openrouter', model: 'deepseek/deepseek-v4.1-flash', effort: 'high' },
      prerequisiteDigests: checks.map(hash), requirement: partition.question, scope, feedback: [], repair: false };
    const request = { ...body, id: hash(body) };
    const files = scope.map(path => ({ path, content: readFileSync(join(candidate.harness.root, path), 'utf8') }));
    const invoke = createDeliveryApi({ directory: join(harness.directory, 'api'),
      observation: event => events.push({ ...event, taskId: partition.id, at: Date.now() }) });
    let proposal;
    try { proposal = await invoke(request, files, [...checks, { acceptedParent: parentProof }], taskDigest, signal); }
    catch (error) {
      if (error.evidence) {
        charges.set(partition.id, error.evidence);
        atomicJson(join(proofDirectory, `${partition.id}.failure.json`), error.evidence);
      }
      throw error;
    }
    charges.set(partition.id, proposal.evidence);
    atomicJson(join(proofDirectory, `${partition.id}.json`), proposal);
    if (harness.snapshot().digest !== source.digest) throw new Error('REVIEW_SOURCE_CHANGED');
    const verified = await verifyNativeStage(request, proposal.response, []);
    atomicJson(join(proofDirectory, `${partition.id}.kernel.json`), verified);
    return { accepted: verified.accepted, receiptValid: verified.kernel.receiptsValid,
      reviewer: proposal.response.native.executorId, actualUsd: proposal.evidence.actualUsd,
      evidencePath: proposal.evidencePath, summary: proposal.response.summary, issues: proposal.response.issues };
  },
})), { maxConcurrency: 2, signal: controller.signal });
const intervals = selected.map(({ id }) => ({ id,
  start: events.find(event => event.taskId === id && event.phase === 'api-start')?.at,
  end: events.find(event => event.taskId === id && event.phase === 'api-settled')?.at }));
const overlapMs = sourceOnly ? 0 : Math.max(0, Math.min(...intervals.map(row => row.end ?? 0)) - Math.max(...intervals.map(row => row.start ?? Infinity)));
const passed = (sourceOnly || overlapMs > 0) && result.results.every(row => row.status === 'fulfilled' && row.value?.accepted && row.value?.receiptValid);
const proof = { mode: sourceOnly ? 'source-review-only' : 'parallel-review', sourceDigest: source.digest, baseCommit: harness.context.head(), checks, parentProof,
  executor: deliveryPoolIdentity(), result, events, intervals, overlapMs, passed,
  knownActualUsd: [...charges.values()].reduce((sum, evidence) => sum + (evidence.actualUsd ?? 0), 0),
  unknownChargeTasks: [...charges.entries()].filter(([, evidence]) => evidence.status === 'completion-unknown').map(([id]) => id) };
atomicJson(join(proofDirectory, 'result.json'), proof);
console.log(JSON.stringify({ proofPath: join(proofDirectory, 'result.json'), passed, overlapMs,
  knownActualUsd: proof.knownActualUsd, unknownChargeTasks: proof.unknownChargeTasks,
  outcomes: result.results.map(row => ({ id: row.id, status: row.status, accepted: row.value?.accepted, issues: row.value?.issues })) }));
process.exitCode = passed ? 0 : 1;
