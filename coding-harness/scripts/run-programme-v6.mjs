// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  chmodSync, existsSync, lstatSync, mkdtempSync, readdirSync, realpathSync, rmSync,
} from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  assertPathAbsent, canonicalDirectory, deepFreeze, digest, exactKeys, gitObject,
  opaque, parsePolicyReviewReceipt, privateDirectory, receiptPath, sha256,
  programmeV6ReplayLaunchDigest,
  stablePrivateFileDigest, stableReadPrivateReceipt, taskPathValue, trustedExecutable, validDigest,
  writePrivateReceipt,
} from './programme-v6-operator-support.mjs';

export { parsePolicyReviewReceipt } from './programme-v6-operator-support.mjs';

const GIT = '/usr/bin/git';
const NODE = '/usr/bin/node';
const NODE_DIGEST = '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc';
const GIT_DIGEST = '2a8c18fbf43da9f692d75474c72bea9dfd796c260b0f3dfe456376abc3bbd668';
const LAUNCHER_PATH = 'coding-harness/scripts/launch-programme-v6.mjs';

export function createPackedControllerStore(input) {
  const repositoryRoot = canonicalDirectory(input.repositoryRoot, 'REPOSITORY_INVALID');
  const runtimeRoot = privateDirectory(input.runtimeRoot, 'RUNTIME_INVALID');
  const controllerCommit = gitObject(input.controllerCommit);
  if (gitText(repositoryRoot, ['rev-parse', '--verify', 'HEAD']) !== controllerCommit
    || gitText(repositoryRoot, ['rev-parse', '--verify', `${controllerCommit}^{commit}`])
      !== controllerCommit) {
    throw new Error('HARNESS_OPERATOR_CONTROLLER_COMMIT_MISMATCH');
  }
  const template = mkdtempSync(join(runtimeRoot, 'semantic-fabric-empty-template-'));
  const store = mkdtempSync(join(runtimeRoot, 'semantic-fabric-controller-store-'));
  chmodSync(template, 0o700);
  chmodSync(store, 0o700);
  const templateIdentity = directoryIdentity(template);
  const storeIdentity = directoryIdentity(store);
  let output;
  let failure;
  try {
    gitChecked(repositoryRoot, ['init', '--quiet', '--bare', `--template=${template}`, store]);
    const pack = gitChecked(repositoryRoot, [
      '-c', 'pack.threads=1',
      'pack-objects', '--stdout', '--revs', '--window=0', '--depth=0',
      '--no-reuse-object', '--no-reuse-delta',
    ], Buffer.from(`${controllerCommit}\n`, 'utf8'), 1_000_000_000);
    gitChecked(repositoryRoot, [
      '-c', 'pack.writeReverseIndex=false', `--git-dir=${store}`,
      'index-pack', '--strict', '--stdin',
    ], pack, 1_000_000_000);
    gitChecked(repositoryRoot, [
      `--git-dir=${store}`, 'update-ref', 'refs/heads/controller', controllerCommit,
    ]);
    gitChecked(repositoryRoot, [
      `--git-dir=${store}`, 'symbolic-ref', 'HEAD', 'refs/heads/controller',
    ]);
    hardenTree(store);
    const digest = controllerStoreDigest(store, controllerCommit);
    output = Object.freeze({
      path: store,
      digest,
      cleanup: () => cleanupStore(
        runtimeRoot, store, storeIdentity, digest, controllerCommit,
      ),
    });
  } catch (error) {
    failure = error;
  }
  try { cleanupStore(runtimeRoot, template, templateIdentity); }
  catch (cleanupError) {
    failure = combineFailure(failure, cleanupError, 'HARNESS_OPERATOR_TEMPLATE_CLEANUP_FAILED');
  }
  if (failure !== undefined) {
    try { cleanupStore(runtimeRoot, store, storeIdentity); }
    catch (cleanupError) {
      failure = combineFailure(failure, cleanupError, 'HARNESS_OPERATOR_STORE_CLEANUP_FAILED');
    }
    throw failure;
  }
  return output;
}

