import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import * as fs from 'node:fs';
import { hash } from '@metaharness/harness';
import { createDeliveryCandidate } from '../src/delivery-candidate.js';
import { DeliveryHarness } from '../src/delivery-runtime.js';
import { runIntegrationReview } from '../src/delivery-integration-review.js';
import { saveReservations } from '../src/delivery-cohort-custody.js';
import { parseIntegrationInput, validateIntegrationEvidence } from '../src/delivery-integration.js';
import { git } from '../src/delivery-workspace.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';

vi.mock('node:fs', async importOriginal => {
  const actual = await importOriginal<typeof import('node:fs')>();
  return { ...actual, renameSync: vi.fn(actual.renameSync) };
});

const roots: string[] = [];
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllEnvs(); for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
async function fixture(readPaths: string[] | null = ['coding-harness/check.mjs'], combined = false, newFirst = false, resources: string[] = []) {
  const f = workflowFixture(roots);
  mkdirSync(join(f.root, 'tests')); writeFileSync(join(f.root, 'tests/proof.rs'), '// original evaluator\n');
  git(f.root, 'add', 'tests/proof.rs'); git(f.root, 'commit', '-qm', 'test input');
  writeFileSync(join(f.root, 'other.txt'), 'before\n'); git(f.root, 'add', 'other.txt'); git(f.root, 'commit', '-qm', 'second scope');
  const parentDirectory = mkdtempSync(join(tmpdir(), 'fabric-integration-')); roots.push(parentDirectory);
  const candidates = [];
  for (const [i, path] of (combined ? ['product.txt'] : [newFirst ? 'new.txt' : 'product.txt', 'other.txt']).entries()) {
    const task = { ...f.task, id: `candidate-${i}`, scope: combined ? ['product.txt', 'other.txt', ...(newFirst ? ['new.txt'] : [])] : [path], ...(readPaths ? { readPaths } : {}) };
    const candidate = createDeliveryCandidate(f.harness, { parentDirectory, scope: task.scope, readPaths: readPaths ?? undefined, resources });
    const h = candidate.harness;
    await h.begin(task); await h.bind(task.id, task.owner, native);
    let action = await h.advance(task.id, task.owner);
    if (action.kind !== 'native') throw new Error('missing implementation');
    for (const scoped of task.scope) writeFileSync(join(h.root, scoped), 'fixed\n');
    await h.submit(task.id, task.owner, { schemaVersion: 1, requestId: action.request.id, sourceDigest: h.snapshot().digest,
      native, outcome: 'completed', summary: 'injected author', issues: [] });
    action = await h.advance(task.id, task.owner);
    if (action.kind !== 'native') throw new Error('missing review');
    await h.submit(task.id, task.owner, { schemaVersion: 1, requestId: action.request.id, sourceDigest: h.snapshot().digest,
      native: { ...native, executorId: 'fresh-reviewer' }, outcome: 'completed', summary: 'injected independent review', issues: [] });
    const run = await h.verify(task.id, task.owner);
    candidates.push({ task, candidate, original: structuredClone(run), input: {
      candidateRoot: h.root, id: task.id, owner: task.owner, expectedDigest: run.digest!,
    } });
  }
  return { ...f, parentDirectory, candidates };
}
async function accept(h: DeliveryHarness, input: Awaited<ReturnType<typeof fixture>>['candidates'][number]['input']) {
  const run = await h.integrate(input);
  for (const check of run.task.checks) await h.check(run.task.id, run.task.owner, check.id);
  expect((await h.verify(run.task.id, run.task.owner)).verdict?.pass).toBe(true);
  git(h.root, 'add', '--', ...run.task.scope); git(h.root, 'commit', '-qm', 'accept candidate');
  return h.finish(run.task.id, run.task.owner, git(h.root, 'rev-parse', 'HEAD'));
}
it('integrates two reviewed same-base siblings serially and releases exact accepted child inputs', async () => {
  const f = await fixture();
  for (const row of f.candidates) {
    expect(() => createDeliveryCandidate(f.harness, { parentDirectory: f.parentDirectory,
      scope: ['product.txt'], acceptedParent: row.task.id })).toThrow();
    const run = await accept(f.harness, row.input);
    expect(run.status).toBe('complete');
    expect(hash(run.workflow)).toBe(hash(row.original.workflow));
    expect(JSON.parse(readFileSync(join(row.candidate.harness.directory, `${row.task.id}.json`), 'utf8'))).toEqual(row.original);
  }
  const child = createDeliveryCandidate(f.harness, { parentDirectory: f.parentDirectory, scope: ['other.txt'],
    acceptedParent: f.candidates[0].task.id, acceptedInputs: ['product.txt'] });
  expect(readFileSync(join(child.harness.root, 'product.txt'), 'utf8')).toBe('fixed\n');
  expect(child.acceptedSource?.commit).toBe(f.harness.read(f.candidates[0].task.id).commit);
});
it('defaults to full read dependencies and refuses stale sibling dependencies', async () => {
  const f = await fixture(null); await accept(f.harness, f.candidates[0].input);
  await expect(f.harness.integrate(f.candidates[1].input)).rejects.toThrow('INPUT_CHANGED');
});
it('default read closure includes files newly introduced by accepted siblings', async () => {
  const f = await fixture(null, false, true); await accept(f.harness, f.candidates[0].input);
  await expect(f.harness.integrate(f.candidates[1].input)).rejects.toThrow('INPUT_CHANGED');
});
it.each(['dirty', 'evaluator', 'source', 'review', 'digest'])('refuses %s tampering before canonical source writes', async kind => {
  const f = await fixture(), row = f.candidates[0], before = readFileSync(join(f.root, 'product.txt'), 'utf8');
  if (kind === 'dirty') writeFileSync(join(f.root, 'unrelated.txt'), 'dirty');
  if (kind === 'evaluator') { writeFileSync(join(f.root, 'coding-harness/check.mjs'), 'process.exit(0); // changed\n'); git(f.root, 'add', '.'); git(f.root, 'commit', '-qm', 'changed evaluator'); }
  if (kind === 'source') writeFileSync(join(row.candidate.harness.root, 'product.txt'), 'tampered\n');
  if (kind === 'review') { const run = structuredClone(row.original); run.workflow!.results.pop(); delete run.digest; run.digest = hash(run);
    writeFileSync(join(row.candidate.harness.directory, `${row.task.id}.json`), JSON.stringify(run)); row.input.expectedDigest = run.digest; }
  if (kind === 'digest') row.input.expectedDigest = '0'.repeat(64);
  await expect(f.harness.integrate(row.input)).rejects.toThrow();
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe(before);
  expect(f.harness.inspect().active).toBeNull();
});
it('resumes prepared integration without model work and refuses foreign owner', async () => {
  const f = await fixture(), row = f.candidates[0];
  await f.harness.integrate(row.input);
  const restarted = new DeliveryHarness(f.root);
  await expect(restarted.integrate({ ...row.input, owner: 'intruder' })).rejects.toThrow('OWNER');
  await restarted.integrate(row.input);
  expect((await restarted.next(row.task.id, row.task.owner)).kind).toBe('check');
  expect((await accept(restarted, row.input)).status).toBe('complete');
});
it('refuses committed unrelated drift and automatically pins omitted evaluator inputs and new runtime files', async () => {
  for (const path of ['unrelated.txt', 'coding-harness/check.mjs', 'coding-harness/new-runtime.mjs']) {
    const f = await fixture([]);
    writeFileSync(join(f.root, path), 'changed\n'); git(f.root, 'add', '--', path); git(f.root, 'commit', '-qm', 'unaccepted drift');
    await expect(f.harness.integrate(f.candidates[0].input)).rejects.toThrow(path === 'unrelated.txt' ? 'UNACCEPTED_DRIFT' : 'INPUT_CHANGED');
  }
});
it('recovers durable partially applied source after interruption without replaying model work', async () => {
  const f = await fixture([], true), row = f.candidates[0], { renameSync: rename } = await vi.importActual<typeof import('node:fs')>('node:fs');
  const fault = vi.mocked(fs.renameSync).mockImplementation((from, to) => {
    if (to === join(f.root, 'other.txt')) throw new Error('injected interruption');
    return rename(from, to);
  });
  await expect(f.harness.integrate(row.input)).rejects.toThrow('injected interruption'); fault.mockImplementation(rename);
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe('fixed\n');
  expect(readFileSync(join(f.root, 'other.txt'), 'utf8')).toBe('before\n');
  expect(f.harness.read(row.task.id).integration?.phase).toBe('applying');
  const restarted = new DeliveryHarness(f.root);
  expect((await accept(restarted, row.input)).status).toBe('complete');
});
it('finishes exact commit after restart and preserves receipt evidence independently of candidate logs', async () => {
  const f = await fixture(), row = f.candidates[0], run = await f.harness.integrate(row.input);
  for (const check of run.task.checks) await f.harness.check(run.task.id, run.task.owner, check.id);
  for (const check of row.original.checks) writeFileSync(check.stdout, 'candidate log later changed');
  git(f.root, 'add', '--', ...run.task.scope); git(f.root, 'commit', '-qm', 'accepted before interruption');
  const restarted = new DeliveryHarness(f.root);
  expect((await restarted.finish(run.task.id, run.task.owner, git(f.root, 'rev-parse', 'HEAD'))).status).toBe('complete');
});
it('preserves negative and superseded outcome history without treating it as current acceptance', async () => {
  const f = await fixture(), row = f.candidates[0], directory = join(row.candidate.harness.directory, 'runner'); mkdirSync(directory);
  const history = [false, true].map(success => {
    const body = { taskDigest: hash(row.task), sourceAfter: '0'.repeat(64), success,
      kernel: row.original.workflow!.results.at(-1)!.kernel };
    const receipt = { ...body, digest: hash(body) };
    writeFileSync(join(directory, `outcome-${randomUUID()}.json`), JSON.stringify(receipt)); return receipt;
  });
  const accepted = await accept(f.harness, row.input);
  expect(accepted.integration?.outcomeHistory).toHaveLength(2);
  expect(accepted.integration?.outcomeReceipts).toEqual([]);
  expect(accepted.integration?.outcomeHistory).toEqual(expect.arrayContaining(history));
});

