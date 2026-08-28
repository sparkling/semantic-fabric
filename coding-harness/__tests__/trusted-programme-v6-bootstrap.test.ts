// SPDX-License-Identifier: MIT
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync,
  realpathSync, readdirSync, rmSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { RECEIPT_FAILURE_CODES } from '../src/failure-code.js';
const GIT = '/usr/bin/git';
const NODE = '/usr/bin/node';
const PRIMARY_ENTRY = 'coding-harness/dist/issue-8-program.js';
const V6_ENTRY = 'coding-harness/dist/programme-v6-program.js';
const DEFAULT_TASK = 'coding-harness/config/programme-v5-acceptance.json';
const BASE_GATE = { schemaVersion: 1 };
const BASE_POLICY = { authority: 'development-only-no-promotion', gateContract: BASE_GATE,
  policyId: 'semantic-fabric-programme-v5-policy-v1', schemaVersion: 1 };
const POLICY = canonical({ authority: 'development-only-no-promotion', basePolicy: BASE_POLICY,
  basePolicyFingerprint: sha256(canonical(BASE_POLICY)), gateContract: {
    authority: 'development-only-no-promotion', baseGateContract: BASE_GATE,
    baseGateContractDigest: sha256(canonical(BASE_GATE)),
    contractId: 'semantic-fabric-programme-gate-contract-v2', envelope: { schemaVersion: 6 },
    schemaVersion: 2 }, policyId: 'semantic-fabric-programme-v6-policy-v2', schemaVersion: 2 });
const RECEIPT_DIGEST = 'a'.repeat(64), ACCEPTANCE_DIGEST = 'b'.repeat(64);
const ENVELOPE_DIGEST = 'c'.repeat(64), CLAIM_DIGEST = 'd'.repeat(64);
const REPLAY_DIGEST = 'e'.repeat(64), CANDIDATE_DIGEST = 'f'.repeat(64);
const LEGACY_LAUNCHER_DIGEST = 'b6be487acfd45c5e947f1cf7b2a1fdbbf789b46918a79f2fdb3724bf8904b660';
const roots: string[] = [];
interface Behavior { readonly policyBlob: string; readonly changingPolicy?: boolean;
  readonly executeThrows?: boolean; readonly wrongReviewFingerprint?: boolean;
  readonly wrongFingerprint?: boolean; }