export function runProgrammeV6Operator(argv, environment = process.env) {
  validateOperatorProcess(environment);
  const invocation = parseInvocation(argv);
  const runtimeRoot = privateDirectory(environment.XDG_RUNTIME_DIR, 'RUNTIME_INVALID');
  const receipt = invocation.operation !== 'policy-review'
    ? parsePolicyReviewReceipt(
        stableReadPrivateReceipt(invocation.repositoryRoot, invocation.policyReviewReceipt, 6_000_000),
        invocation,
      )
    : null;
  if (invocation.operation === 'policy-review') {
    assertPathAbsent(invocation.receiptPath, 'HARNESS_OPERATOR_POLICY_RECEIPT_EXISTS');
  }
  if (receipt !== null && (receipt.policyFingerprint !== invocation.expectedPolicyFingerprint)) {
    throw new Error('HARNESS_OPERATOR_EXPECTED_POLICY_MISMATCH');
  }
  const store = createPackedControllerStore({ ...invocation, runtimeRoot });
  let output;
  let failure;
  try {
    if (receipt !== null && receipt.controllerStoreDigest !== store.digest) {
      throw new Error('HARNESS_OPERATOR_CONTROLLER_STORE_REPLAY_MISMATCH');
    }
    const launcher = gitChecked(store.path, [
      'show', `${invocation.controllerCommit}:${LAUNCHER_PATH}`,
    ], undefined, 10_000_000);
    const operationArgs = invocation.operation === 'policy-review'
      ? ['--policy-review', 'prepare-only']
      : invocation.operation === 'execute' ? [
          '--expected-policy-fingerprint', invocation.expectedPolicyFingerprint,
          '--policy-review-receipt', invocation.policyReviewReceipt,
        ] : [
          '--replay', 'verify-only',
          '--expected-policy-fingerprint', invocation.expectedPolicyFingerprint,
          '--policy-review-receipt', invocation.policyReviewReceipt,
          '--envelope-receipt', invocation.envelopeReceipt,
          '--receipt-path', invocation.receiptPath,
        ];
    const childArgs = [
      '--no-addons', '--disable-proto=throw', '--input-type=module', '-',
      '--repository', invocation.repositoryRoot,
      '--controller-store', store.path,
      '--controller-commit', invocation.controllerCommit,
      '--run-id', invocation.runId,
      '--swarm-id', invocation.swarmId,
      '--coordination-task-id', invocation.coordinationTaskId,
      '--hive-id', 'hierarchical', '--consensus-id', 'raft',
      '--task-path', invocation.taskPath,
      ...operationArgs,
    ];
    const previousUmask = process.umask(0o077);
    let result;
    try {
      result = spawnSync(NODE, childArgs, {
        input: launcher,
        encoding: 'utf8',
        maxBuffer: 20_000_000,
        env: {
          LANG: 'C.UTF-8',
          XDG_RUNTIME_DIR: runtimeRoot,
          DBUS_SESSION_BUS_ADDRESS: `unix:path=${runtimeRoot}/bus`,
        },
      });
    } finally {
      process.umask(previousUmask);
    }
    const acceptedStatus = invocation.operation === 'execute'
      ? result.status === 0 || result.status === 1
      : result.status === 0;
    if (result.error !== undefined || !acceptedStatus || result.stderr !== '') {
      throw new Error(safeChildReason(result));
    }
    if (invocation.operation === 'policy-review') {
      const prepared = parsePolicyReviewReceipt(result.stdout, invocation);
      if (prepared.controllerStoreDigest !== store.digest) {
        throw new Error('HARNESS_OPERATOR_CONTROLLER_STORE_REVIEW_MISMATCH');
      }
      writePrivateReceipt(invocation.repositoryRoot, invocation.receiptPath, result.stdout);
      output = Object.freeze({
        operation: 'policy-review',
        receiptPath: invocation.receiptPath,
        policyFingerprint: prepared.policyFingerprint,
        policyReviewReceiptDigest: prepared.policyReviewReceiptDigest,
      });
    } else if (invocation.operation === 'execute') {
      const parsed = parseProgrammeV6ExecutionSummary(
        result.stdout, invocation.expectedPolicyFingerprint,
      );
      assertProgrammeV6ChildStatus('execute', result.status, parsed);
      output = parsed;
    } else {
      output = parseProgrammeV6ReplaySummary(
        result.stdout, invocation, JSON.parse(receipt.policyBlob).basePolicyFingerprint,
      );
    }
  } catch (error) {
    failure = error;
  }
  try { store.cleanup(); }
  catch (cleanupError) {
    failure = failure === undefined
      ? cleanupError
      : new AggregateError([failure, cleanupError], 'HARNESS_OPERATOR_OPERATION_AND_CLEANUP_FAILED');
  }
  if (failure !== undefined) throw failure;
  return output;
}

export function programmeV6OperatorExitCode(operation, result) {
  return operation === 'execute' && result?.status !== 'pass' ? 1 : 0;
}