it.each(['tests/proof.rs', 'coding-harness/check.mjs', 'other.txt'])('owner-pinned %s drift requires fresh checks/review without author replay', async path => {
  const f = await fixture(['coding-harness/check.mjs', 'other.txt']), row = f.candidates[0];
  const original = readFileSync(join(row.candidate.harness.directory, `${row.task.id}.json`), 'utf8');
  const before = readFileSync(join(f.root, path), 'utf8');
  writeFileSync(join(f.root, path), before + '\n// current input\n');
  git(f.root, 'add', path); git(f.root, 'commit', '-qm', 'owner-reviewed input change');
  await expect(f.harness.integrate(row.input)).rejects.toThrow('INPUT_CHANGED');
  const pin = { commit: git(f.root, 'rev-parse', 'HEAD'), sourceDigest: f.harness.snapshot().digest };
  const input = { ...row.input, revalidateAgainst: pin };
  await f.harness.integrate(input);
  await expect(f.harness.integrate(row.input)).rejects.toThrow('OWNER_OR_IDENTITY');
  for (const check of row.task.checks) await f.harness.check(row.task.id, row.task.owner, check.id);
  expect((await f.harness.verify(row.task.id, row.task.owner)).verdict?.pass).toBe(false);
  const stages: string[] = [];
  const result = await runIntegrationReview(f.harness, row.task.id, row.task.owner, { execute: async (request, files, checks) => {
    stages.push(request.stage);
    expect(files.find(file => file.path === path)?.content).toContain('current input');
    expect(checks).toContainEqual({ ownerRevalidation: pin });
    return { changes: [], response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, executorId: 'owner-current-source-reviewer' }, outcome: 'completed', summary: 'fresh review', issues: [] } };
  } });
  expect(result.success).toBe(true); expect(stages).toEqual(['review']);
  expect(readFileSync(join(row.candidate.harness.directory, `${row.task.id}.json`), 'utf8')).toBe(original);
  git(f.root, 'add', ...row.task.scope); git(f.root, 'commit', '-qm', 'accepted revalidation');
  expect((await f.harness.finish(row.task.id, row.task.owner, git(f.root, 'rev-parse', 'HEAD'))).status).toBe('complete');
});

