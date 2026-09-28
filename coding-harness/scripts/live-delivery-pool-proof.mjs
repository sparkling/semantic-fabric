// SPDX-License-Identifier: MIT
// Explicit local harness proof; paid fixture outcomes or read-only reviews, never application work.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, mkdirSync, mkdtempSync, readdirSync, writeFileSync } from 'node:fs';
import { availableParallelism, tmpdir } from 'node:os';
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
import { dispatchDeliveryReady } from '../dist/delivery-ready.js';
import { runDeliveryOutcome } from '../dist/delivery-runner.js';
import { createDeliveryExecutor } from '../dist/delivery-executor.js';

const directory = resolve(dirname(fileURLToPath(import.meta.url)), '..');
if (process.argv[2] === '--whole-outcome') {
  if (process.argv.length > 4 || (process.argv[3] && process.argv[3] !== '--preflight')) throw new Error('usage: live-delivery-pool-proof.mjs --whole-outcome [--preflight]');
  await wholeOutcomeProof(process.argv[3] === '--preflight');
} else {
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
}

async function wholeOutcomeProof(preflight) {
  const canonical = new DeliveryHarness(resolve(directory, '..'));
  if (canonical.inspect().active !== null || canonical.inspect().operation !== null) throw new Error('ACTIVE_WRITER');
  if (!preflight && !process.env.OPENROUTER_API_KEY) throw new Error('API_KEY_UNAVAILABLE');
  if (existsSync(join(canonical.directory, 'api/unknown-charge.json'))) throw new Error('CANONICAL_UNKNOWN_CHARGE_HOLD');
  const original = canonical.snapshot(), proofRoot = mkdtempSync(join(canonical.directory, 'whole-outcome-proof-'));
  const fixture = mkdtempSync(join(tmpdir(), 'fabric-live-outcomes-'));
  const root = join(fixture, 'canonical'), parentDirectory = join(fixture, 'candidates');
  mkdirSync(root); mkdirSync(parentDirectory); mkdirSync(join(root, 'coding-harness'));
  writeFileSync(join(root, '.gitignore'), '.metaharness/\n');
  const paths = ['product.txt', 'other.txt'];
  for (const path of paths) writeFileSync(join(root, path), 'before\n');
  writeFileSync(join(root, 'coding-harness/check.mjs'),
    "import assert from 'node:assert/strict';\nimport { readFileSync } from 'node:fs';\nconst path = process.argv[2];\nassert.ok(['product.txt', 'other.txt'].includes(path));\nassert.equal(readFileSync(new URL('../' + path, import.meta.url), 'utf8'), 'fixed\\n');\n");
  writeFileSync(join(root, 'coding-harness/build.mjs'),
    "import { execFileSync } from 'node:child_process';\nexecFileSync(process.execPath, ['--check', new URL('check.mjs', import.meta.url).pathname], { stdio: 'inherit' });\n");
  const git = (...args) => execFileSync('git', args, { cwd: root, env: buildCheckEnvironment(), encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  git('init', '-b', 'main'); git('config', 'user.name', 'Fabric live harness proof'); git('config', 'user.email', 'harness@example.invalid');
  git('add', '.'); git('-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'test: frozen live harness fixture');
  const harness = new DeliveryHarness(root), source = harness.snapshot();
  const route = { host: 'openrouter', model: 'deepseek/deepseek-v4.1-flash', effort: 'high' };
  const outcomes = paths.map((path, index) => ({ task: {
    schemaVersion: 1, id: `fabric-live-fixture-${index}`, owner: 'fabric-proof-integrator', thread: 'fabric-live-whole-outcome',
    requirement: `Harness fixture only: replace the entire contents of ${path} with exactly fixed followed by one newline. Preserve all other files. This is transport/lifecycle proof, not product work. Plan against declared build and acceptance check IDs.`,
    taskClass: 'implementation', host: route.host, scope: [path], checks: [
      { id: 'build', kind: 'build', argv: ['node', 'build.mjs'], cwd: 'coding-harness' },
      { id: 'acceptance', kind: 'acceptance', argv: ['node', 'check.mjs', path], cwd: 'coding-harness' },
    ] }, handoff: { ...route, executorId: `live-api-author-${index}`, authentication: 'openrouter-api', observation: 'Actual isolated OpenRouter executor; provider request evidence retained' }, resources: [`private-fixture-${index}`] }));
  const manifest = { schemaVersion: 1, parentDirectory, maxConcurrency: 2, mode: 'run', outcomes };
  atomicJson(join(proofRoot, 'manifest.json'), manifest);
  const baseline = [];
  for (const { task, handoff } of outcomes) {
    await harness.begin(task); await harness.bind(task.id, task.owner, { ...handoff, observation: 'Frozen baseline admission only; no model invocation' });
    for (const check of task.checks) {
      const result = (await harness.check(task.id, task.owner, check.id)).checks.at(-1);
      baseline.push({ taskId: task.id, checkId: check.id, exitCode: result.exitCode, passed: result.passed, sourceDigest: result.sourceAfter });
    }
    await harness.pause(task.id, task.owner, 'Frozen baseline checked; no model invocation or acceptance');
  }
  atomicJson(join(proofRoot, 'baseline.json'), baseline);
  if (baseline.some(row => row.checkId === 'build' ? !row.passed : row.passed || row.exitCode !== 1)) throw new Error('LIVE_FIXTURE_BASELINE_INVALID');
  const excluded = [];
  for (const conflict of ['path', 'resource']) {
    const blocked = structuredClone(manifest);
    if (conflict === 'path') blocked.outcomes[1].task.scope = blocked.outcomes[0].task.scope;
    else blocked.outcomes[1].resources = blocked.outcomes[0].resources;
    try { await dispatchDeliveryReady(harness, blocked, async () => { throw new Error('CONFLICT_DISPATCHED'); }); }
    catch (error) { if (!String(error).includes('RESOURCE_CONFLICT')) throw error; excluded.push(conflict); }
  }
  if (excluded.length !== 2 || readdirSync(parentDirectory).length) throw new Error('CONFLICT_PROOF_FAILED');
  const events = [], samples = [], completed = [];
  let previousCpu;
  const safeRead = path => { try { return readFileSync(path, 'utf8').trim(); } catch { return null; } };
  const sample = () => {
    const counters = safeRead('/proc/stat').split('\n')[0].trim().split(/\s+/).slice(1).map(Number);
    const cpu = { total: counters.slice(0, 8).reduce((sum, value) => sum + value, 0), idle: counters[3] + counters[4] };
    const idleFraction = previousCpu ? (cpu.idle - previousCpu.idle) / (cpu.total - previousCpu.total) : null; previousCpu = cpu;
    const row = { at: Date.now(), availableParallelism: availableParallelism(), cpuMax: safeRead('/sys/fs/cgroup/cpu.max'),
      cpusAllowed: safeRead('/proc/self/status').split('\n').find(line => line.startsWith('Cpus_allowed_list:')),
      idleFraction, memory: safeRead('/proc/meminfo').split('\n').filter(line => /^(MemAvailable|SwapFree):/.test(line)),
      pressure: Object.fromEntries(['cpu', 'memory', 'io'].map(name => [name, safeRead(`/proc/pressure/${name}`)])),
      heavyCheckConcurrency: 2 };
    samples.push(row); atomicJson(join(proofRoot, 'capacity.json'), samples);
    console.log(JSON.stringify({ phase: 'capacity', at: row.at, idleFraction, heavyCheckConcurrency: 2 }));
  };
  sample();
  const prerequisites = { sourceDigest: original.digest, fabricCommit: canonical.context.head(), fixtureRoot: root, fixtureCommit: git('rev-parse', 'HEAD'),
    baseline, excluded, model: route, apiKeyPresent: Boolean(process.env.OPENROUTER_API_KEY), preflight,
    dependentProof: { status: 'blocked', reason: 'No production candidate integration seam; historical accepted delivery-api.ts input is stale. No fabricated acceptance or extra model join.' } };
  atomicJson(join(proofRoot, 'prerequisites.json'), prerequisites);
  console.log(JSON.stringify({ phase: preflight ? 'preflight-complete' : 'live-start', pid: process.pid, proofRoot, fixtureRoot: root, model: route.model }));
  if (preflight) return;
  const controller = new AbortController(), timer = setInterval(sample, 30000);
  const stop = () => controller.abort(); process.once('SIGINT', stop); process.once('SIGTERM', stop);
  try {
    const result = await dispatchDeliveryReady(harness, manifest, async (candidate, mode, id, owner, signal) => {
      if (mode !== 'run') throw new Error('WHOLE_OUTCOME_MODE_REQUIRED');
      const task = candidate.read(id).task, actual = createDeliveryExecutor(candidate, hash(task));
      const outcome = await runDeliveryOutcome(candidate, id, owner, { signal, execute: async (request, files, checks, signal) => {
        const mark = phase => {
          const event = { at: Date.now(), taskId: id, phase, stage: request.stage, requestId: request.id, route: request.route };
          events.push(event); atomicJson(join(proofRoot, 'events.json'), events); console.log(JSON.stringify(event));
        };
        mark('start'); try { return await actual(request, files, checks, signal); } finally { mark('settled'); }
      } });
      completed.push({ taskId: id, success: outcome.success, failure: outcome.failure, receiptPath: outcome.receiptPath,
        sourceDigest: outcome.sourceAfter, actualUsd: outcome.knownActualUsd, receiptDigest: hash(JSON.parse(readFileSync(outcome.receiptPath, 'utf8'))) });
      atomicJson(join(proofRoot, 'outcomes.json'), completed);
      return outcome.success;
    }, controller.signal);
    const first = outcomes.map(({ task }) => ({ id: task.id,
      start: events.find(event => event.taskId === task.id && event.stage === 'architecture' && event.phase === 'start')?.at,
      end: events.find(event => event.taskId === task.id && event.stage === 'architecture' && event.phase === 'settled')?.at }));
    const overlapMs = Math.max(0, Math.min(...first.map(row => row.end ?? 0)) - Math.max(...first.map(row => row.start ?? Infinity)));
    const unchanged = harness.snapshot().digest === source.digest && canonical.snapshot().digest === original.digest;
    const passed = unchanged && overlapMs > 0 && completed.length === 2 && completed.every(row => row.success)
      && result.results.every(row => row.status === 'fulfilled');
    const proof = { ...prerequisites, result, completed, events, first, overlapMs, unchanged, passed,
      pool: deliveryPoolIdentity(), restart: { fabric: canonical.inspect(), scratch: harness.inspect() },
      knownActualUsd: completed.reduce((sum, row) => sum + row.actualUsd, 0) };
    atomicJson(join(proofRoot, 'result.json'), proof);
    console.log(JSON.stringify({ phase: 'live-complete', proofRoot, passed, overlapMs, knownActualUsd: proof.knownActualUsd,
      outcomes: completed.map(({ taskId, success, failure }) => ({ taskId, success, failure })), dependentProof: prerequisites.dependentProof }));
    process.exitCode = passed ? 0 : 1;
  } finally { clearInterval(timer); process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop); sample(); }
}