export function assertProgrammeV6ChildStatus(operation, childStatus, result) {
  if (childStatus !== programmeV6OperatorExitCode(operation, result)) {
    throw new Error('HARNESS_OPERATOR_CHILD_STATUS_MISMATCH');
  }
}

function parseInvocation(argv) {
  if (!['policy-review', 'execute', 'replay'].includes(argv[0])) {
    throw new Error('HARNESS_OPERATOR_ARGUMENTS_INVALID');
  }
  const operation = argv[0];
  const common = [
    'repository', 'controller-commit', 'run-id', 'swarm-id', 'coordination-task-id',
  ];
  const operationFlags = operation === 'policy-review'
    ? ['receipt-path']
    : operation === 'execute'
      ? ['expected-policy-fingerprint', 'policy-review-receipt']
      : [
          'expected-policy-fingerprint', 'policy-review-receipt',
          'envelope-receipt', 'receipt-path',
        ];
  const allowed = [...common, ...operationFlags, 'task-path'];
  const args = argv.slice(1);
  if (args.length !== allowed.length * 2) throw new Error('HARNESS_OPERATOR_ARGUMENTS_INVALID');
  const values = new Map();
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index]?.startsWith('--') ? args[index].slice(2) : '';
    const value = args[index + 1];
    if (!allowed.includes(name) || values.has(name) || typeof value !== 'string'
      || value.length === 0 || value.includes('\0')) {
      throw new Error('HARNESS_OPERATOR_ARGUMENTS_INVALID');
    }
    values.set(name, value);
  }
  if (allowed.some((name) => !values.has(name))) {
    throw new Error('HARNESS_OPERATOR_ARGUMENTS_INVALID');
  }
  const repositoryRoot = canonicalDirectory(values.get('repository'), 'REPOSITORY_INVALID');
  const controllerCommit = gitObject(values.get('controller-commit'));
  const runId = opaque(values.get('run-id'));
  const taskPath = taskPathValue(values.get('task-path'));
  const base = {
    operation, repositoryRoot, controllerCommit, runId, taskPath,
    swarmId: opaque(values.get('swarm-id')),
    coordinationTaskId: opaque(values.get('coordination-task-id')),
    hiveId: 'hierarchical',
    consensusId: 'raft',
  };
  if (operation === 'policy-review') {
    return Object.freeze({
      ...base,
      receiptPath: receiptPath(repositoryRoot, runId, values.get('receipt-path')),
    });
  }
  const anchored = {
    ...base,
    expectedPolicyFingerprint: digest(values.get('expected-policy-fingerprint')),
    policyReviewReceipt: receiptPath(
      repositoryRoot, runId, values.get('policy-review-receipt'),
    ),
  };
  if (operation === 'execute') return Object.freeze(anchored);
  return Object.freeze({
    ...anchored,
    envelopeReceipt: receiptPath(
      repositoryRoot, runId, values.get('envelope-receipt'), 'execution',
    ),
    receiptPath: receiptPath(repositoryRoot, runId, values.get('receipt-path'), 'replay'),
  });
}

function validateOperatorProcess(environment) {
  const expectedArgs = ['--no-addons', '--disable-proto=throw'];
  const expectedKeys = ['DBUS_SESSION_BUS_ADDRESS', 'LANG', 'XDG_RUNTIME_DIR'];
  if (process.execPath !== NODE || realpathSync(process.execPath) !== NODE
    || JSON.stringify(process.execArgv) !== JSON.stringify(expectedArgs)
    || JSON.stringify(Object.keys(environment).sort()) !== JSON.stringify(expectedKeys)
    || environment.LANG !== 'C.UTF-8'
    || environment.DBUS_SESSION_BUS_ADDRESS !== `unix:path=${environment.XDG_RUNTIME_DIR}/bus`
    || process.umask() !== 0o077) {
    throw new Error('HARNESS_OPERATOR_ENVIRONMENT_INVALID');
  }
  trustedExecutable(NODE, NODE_DIGEST);
  trustedExecutable(GIT, GIT_DIGEST);
}