it.each(['commit', 'digest', 'scope', 'owner'])('owner revalidation rejects stale or unauthorized %s before writes', async kind => {
  const f = await fixture(), row = f.candidates[0];
  if (kind === 'scope') { writeFileSync(join(f.root, 'product.txt'), 'foreign change\n'); git(f.root, 'add', '.'); git(f.root, 'commit', '-qm', 'scope changed'); }
  const before = readFileSync(join(f.root, 'product.txt'), 'utf8');
  await expect(f.harness.integrate({ ...row.input, ...(kind === 'owner' ? { owner: 'intruder' } : {}),
    revalidateAgainst: { commit: kind === 'commit' ? '0'.repeat(40) : git(f.root, 'rev-parse', 'HEAD'),
      sourceDigest: kind === 'digest' ? '0'.repeat(64) : f.harness.snapshot().digest } })).rejects.toThrow(kind === 'owner'
        ? 'DELIVERY_INTEGRATION_OWNER_MISMATCH' : kind === 'scope' ? 'DELIVERY_INTEGRATION_INPUT_CHANGED' : 'DELIVERY_INTEGRATION_REVALIDATION_STALE');
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe(before); expect(f.harness.inspect().active).toBeNull();
});

it('owner revalidation preserves current failed checks and never reaches reviewer', async () => {
  const f = await fixture(), row = f.candidates[0];
  writeFileSync(join(f.root, 'coding-harness/check.mjs'), 'process.exit(1);\n');
  git(f.root, 'add', '.'); git(f.root, 'commit', '-qm', 'stricter current evaluator');
  await f.harness.integrate({ ...row.input, revalidateAgainst: { commit: git(f.root, 'rev-parse', 'HEAD'), sourceDigest: f.harness.snapshot().digest } });
  const execute = vi.fn();
  const result = await runIntegrationReview(f.harness, row.task.id, row.task.owner, { execute });
  expect(result.success).toBe(false); expect(execute).not.toHaveBeenCalled();
  expect(f.harness.read(row.task.id).checks.at(-1)?.passed).toBe(false);
  expect(f.harness.read(row.task.id).integration?.original.verdict?.pass).toBe(true);
});