interface Fixture { readonly repository: string; readonly runtime: string;
  readonly store: string; readonly commit: string; readonly events: string;
  readonly policyFingerprint: string; }
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
describe('trusted programme-v6 bootstrap', () => {
  it('anchors canonical policy bytes after prepare and before execute', () => {
      const fixture = controllerFixture({ policyBlob: POLICY });
      const result = runLauncher(fixture);
      const policyFingerprint = sha256(POLICY);
      const events = readEvents(fixture);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(events.map(({ event }) => event)).toEqual(['prepare', 'execute', 'seal']);
      expect(events[0]).toMatchObject({
        event: 'prepare',
        taskPath: DEFAULT_TASK,
        bootstrapTaskPath: DEFAULT_TASK,
      });
      expect(events[1]).toEqual({
        event: 'execute',
        policyBlob: POLICY,
        policyFingerprint,
        argumentCount: 1,
      });
      expect(events[2]).toEqual({ event: 'seal', runtimePresent: false });
      const output = JSON.parse(result.stdout);
      expect(output).toMatchObject({
        status: 'pass', transactionStatus: 'pass',
        reason: null,
        receiptDigest: RECEIPT_DIGEST,
        candidateTransactionEvidenceDigest: CANDIDATE_DIGEST,
        programmeAcceptanceDigest: ACCEPTANCE_DIGEST,
        envelopeDigest: ENVELOPE_DIGEST,
        policyFingerprint,
        executionClaimDigest: CLAIM_DIGEST,
      });
      expect(output.launchReceiptDigest).toBe(sha256(canonical({
        schemaVersion: 1,
        domain: 'semantic-fabric/programme-v6/execution-launch/v1',
        operation: 'programme-v6-execution-launch',
        controllerCommit: fixture.commit,
        taskPath: DEFAULT_TASK,
        outerPolicyFingerprint: policyFingerprint,
        basePolicyFingerprint: sha256(canonical(BASE_POLICY)), transactionStatus: 'pass',
        receiptDigest: RECEIPT_DIGEST, envelopeDigest: ENVELOPE_DIGEST,
        candidateTransactionEvidenceDigest: CANDIDATE_DIGEST,
        executionClaimDigest: CLAIM_DIGEST,
      })));
      expect(existsSync(fixture.store)).toBe(false);
    });
  it('emits a replayable prepare-only policy receipt after complete cleanup', () => {
      const fixture = controllerFixture({ policyBlob: POLICY });
      const result = runLauncher(fixture, [], fixture.policyFingerprint, true);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(readEvents(fixture)).toEqual([{ event: 'prepare-review' }]);
      expect(existsSync(fixture.store)).toBe(false);
      const receipt = JSON.parse(result.stdout);
      const { policyReviewReceiptDigest, ...body } = receipt;
      expect(body).toMatchObject({
        schemaVersion: 1,
        authority: 'development-only-no-promotion',
        operation: 'programme-v6-policy-review',
        controllerCommit: fixture.commit,
        taskPath: DEFAULT_TASK,
        runId: 'programme_v6_run',
        swarmId: 'programme_v6_swarm', coordinationTaskId: 'programme_v6_task',
        hiveId: 'hierarchical', consensusId: 'raft',
        policyFingerprint: fixture.policyFingerprint,
        policyBlob: POLICY,
      });
      expect(policyReviewReceiptDigest).toBe(sha256(canonical(body)));
    });
  it('verifies replay without preparing or executing and preserves recorded rejection', () => {
      const fixture = controllerFixture({ policyBlob: POLICY });
      const result = runLauncher(fixture, [], fixture.policyFingerprint, false, true);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(readEvents(fixture)).toEqual([
        { event: 'replay' }, { event: 'replay-seal', runtimePresent: false },
      ]);
      expect(JSON.parse(result.stdout)).toMatchObject({
        verificationStatus: 'verified', transactionStatus: 'pass',
        recordedStatus: 'gated',
        recordedReason: 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED',
        replayReceiptDigest: REPLAY_DIGEST,
        receiptDigest: RECEIPT_DIGEST,
        candidateTransactionEvidenceDigest: CANDIDATE_DIGEST,
        executionClaimDigest: CLAIM_DIGEST,
        basePolicyFingerprint: JSON.parse(POLICY).basePolicyFingerprint,
        policyFingerprint: fixture.policyFingerprint,
      });
    });
  it('rejects a false prepare-only fingerprint without leaking its store', () => {
      const fixture = controllerFixture({ policyBlob: POLICY, wrongReviewFingerprint: true });
      const result = runLauncher(fixture, [], fixture.policyFingerprint, true);
      expect(result.status).toBe(1);
      expect(result.stdout).toBe('');
      expect(JSON.parse(result.stderr)).toEqual({
        status: 'error',
        reason: 'HARNESS_BOOTSTRAP_POLICY_REVIEW_FINGERPRINT_MISMATCH',
      });
      expect(readEvents(fixture)).toEqual([{ event: 'prepare-review' }]);
      expect(existsSync(fixture.store)).toBe(false);
    });
  it('rejects changing, noncanonical, and malformed policy without execute', () => {
      const cases: readonly [Behavior, string][] = [
        [
          { policyBlob: POLICY, changingPolicy: true },
          'HARNESS_BOOTSTRAP_PREPARED_INVALID',
        ],
        [
          { policyBlob: '{"zeta":1,"alpha":[{"beta":true}]}' },
          'HARNESS_BOOTSTRAP_POLICY_CANONICAL_INVALID',
        ],
        [
          { policyBlob: '{"alpha":' },
          'HARNESS_BOOTSTRAP_POLICY_JSON_INVALID',
        ],
      ];
      for (const [behavior, reason] of cases) {
        const fixture = controllerFixture(behavior);
        const result = runLauncher(fixture);
        const events = readEvents(fixture);
        expect(result.status).toBe(1);
        expect(JSON.parse(result.stderr)).toEqual({ status: 'error', reason });
        expect(events.map(({ event }) => event)).toEqual(['prepare', 'abort']);
        expect(events.filter(({ event }) => event === 'abort')).toHaveLength(1);
        expect(events.some(({ event }) => event === 'execute')).toBe(false);
        expect(existsSync(fixture.store)).toBe(false);
      }
    });
  it('rejects an externally anchored fingerprint mismatch before execute', () => {
      const fixture = controllerFixture({ policyBlob: POLICY });
      const result = runLauncher(fixture, [], 'd'.repeat(64));
      const events = readEvents(fixture);
      expect(result.status).toBe(1);
      expect(JSON.parse(result.stderr)).toEqual({
        status: 'error',
        reason: 'HARNESS_BOOTSTRAP_POLICY_FINGERPRINT_MISMATCH',
      });
      expect(events.map(({ event }) => event)).toEqual(['prepare', 'abort']);
      expect(events.some(({ event }) => event === 'execute')).toBe(false);
      expect(existsSync(fixture.store)).toBe(false);
    });
  it('rejects a sealed fingerprint that differs from the launcher anchor', () => {
      const fixture = controllerFixture({ policyBlob: POLICY, wrongFingerprint: true });
      const result = runLauncher(fixture);
      const events = readEvents(fixture);
      expect(result.status).toBe(1);
      expect(JSON.parse(result.stderr)).toEqual({
        status: 'error',
        reason: 'HARNESS_BOOTSTRAP_SEALED_OUTCOME_INVALID',
      });
      expect(events.map(({ event }) => event)).toEqual(['prepare', 'execute', 'seal']);
      expect(events.some(({ event }) => event === 'abort')).toBe(false);
      expect(existsSync(fixture.store)).toBe(false);
    });
  it('calls abort exactly once when execute does not complete successfully', () => {
      const fixture = controllerFixture({ policyBlob: POLICY, executeThrows: true });
      const result = runLauncher(fixture);
      const events = readEvents(fixture);
      expect(result.status).toBe(1);
      expect(JSON.parse(result.stderr)).toEqual({
        status: 'error',
        reason: 'HARNESS_TRANSACTION_FAILED',
      });
      expect(events.map(({ event }) => event)).toEqual(['prepare', 'execute', 'abort']);
      expect(events.filter(({ event }) => event === 'abort')).toHaveLength(1);
      expect(existsSync(fixture.store)).toBe(false);
    });
  it('binds an explicit v6 task without falling back to the v4 default', () => {
      const fixture = controllerFixture({ policyBlob: POLICY });
      const taskPath = 'coding-harness/config/alternate-programme-v5-acceptance.json';
      const result = runLauncher(fixture, ['--task-path', taskPath]);
      expect(result.status).toBe(0);
      expect(readEvents(fixture)[0]).toMatchObject({
        event: 'prepare',
        taskPath,
        bootstrapTaskPath: taskPath,
      });
      expect(result.stdout).not.toContain('issue-8-acceptance.json');
    });
  it('keeps the legacy bootstrap byte-frozen and imports only the secondary v6 entry', () => {
    const legacy = readFileSync(
      new URL('../scripts/launch-issue-8.mjs', import.meta.url),
    );
    const launcher = readFileSync(
      new URL('../scripts/launch-programme-v6.mjs', import.meta.url),
      'utf8',
    );
    expect(sha256(legacy)).toBe(LEGACY_LAUNCHER_DIGEST);
    expect(launcher).toContain('const PRIMARY_ENTRY="coding-harness/dist/issue-8-program.js"');
    expect(launcher).toContain('const V6_ENTRY="coding-harness/dist/programme-v6-program.js"');
    expect(launcher).toContain('safePath(privateRuntime,V6_ENTRY)');
    expect(launcher).not.toContain('trustedControllerMain');
    for (const reason of ['"HARNESS_PROGRAMME_ACCEPTANCE_REJECTED"', '"HARNESS_PROGRAMME_V5_NATIVE_EXECUTABLE_BINDING_MISMATCH"']) expect(launcher).toContain(reason);
    for (const code of RECEIPT_FAILURE_CODES) expect(launcher).toContain(`"${code}"`);
  });
});
function controllerFixture(behavior: Behavior): Fixture {
  const repository = privateTemporary('programme-v5-bootstrap-repository-');
  const runtime = privateTemporary('programme-v5-bootstrap-runtime-');
  const template = join(runtime, 'empty-template');
  mkdirSync(template, { mode: 0o700 });
  git(repository, ['init', '--quiet', '--template=' + template]);
  git(repository, ['config', 'user.name', 'Harness Test']);
  git(repository, ['config', 'user.email', 'harness@example.invalid']);
  const manifest = '{"schemaVersion":1}\n';
  const lockfile = '{"lockfileVersion":3}\n';
  const packageJson = '{"name":"bootstrap-fixture","private":true,"type":"module"}\n';
  const files = new Map<string, string>([
    ['coding-harness/.harness/manifest.json', manifest],
    ['coding-harness/package-lock.json', lockfile],
    ['coding-harness/package.json', packageJson],
    [PRIMARY_ENTRY, 'export const legacyEntry = true;\n'],
    [V6_ENTRY, controllerModule(behavior)],
    ['coding-harness/node_modules/bootstrap-fixture/package.json', '{"name":"bootstrap-fixture","version":"1.0.0"}\n'],
  ]);
  for (const [path, contents] of files) write(repository, path, contents);
  const outputs = Object.fromEntries(
    [PRIMARY_ENTRY, V6_ENTRY].sort().map((path) => [path, sha256(files.get(path)!)]),
  );
  const productionFiles = {
    'coding-harness/node_modules/bootstrap-fixture/package.json':
      sha256(files.get('coding-harness/node_modules/bootstrap-fixture/package.json')!),
  };
  const buildBody = {
    schemaVersion: 1,
    authority: 'development-only-no-promotion',
    runtimeEntry: PRIMARY_ENTRY,
    harnessManifestDigest: sha256(manifest),
    lockfileDigest: sha256(lockfile),
    outputs,
    productionFiles,
  };
  write(repository, 'coding-harness/.harness/controller-build.json', JSON.stringify({
    ...buildBody,
    runtimeTreeDigest: sha256(JSON.stringify(buildBody)),
  }, null, 2) + '\n');
  git(repository, ['add', '--', '.']);
  git(repository, ['commit', '--quiet', '-m', 'controller']);
  const commit = gitText(repository, ['rev-parse', 'HEAD']);
  const store = mkdtempSync(join(runtime, 'semantic-fabric-controller-store-'));
  git(repository, ['init', '--quiet', '--bare', '--template=' + template, store]);
  const pack = git(repository, [
    'pack-objects', '--stdout', '--revs', '--no-reuse-object', '--no-reuse-delta',
  ], Buffer.from(commit + '\n'));
  git(repository, [
    '-c', 'pack.writeReverseIndex=false', '--git-dir=' + store,
    'index-pack', '--strict', '--stdin',
  ], pack);
  git(repository, ['--git-dir=' + store, 'update-ref', 'refs/heads/controller', commit]);
  git(repository, ['--git-dir=' + store, 'symbolic-ref', 'HEAD', 'refs/heads/controller']);
  harden(store);
  return { repository, runtime, store, commit, events: join(repository, 'launcher-events.jsonl'),
    policyFingerprint: sha256(behavior.policyBlob) };
}
function controllerModule(behavior: Behavior): string {
  const setup = behavior.changingPolicy
    ? [
        'let reads=0;',
        'Object.defineProperty(handle,"policyBlob",{enumerable:true,get(){',
        'reads+=1;return reads===1?POLICY:CHANGED_POLICY;}});',
      ].join('')
    : 'handle.policyBlob=POLICY;';
  return [
    "import{createHash}from'node:crypto';",
    "import{appendFileSync,existsSync}from'node:fs';",
    "import{join}from'node:path';",
    "import{fileURLToPath}from'node:url';",
    'const MODULE_PATH=fileURLToPath(import.meta.url);',
    'const POLICY=' + JSON.stringify(behavior.policyBlob) + ';',
    'const CHANGED_POLICY=' + JSON.stringify('{"alpha":[],"zeta":2}') + ';',
    'function flag(argv,name){const index=argv.indexOf(name);',
    'return index<0?undefined:argv[index+1];}',
    'function digest(value){const sorted=Object.fromEntries(Object.keys(value).sort().map(key=>[key,value[key]]));return createHash("sha256").update(JSON.stringify(sorted)).digest("hex");}',
    'function record(argv,event){appendFileSync(join(flag(argv,"--repository"),',
    '"launcher-events.jsonl"),JSON.stringify(event)+"\\n");}',
    'export async function prepareReviewableProgrammeV6Policy(argv){',
    'record(argv,{event:"prepare-review"});return{policyBlob:POLICY,',
    'policyFingerprint:' + JSON.stringify(
      behavior.wrongReviewFingerprint ? 'f'.repeat(64) : sha256(behavior.policyBlob),
    ) + '};}',
    'export async function replayTrustedProgrammeV6(argv){record(argv,{event:"replay"});',
    'return{status:"gated",reason:"HARNESS_PROGRAMME_ACCEPTANCE_REJECTED",async seal(){',
    'record(argv,{event:"replay-seal",runtimePresent:existsSync(MODULE_PATH)});return{',
    'verificationStatus:"verified",transactionStatus:"pass",recordedStatus:"gated",',
    'recordedReason:"HARNESS_PROGRAMME_ACCEPTANCE_REJECTED",',
    'receiptPath:join(flag(argv,"--repository"),"coding-harness",".metaharness","runs",',
    'flag(argv,"--run-id")+".replay.json"),replayReceiptDigest:'
      + JSON.stringify(REPLAY_DIGEST) + ',envelopeDigest:' + JSON.stringify(ENVELOPE_DIGEST) + ',',
    'receiptDigest:' + JSON.stringify(RECEIPT_DIGEST) + ',basePolicyFingerprint:JSON.parse(POLICY).basePolicyFingerprint,executionClaimDigest:' + JSON.stringify(CLAIM_DIGEST) + ',',
    'candidateTransactionEvidenceDigest:' + JSON.stringify(CANDIDATE_DIGEST) + ',',
    'policyFingerprint:flag(argv,"--expected-policy-fingerprint"),launchReceiptDigest:digest({',
    'schemaVersion:1,domain:"semantic-fabric/programme-v6/replay-launch/v1",operation:"programme-v6-replay-launch",controllerCommit:flag(argv,"--controller-commit"),',
    'taskPath:flag(argv,"--task-path")??' + JSON.stringify(DEFAULT_TASK) + ',outerPolicyFingerprint:flag(argv,"--expected-policy-fingerprint"),basePolicyFingerprint:JSON.parse(POLICY).basePolicyFingerprint,',
    'envelopeDigest:' + JSON.stringify(ENVELOPE_DIGEST) + ',',
    'transactionStatus:"pass",receiptDigest:' + JSON.stringify(RECEIPT_DIGEST) + ',',
    'candidateTransactionEvidenceDigest:' + JSON.stringify(CANDIDATE_DIGEST) + ',',
    'executionClaimDigest:' + JSON.stringify(CLAIM_DIGEST) + '})};}};}',
    'export async function prepareTrustedProgrammeV6(argv,bootstrap){',
    'const taskPath=flag(argv,"--task-path")??' + JSON.stringify(DEFAULT_TASK) + ';',
    'const expectedPolicyFingerprint=flag(argv,"--expected-policy-fingerprint");',
    'record(argv,{event:"prepare",taskPath,bootstrapTaskPath:bootstrap.taskPath});',
    'const handle={',
    'async execute(policyBlob){',
    'record(argv,{event:"execute",policyBlob,',
    'policyFingerprint:expectedPolicyFingerprint,argumentCount:arguments.length});',
    behavior.executeThrows
      ? 'throw new Error("HARNESS_TRANSACTION_FAILED");'
      : '',
    'return{status:"pass",reason:null,async seal(){',
    'record(argv,{event:"seal",runtimePresent:existsSync(MODULE_PATH)});',
    'return{status:"pass",transactionStatus:"pass",receiptPath:join(flag(argv,"--repository"),',
    '"coding-harness",".metaharness","runs",flag(argv,"--run-id")+".json"),',
    'receiptDigest:' + JSON.stringify(RECEIPT_DIGEST) + ',',
    'candidateTransactionEvidenceDigest:' + JSON.stringify(CANDIDATE_DIGEST) + ',',
    'programmeAcceptanceDigest:' + JSON.stringify(ACCEPTANCE_DIGEST) + ',',
    'envelopeDigest:' + JSON.stringify(ENVELOPE_DIGEST) + ',',
    'executionClaimDigest:' + JSON.stringify(CLAIM_DIGEST) + ',',
    behavior.wrongFingerprint
      ? 'policyFingerprint:"f".repeat(64)};'
      : 'policyFingerprint:expectedPolicyFingerprint};',
    '}};},',
    'async abort(){record(argv,{event:"abort"});}',
    '};',
    setup,
    'return handle;',
    '}',
    '',
  ].join('\n');
}
function runLauncher(
  fixture: Fixture,
  extraArgs: readonly string[] = [],
  expectedPolicyFingerprint = fixture.policyFingerprint,
  policyReview = false,
  replay = false,
) {
  const launcher = readFileSync(
    new URL('../scripts/launch-programme-v6.mjs', import.meta.url),
  );
  const previousUmask = process.umask(0o077);
  try {
    return spawnSync(NODE, [
      '--no-addons', '--disable-proto=throw', '--input-type=module', '-',
      '--repository', fixture.repository,
      '--controller-store', fixture.store,
      '--controller-commit', fixture.commit,
      '--run-id', 'programme_v6_run',
      '--swarm-id', 'programme_v6_swarm',
      '--coordination-task-id', 'programme_v6_task',
      '--hive-id', 'hierarchical',
      '--consensus-id', 'raft',
      ...(policyReview
        ? ['--policy-review', 'prepare-only']
        : replay ? [
            '--replay', 'verify-only',
            '--expected-policy-fingerprint', expectedPolicyFingerprint,
            '--policy-review-receipt', join(
              fixture.repository, 'coding-harness', '.metaharness', 'runs',
              'programme_v6_run.policy-review.json',
            ),
            '--envelope-receipt', join(
              fixture.repository, 'coding-harness', '.metaharness', 'runs',
              'programme_v6_run.json',
            ),
            '--receipt-path', join(
              fixture.repository, 'coding-harness', '.metaharness', 'runs',
              'programme_v6_run.replay.json',
            ),
          ] : [
            '--expected-policy-fingerprint', expectedPolicyFingerprint,
            '--policy-review-receipt', join(
              fixture.repository, 'coding-harness', '.metaharness', 'runs',
              'programme_v6_run.policy-review.json',
            ),
          ]),
      ...extraArgs,
    ], {
      input: launcher,
      env: {
        XDG_RUNTIME_DIR: fixture.runtime,
        DBUS_SESSION_BUS_ADDRESS: 'unix:path=' + fixture.runtime + '/bus',
        LANG: 'C.UTF-8',
      },
      encoding: 'utf8',
      maxBuffer: 2_000_000,
    });
  } finally {
    process.umask(previousUmask);
  }
}
function readEvents(fixture: Fixture): Array<Record<string, unknown>> {
  if (!existsSync(fixture.events)) return [];
  return readFileSync(fixture.events, 'utf8').trim().split('\n')
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line));
}
function write(root: string, path: string, contents: string): void {
  const target = join(root, path);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, contents);
  chmodSync(target, 0o600);
}

function privateTemporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  chmodSync(root, 0o700);
  roots.push(root);
  return root;
}

function harden(directory: string): void {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) harden(path);
    else chmodSync(path, 0o400);
  }
  chmodSync(directory, 0o500);
}

function git(cwd: string, args: readonly string[], input?: Buffer): Buffer {
  const result = spawnSync(GIT, args, {
    cwd,
    input,
    env: {
      PATH: '/usr/bin:/bin',
      HOME: '/nonexistent',
      LANG: 'C',
      LC_ALL: 'C',
      GIT_CONFIG_NOSYSTEM: '1',
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_ATTR_NOSYSTEM: '1',
      GIT_NO_REPLACE_OBJECTS: '1',
      GIT_NO_LAZY_FETCH: '1',
    },
    maxBuffer: 20_000_000,
  });
  if (result.status !== 0) throw new Error(result.stderr.toString('utf8'));
  return result.stdout;
}

function gitText(cwd: string, args: readonly string[]): string {
  return git(cwd, args).toString('utf8').trim();
}

function canonical(value: unknown): string {
  if (Array.isArray(value)) return JSON.stringify(value.map(canonicalValue));
  return JSON.stringify(canonicalValue(value));
}

function canonicalValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalValue);
  if (value !== null && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return Object.fromEntries(Object.keys(record).sort()
      .map((key) => [key, canonicalValue(record[key])]));
  }
  return value;
}

function sha256(value: string | Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}