export function parseProgrammeV6ExecutionSummary(serialized, expectedPolicyFingerprint) {
  let value;
  try { value = JSON.parse(serialized); }
  catch { throw new Error('HARNESS_OPERATOR_EXECUTION_SUMMARY_INVALID'); }
  if (serialized !== `${JSON.stringify(value)}\n`) {
    throw new Error('HARNESS_OPERATOR_EXECUTION_SUMMARY_INVALID');
  }
  exactKeys(value, [
    'status', 'transactionStatus', 'reason', 'receiptDigest', 'candidateTransactionEvidenceDigest',
    'programmeAcceptanceDigest', 'envelopeDigest', 'policyFingerprint',
    'executionClaimDigest', 'launchReceiptDigest',
  ], 'HARNESS_OPERATOR_EXECUTION_SUMMARY_INVALID');
  const status = value.status;
  if (!['pass', 'fail', 'gated', 'cancelled'].includes(status)
    || !['pass', 'fail', 'gated', 'cancelled'].includes(value.transactionStatus)
    || (status === 'pass' ? value.reason !== null : typeof value.reason !== 'string')
    || !validTransactionOutcome(value.transactionStatus, status, value.reason)
    || !validCandidateEvidence(
      value.transactionStatus, value.candidateTransactionEvidenceDigest,
    )
    || value.policyFingerprint !== expectedPolicyFingerprint
    || !['receiptDigest', 'programmeAcceptanceDigest', 'envelopeDigest',
      'policyFingerprint', 'executionClaimDigest', 'launchReceiptDigest']
      .every((key) => validDigest(value[key]))) {
    throw new Error('HARNESS_OPERATOR_EXECUTION_SUMMARY_INVALID');
  }
  return deepFreeze(value);
}

export function parseProgrammeV6ReplaySummary(serialized, invocation, expectedBasePolicyFingerprint) {
  let value;
  try { value = JSON.parse(serialized); }
  catch { throw new Error('HARNESS_OPERATOR_REPLAY_SUMMARY_INVALID'); }
  if (serialized !== `${JSON.stringify(value)}\n`) {
    throw new Error('HARNESS_OPERATOR_REPLAY_SUMMARY_INVALID');
  }
  exactKeys(value, [
    'verificationStatus', 'transactionStatus', 'recordedStatus', 'recordedReason', 'receiptPath',
    'replayReceiptDigest', 'receiptDigest', 'envelopeDigest', 'policyFingerprint',
    'basePolicyFingerprint', 'candidateTransactionEvidenceDigest', 'executionClaimDigest',
    'launchReceiptDigest',
  ], 'HARNESS_OPERATOR_REPLAY_SUMMARY_INVALID');
  if (value.verificationStatus !== 'verified'
    || !['pass', 'fail', 'gated', 'cancelled'].includes(value.transactionStatus)
    || !['pass', 'fail', 'gated', 'cancelled'].includes(value.recordedStatus)
    || (value.recordedStatus === 'pass'
      ? value.recordedReason !== null : typeof value.recordedReason !== 'string')
    || !validTransactionOutcome(
      value.transactionStatus, value.recordedStatus, value.recordedReason,
    )
    || !validCandidateEvidence(
      value.transactionStatus, value.candidateTransactionEvidenceDigest,
    )
    || value.receiptPath !== invocation.receiptPath
    || value.policyFingerprint !== invocation.expectedPolicyFingerprint
    || value.basePolicyFingerprint !== expectedBasePolicyFingerprint
    || !['replayReceiptDigest', 'receiptDigest', 'envelopeDigest', 'policyFingerprint',
      'basePolicyFingerprint', 'executionClaimDigest', 'launchReceiptDigest']
      .every((key) => validDigest(value[key]))
    || value.launchReceiptDigest !== programmeV6ReplayLaunchDigest(value, invocation)) {
    throw new Error('HARNESS_OPERATOR_REPLAY_SUMMARY_INVALID');
  }
  return deepFreeze(value);
}

function validCandidateEvidence(status, value) {
  return status === 'pass' ? validDigest(value) : value === null;
}

function validTransactionOutcome(transactionStatus, outcomeStatus, reason) {
  return transactionStatus === 'pass'
    ? (outcomeStatus === 'pass' && reason === null)
      || (outcomeStatus === 'gated' && reason === 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED')
    : transactionStatus === outcomeStatus;
}

function controllerStoreDigest(root, commit) {
  const files = [];
  const visit = (directory, prefix = '') => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      const relativePath = prefix === '' ? entry.name : `${prefix}/${entry.name}`;
      if (entry.isDirectory()) visit(path, relativePath);
      else if (entry.isFile()) files.push(relativePath);
      else throw new Error('HARNESS_OPERATOR_CONTROLLER_STORE_INVALID');
    }
  };
  visit(root);
  const digests = Object.fromEntries(files.sort().map((path) => [
    path, stablePrivateFileDigest(join(root, path)),
  ]));
  return sha256(JSON.stringify({ commit, files: digests }));
}