it.each(['dirty', 'reservation', 'source', 'review', 'digest'])('opt-in still rejects %s before canonical writes', async kind => {
  const f = await fixture(), row = f.candidates[0], before = readFileSync(join(f.root, 'product.txt'), 'utf8');
  if (kind === 'dirty') writeFileSync(join(f.root, 'dirty.txt'), 'uncommitted');
  if (kind === 'reservation') {
    const directory=join(f.harness.directory,'pool-active');mkdirSync(directory);
    saveReservations(directory,[{id:'active-reader',mutationPaths:['other.txt'],readPaths:['product.txt'],resources:[]}]);
  }
  if (kind === 'source') writeFileSync(join(row.candidate.harness.root, 'product.txt'), 'tampered\n');
  if (kind === 'review') {
    const run=structuredClone(row.original);run.workflow!.results.pop();delete run.digest;run.digest=hash(run);
    writeFileSync(join(row.candidate.harness.directory,`${row.task.id}.json`),JSON.stringify(run));row.input.expectedDigest=run.digest;
  }
  if (kind === 'digest') row.input.expectedDigest='0'.repeat(64);
  const code=kind==='dirty'?'DELIVERY_INTEGRATION_DIRTY_CANONICAL':kind==='reservation'?'DELIVERY_INTEGRATION_ACTIVE_DEPENDENCY':
    kind==='digest'?'DELIVERY_CANDIDATE_IDENTITY':'DELIVERY_CANDIDATE_NOT_REVIEWED';
  await expect(f.harness.integrate({...row.input,revalidateAgainst:{commit:git(f.root,'rev-parse','HEAD'),sourceDigest:f.harness.snapshot().digest}})).rejects.toThrow(code);
  expect(readFileSync(join(f.root,'product.txt'),'utf8')).toBe(before);expect(f.harness.inspect().active).toBeNull();
});

it.each(['added', 'deleted', 'deleted-parent', 'unchanged'])('owner review receives %s input evidence and never skips fresh review', async kind => {
  const f=await fixture(),row=f.candidates[0];
  const path=kind==='added'?'coding-harness/new-evaluator.mjs':'tests/proof.rs';
  if(kind==='added')writeFileSync(join(f.root,path),'// added current evaluator\n');
  if(kind==='deleted')rmSync(join(f.root,path));
  if(kind==='deleted-parent')rmSync(join(f.root,'tests'),{recursive:true});
  if(kind!=='unchanged'){git(f.root,'add','.');git(f.root,'commit','-qm','input migration');}
  await f.harness.integrate({...row.input,revalidateAgainst:{commit:git(f.root,'rev-parse','HEAD'),sourceDigest:f.harness.snapshot().digest}});
  for(const c of row.task.checks)await f.harness.check(row.task.id,row.task.owner,c.id);
  expect((await f.harness.verify(row.task.id,row.task.owner)).verdict?.pass).toBe(false);
  const action=await f.harness.next(row.task.id,row.task.owner);expect(action.kind).toBe('native');
  let calls=0;
  const result=await runIntegrationReview(f.harness,row.task.id,row.task.owner,{execute:async(request,files)=>{
    calls++;expect(request.stage).toBe('review');
    if(kind==='added')expect(files.find(f=>f.path===path)?.content).toBe('// added current evaluator\n');
    if(kind.startsWith('deleted'))expect(files.find(f=>f.path===path)).toEqual({path,content:null});
    return{changes:[],response:{schemaVersion:1,requestId:request.id,sourceDigest:request.sourceDigest,
      native:{...native,executorId:'fresh-migration-reviewer'},outcome:'completed',summary:'fresh current inputs',issues:[]}};
  }});
  expect(calls).toBe(1);expect(result.success).toBe(true);
});

it('rejects malformed owner pins at parsing boundary', async () => {
  const f=await fixture(),row=f.candidates[0];
  const pin={commit:git(f.root,'rev-parse','HEAD'),sourceDigest:f.harness.snapshot().digest};
  for(const value of [{...pin,commit:'short'},{...pin,sourceDigest:'x'.repeat(64)}]) {
    expect(()=>parseIntegrationInput({...row.input,revalidateAgainst:value})).toThrow('DELIVERY_INTEGRATION_REVALIDATION_PIN_REQUIRED');
  }
  expect(()=>parseIntegrationInput({...row.input,revalidateAgainst:{...pin,extra:true}})).toThrow('integration revalidation');
});

async function reviewedReplacement(f: Awaited<ReturnType<typeof fixture>>, text: string, resources: string[]) {
  const row = f.candidates[0];
  const replacement = createDeliveryCandidate(f.harness, { parentDirectory: f.parentDirectory,
    scope: row.task.scope, readPaths: row.task.readPaths, resources });
  const h = replacement.harness;
  await h.begin(row.task); await h.bind(row.task.id, row.task.owner, native);
  let action = await h.advance(row.task.id, row.task.owner);
  if (action.kind !== 'native') throw new Error('missing repair implementation');
  writeFileSync(join(h.root, 'product.txt'), text);
  if (row.task.scope.includes('new.txt')) writeFileSync(join(h.root, 'new.txt'), 'repaired new\n');
  await h.submit(row.task.id, row.task.owner, { schemaVersion: 1, requestId: action.request.id,
    sourceDigest: h.snapshot().digest, native, outcome: 'completed', summary: 'repaired proposal', issues: [] });
  action = await h.advance(row.task.id, row.task.owner);
  if (action.kind !== 'native') throw new Error('missing repair review');
  await h.submit(row.task.id, row.task.owner, { schemaVersion: 1, requestId: action.request.id,
    sourceDigest: h.snapshot().digest, native: { ...native, executorId: 'repair-proposal-reviewer' },
    outcome: 'completed', summary: 'fresh repaired proposal review', issues: [] });
  const repaired = await h.verify(row.task.id, row.task.owner);
  return { replacement, input: { ...row.input, candidateRoot: h.root, expectedDigest: repaired.digest!, recover: true as const } };
}
async function rejectedIntegration(resources: string[] = [], secondRound = false, combined = false, newScope = false) {
  const f = await fixture(undefined, combined, newScope, resources), row = f.candidates[0];
  const repair = await reviewedReplacement(f, 'repaired\n', resources);
  const next = secondRound ? await reviewedReplacement(f, 'repaired again\n', resources) : undefined;
  const pin = { commit: git(f.root, 'rev-parse', 'HEAD'), sourceDigest: f.harness.snapshot().digest };
  await f.harness.integrate({ ...row.input, revalidateAgainst: pin });
  await runIntegrationReview(f.harness, row.task.id, row.task.owner, { execute: async request => ({ changes: [],
    response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, executorId: 'rejected-canonical-reviewer' }, outcome: 'changes-requested',
      summary: 'canonical behavior defect', issues: ['repair scoped behavior'] } }) });
  return { ...f, row, ...repair, next, previous: f.harness.read(row.task.id) };
}