function hardenTree(root) {
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      const stat = lstatSync(path);
      if (entry.isDirectory() && !stat.isSymbolicLink()) visit(path);
      else if (entry.isFile() && !stat.isSymbolicLink() && stat.nlink === 1) chmodSync(path, 0o400);
      else throw new Error('HARNESS_OPERATOR_CONTROLLER_STORE_INVALID');
    }
    chmodSync(directory, 0o500);
  };
  visit(root);
}

function cleanupStore(runtimeRoot, path, identity, expectedDigest, controllerCommit) {
  if (!existsSync(path)) return;
  if (dirname(path) !== runtimeRoot
    || !['semantic-fabric-controller-store-', 'semantic-fabric-empty-template-']
      .some((prefix) => basename(path).startsWith(prefix))) {
    throw new Error('HARNESS_OPERATOR_CLEANUP_TARGET_INVALID');
  }
  const current = directoryIdentity(path);
  if (current.dev !== identity.dev || current.ino !== identity.ino) {
    throw new Error('HARNESS_OPERATOR_CLEANUP_TARGET_CHANGED');
  }
  if (expectedDigest !== undefined
    && controllerStoreDigest(path, controllerCommit) !== expectedDigest) {
    throw new Error('HARNESS_OPERATOR_CLEANUP_TARGET_CHANGED');
  }
  makeRemovable(path);
  rmSync(path, { recursive: true, force: true });
}

function directoryIdentity(path) {
  const stat = lstatSync(path, { bigint: true });
  if (!stat.isDirectory() || stat.isSymbolicLink() || stat.uid !== BigInt(process.getuid())
    || realpathSync(path) !== path) {
    throw new Error('HARNESS_OPERATOR_PRIVATE_DIRECTORY_INVALID');
  }
  return Object.freeze({ dev: stat.dev, ino: stat.ino });
}

function makeRemovable(root) {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) makeRemovable(path);
  }
  chmodSync(root, 0o700);
}

function gitChecked(cwd, args, input, maxBuffer = 20_000_000) {
  const result = spawnSync(GIT, args, {
    cwd,
    input,
    encoding: input === undefined ? 'utf8' : null,
    maxBuffer,
    env: gitEnvironment(),
  });
  if (result.error !== undefined || result.status !== 0) {
    throw new Error('HARNESS_OPERATOR_GIT_FAILED');
  }
  return result.stdout;
}

function gitText(cwd, args) {
  const output = gitChecked(cwd, args);
  if (typeof output !== 'string') throw new Error('HARNESS_OPERATOR_GIT_FAILED');
  return output.trim();
}

function gitEnvironment() {
  return {
    PATH: '/usr/bin:/bin', HOME: '/nonexistent', LANG: 'C', LC_ALL: 'C',
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_ATTR_NOSYSTEM: '1', GIT_NO_REPLACE_OBJECTS: '1', GIT_NO_LAZY_FETCH: '1',
    GIT_TERMINAL_PROMPT: '0', GIT_ASKPASS: '/bin/false', GIT_PAGER: 'cat', PAGER: 'cat',
  };
}

function safeChildReason(result) {
  if (typeof result?.stderr === 'string') {
    try {
      const parsed = JSON.parse(result.stderr);
      if (typeof parsed.reason === 'string' && /^HARNESS_[A-Z0-9_]+$/.test(parsed.reason)) {
        return parsed.reason;
      }
    } catch { /* use the generic local-only failure below */ }
  }
  return 'HARNESS_OPERATOR_LAUNCH_FAILED';
}

function safeReason(error) {
  const seen = new Set();
  const visit = (value) => {
    if (!(value instanceof Error) || seen.has(value)) return null;
    seen.add(value);
    if (value instanceof AggregateError) {
      for (const nested of [...value.errors, value.cause]) {
        const reason = visit(nested);
        if (reason !== null) return reason;
      }
    }
    return /^(HARNESS_[A-Z0-9_]+)/.exec(value.message)?.[1] ?? visit(value.cause);
  };
  return visit(error) ?? 'HARNESS_OPERATOR_FAILED';
}

function combineFailure(primary, secondary, message) {
  return primary === undefined ? secondary : new AggregateError([primary, secondary], message);
}

const scriptPath = fileURLToPath(import.meta.url);
if (process.argv[1] !== undefined && realpathSync(process.argv[1]) === scriptPath) {
  try {
    const result = runProgrammeV6Operator(process.argv.slice(2));
    process.stdout.write(`${JSON.stringify(result)}\n`);
    process.exitCode = programmeV6OperatorExitCode(process.argv[2], result);
  } catch (error) {
    process.stderr.write(`${JSON.stringify({ status: 'error', reason: safeReason(error) })}\n`);
    process.exitCode = 1;
  }
}