it('recovers all restored scope with an exact pin, truthful entry archive and normal finish', async () => {
  const f = await rejectedIntegration([], false, true, true);
  const candidateRun = readFileSync(join(f.row.candidate.harness.directory, `${f.row.task.id}.json`));
  for (const path of f.row.task.scope) {
    if (f.previous.integration!.sourceBefore.files[path]) writeFileSync(join(f.root, path), 'before\n');
    else rmSync(join(f.root, path));
  }
  const entry = f.harness.snapshot();
  expect(f.row.task.scope.every(path => entry.files[path] === f.previous.integration!.sourceBefore.files[path])).toBe(true);
  const recovered = await f.harness.integrate({ ...f.input,
    revalidateAgainst: { commit: git(f.root, 'rev-parse', 'HEAD'), sourceDigest: entry.digest } });
  const archive = recovered.events.find(event => event.kind === 'integration-recovery')!.reason;
  expect(JSON.parse(readFileSync(join(archive, 'run.json'), 'utf8'))).toEqual(f.previous);
  expect(JSON.parse(readFileSync(join(archive, 'entry-source.json'), 'utf8'))).toEqual({
    source: entry, scopeState: 'original', rejectedSourceDigest: f.previous.integration!.sourceAfter.digest });
  expect(recovered.integration!.sourceBefore).toEqual(entry);
  for (const path of f.row.task.scope) {
    if (entry.files[path]) expect(readFileSync(join(archive, 'source', path), 'utf8')).toBe('before\n');
    else expect(fs.existsSync(join(archive, 'source', path))).toBe(false);
    expect(readFileSync(join(f.row.candidate.harness.root, path), 'utf8')).toBe('fixed\n');
  }
  expect(entry.files['new.txt']).toBeUndefined();
  expect(readFileSync(join(f.root, 'new.txt'), 'utf8')).toBe('repaired new\n');
  expect((await f.harness.verify(f.row.task.id, f.row.task.owner)).verdict?.pass).toBe(false);
  const reviewed = await runIntegrationReview(f.harness, f.row.task.id, f.row.task.owner, { execute: async request => ({
    changes: [], response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, executorId: 'restored-scope-fresh-reviewer' }, outcome: 'completed',
      summary: 'fresh restored-scope review', issues: [] } }) });
  expect(reviewed.success).toBe(true);
  expect(f.harness.read(f.row.task.id).checks.slice(-2).every(check => check.passed && check.attempt === 2)).toBe(true);
  git(f.root, 'add', '--', ...f.row.task.scope); git(f.root, 'commit', '-qm', 'accept restored-scope recovery');
  expect((await f.harness.finish(f.row.task.id, f.row.task.owner, git(f.root, 'rev-parse', 'HEAD'))).status).toBe('complete');
  expect(readFileSync(join(f.row.candidate.harness.directory, `${f.row.task.id}.json`))).toEqual(candidateRun);
});

it.each(['mixed', 'unknown', 'no-pin'])('refuses %s rollback without changing source or rejected evidence', async kind => {
  const f = await rejectedIntegration([], false, true);
  writeFileSync(join(f.root, 'product.txt'), kind === 'unknown' ? 'unknown\n' : 'before\n');
  if (kind !== 'mixed') writeFileSync(join(f.root, 'other.txt'), 'before\n');
  const before = f.harness.snapshot();
  await expect(f.harness.integrate({ ...f.input, ...(kind === 'no-pin' ? {} : {
    revalidateAgainst: { commit: git(f.root, 'rev-parse', 'HEAD'), sourceDigest: before.digest } }) }))
    .rejects.toThrow('DELIVERY_INTEGRATION_RECOVERY_SOURCE_CHANGED');
  expect(f.harness.snapshot()).toEqual(before);
  expect(f.harness.read(f.row.task.id)).toEqual(f.previous);
  expect(fs.existsSync(join(f.harness.directory, `integration-recovery-${f.previous.digest}`))).toBe(false);
});

it.each(['unchanged', 'canonical-drift', 'environment-drift'])('recovers rejected integration after %s with fresh checks/review and retained history', async migration => {
  const f = await rejectedIntegration();
  if (migration === 'canonical-drift') {
    writeFileSync(join(f.root, 'coding-harness/check.mjs'), 'process.exit(0);\n// accepted harness change\n');
    git(f.root, 'add', 'coding-harness/check.mjs'); git(f.root, 'commit', '-qm', 'accepted harness change');
  }
  if (migration === 'environment-drift') vi.stubEnv('SF_TEST_REVALIDATION', 'current');
  if (migration !== 'unchanged') await expect(f.harness.integrate(f.input)).rejects.toThrow(migration === 'canonical-drift'
    ? 'DELIVERY_INTEGRATION_RECOVERY_SOURCE_CHANGED' : 'DELIVERY_CANDIDATE_CHECK_INVALID');
  const input = migration === 'unchanged' ? f.input : { ...f.input,
    revalidateAgainst: { commit: git(f.root, 'rev-parse', 'HEAD'), sourceDigest: f.harness.snapshot().digest } };
  const { recover: _recover, ...ordinary } = f.input;
  await expect(f.harness.integrate(ordinary)).rejects.toThrow('RUN_NOT_ACTIVE');
  const original = readFileSync(join(f.row.candidate.harness.directory, `${f.row.task.id}.json`), 'utf8');
  const rejectedLog = readFileSync(f.previous.checks[0].stdout);
  const recovered = await f.harness.integrate(input);
  expect(recovered.task.id).toBe(f.previous.task.id);
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe('repaired\n');
  const archive = recovered.events.find(event => event.kind === 'integration-recovery')!.reason;
  expect(JSON.parse(readFileSync(join(archive, 'run.json'), 'utf8'))).toEqual(f.previous);
  expect(readFileSync(join(archive, 'source/product.txt'), 'utf8')).toBe('fixed\n');
  expect((await f.harness.verify(f.row.task.id, f.row.task.owner)).verdict?.pass).toBe(false);
  for (const check of f.row.task.checks) await f.harness.check(f.row.task.id, f.row.task.owner, check.id);
  expect(readFileSync(f.previous.checks[0].stdout)).toEqual(rejectedLog);
  expect(f.harness.read(f.row.task.id).checks.at(-1)!.attempt).toBe(2);
  expect((await f.harness.verify(f.row.task.id, f.row.task.owner)).verdict?.pass).toBe(false);
  const stages: string[] = [];
  const reviewed = await runIntegrationReview(f.harness, f.row.task.id, f.row.task.owner, { execute: async (request, files) => {
    stages.push(request.stage); expect(files.find(file => file.path === 'product.txt')?.content).toBe('repaired\n');
    return { changes: [], response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, executorId: 'recovery-current-source-reviewer' }, outcome: 'completed', summary: 'fresh canonical repair review', issues: [] } };
  } });
  expect(reviewed.success).toBe(true); expect(stages).toEqual(['review']);
  if (migration === 'environment-drift') {
    vi.stubEnv('SF_TEST_REVALIDATION', 'changed-again');
    expect((await f.harness.verify(f.row.task.id, f.row.task.owner)).verdict?.pass).toBe(false);
    vi.stubEnv('SF_TEST_REVALIDATION', 'current');
    expect((await f.harness.verify(f.row.task.id, f.row.task.owner)).verdict?.pass).toBe(true);
  }
  git(f.root, 'add', 'product.txt'); git(f.root, 'commit', '-qm', 'accepted same-task recovery');
  expect((await f.harness.finish(f.row.task.id, f.row.task.owner, git(f.root, 'rev-parse', 'HEAD'))).status).toBe('complete');
  expect(readFileSync(join(f.row.candidate.harness.directory, `${f.row.task.id}.json`), 'utf8')).toBe(original);
});

it.each(['owner', 'source', 'task', 'reservation', 'pin-commit', 'pin-source', 'index', 'uncommitted-input', 'deleted-new-input'])('refuses unauthorized or stale recovery %s without replacing rejected bytes', async kind => {
  const f = await rejectedIntegration();
  if (kind === 'source') writeFileSync(join(f.root, 'product.txt'), 'foreign replacement\n');
  if (kind === 'task') {
    const run = f.replacement.harness.read(f.row.task.id); run.task.requirement = 'different outcome';
    delete run.digest; run.digest = hash(run);
    writeFileSync(join(f.replacement.harness.directory, `${f.row.task.id}.json`), JSON.stringify(run)); f.input.expectedDigest = run.digest;
  }
  if (kind === 'reservation') {
    const directory = join(f.harness.directory, 'pool-held-reader'); mkdirSync(directory);
    saveReservations(directory, [{ id: 'held-reader', mutationPaths: ['product.txt'], resources: [], readPaths: [] }]);
  }
  if (kind === 'index') git(f.root, 'add', 'product.txt');
  if (kind === 'uncommitted-input') writeFileSync(join(f.root, 'coding-harness/check.mjs'), '// foreign input\n');
  if (kind === 'deleted-new-input') {
    writeFileSync(join(f.root, 'coding-harness/new.mjs'), '// accepted new evaluator\n');
    git(f.root, 'add', 'coding-harness/new.mjs'); git(f.root, 'commit', '-qm', 'accepted new evaluator');
    rmSync(join(f.root, 'coding-harness/new.mjs'));
  }
  const before = f.harness.snapshot().digest;
  await expect(f.harness.integrate({ ...f.input, ...(kind === 'owner' ? { owner: 'intruder' } : {}),
    revalidateAgainst: { commit: kind === 'pin-commit' ? '0'.repeat(40) : git(f.root, 'rev-parse', 'HEAD'),
      sourceDigest: kind === 'pin-source' ? '0'.repeat(64) : before } })).rejects.toThrow();
  expect(f.harness.snapshot().digest).toBe(before);
  expect(f.harness.read(f.row.task.id)).toEqual(f.previous);
  expect(f.harness.inspect().active).toBeNull();
});

it('ignores deletion of managed paths excluded from source snapshots during pinned recovery', async () => {
  const f = await rejectedIntegration();
  mkdirSync(join(f.root, '.swarm')); writeFileSync(join(f.root, '.swarm/helper.json'), '{}');
  git(f.root, 'add', '.swarm/helper.json'); git(f.root, 'commit', '-qm', 'managed helper fixture');
  rmSync(join(f.root, '.swarm/helper.json'));
  const pin = { commit: git(f.root, 'rev-parse', 'HEAD'), sourceDigest: f.harness.snapshot().digest };
  expect((await f.harness.integrate({ ...f.input, revalidateAgainst: pin })).integration?.phase).toBe('prepared');
});

it('rejects altered archived check logs even when historical environment is allowed', async () => {
  const f = await rejectedIntegration(), archived = structuredClone(f.previous);
  vi.stubEnv('SF_TEST_REVALIDATION', 'current');
  archived.integration!.checkLogs[archived.integration!.original.checks[0].stdout] = Buffer.from('altered log').toString('base64');
  await expect(validateIntegrationEvidence(archived)).rejects.toThrow('DELIVERY_CANDIDATE_CHECK_INVALID');
});

it.each(['resources', 'readPaths', 'acceptedSource'])('retains admitted %s custody during same-task recovery', async field => {
  const f = await rejectedIntegration(['fixture-resource']);
  const file = join(f.harness.directory, `candidate-${hash(f.replacement.harness.root)}.json`);
  const custody = JSON.parse(readFileSync(file, 'utf8'));
  custody[field] = field === 'acceptedSource' ? { forged: true } : [];
  writeFileSync(file, JSON.stringify(custody));
  const directory = join(f.harness.directory, 'pool-independent-resource'); mkdirSync(directory);
  saveReservations(directory, [{ id: 'independent', mutationPaths: ['other.txt'], resources: ['fixture-resource'], readPaths: [] }]);
  await expect(f.harness.integrate(f.input)).rejects.toThrow('RECOVERY_CUSTODY_CHANGED');
  expect(f.harness.read(f.row.task.id)).toEqual(f.previous);
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe('fixed\n');
});

it('recovers two rejected rounds from original admitted scope and preserves every failed join', async () => {
  const f = await rejectedIntegration([], true);
  await f.harness.integrate(f.input);
  await runIntegrationReview(f.harness, f.row.task.id, f.row.task.owner, { execute: async request => ({ changes: [],
    response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, executorId: 'second-rejected-reviewer' }, outcome: 'changes-requested', summary: 'second defect', issues: ['repair again'] } }) });
  const second = f.harness.read(f.row.task.id);
  expect(second.status).toBe('paused');
  const recovered = await f.harness.integrate(f.next!.input);
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe('repaired again\n');
  const archives = recovered.events.filter(event => event.kind === 'integration-recovery').map(event => event.reason);
  expect(archives).toHaveLength(2);
  expect(JSON.parse(readFileSync(join(archives[0], 'run.json'), 'utf8'))).toEqual(f.previous);
  expect(JSON.parse(readFileSync(join(archives[1], 'run.json'), 'utf8'))).toEqual(second);
  expect(readFileSync(join(archives[1], 'source/product.txt'), 'utf8')).toBe('repaired\n');
  const reviewed = await runIntegrationReview(f.harness, f.row.task.id, f.row.task.owner, { execute: async request => ({ changes: [],
    response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, executorId: 'third-fresh-reviewer' }, outcome: 'completed', summary: 'final repair review', issues: [] } }) });
  expect(reviewed.success).toBe(true);
  expect(f.harness.read(f.row.task.id).checks.at(-1)!.attempt).toBe(3);
});

it('resumes interrupted recovery through ordinary integrate without losing archived rejection', async () => {
  const f = await rejectedIntegration();
  const { renameSync: rename } = await vi.importActual<typeof import('node:fs')>('node:fs');
  vi.mocked(fs.renameSync).mockImplementation((from, to) => {
    if (to === join(f.root, 'product.txt')) throw new Error('interrupted recovery apply');
    return rename(from, to);
  });
  await expect(f.harness.integrate(f.input)).rejects.toThrow('interrupted recovery apply');
  vi.mocked(fs.renameSync).mockImplementation(rename);
  const { recover: _recover, ...ordinary } = f.input;
  const run = await new DeliveryHarness(f.root).integrate(ordinary);
  expect(run.integration?.phase).toBe('prepared');
  expect(run.events.filter(event => event.kind === 'integration-recovery')).toHaveLength(1);
  expect(run.checks).toEqual(f.previous.checks);
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe('repaired\n');
});
