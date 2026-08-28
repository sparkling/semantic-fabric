// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  closeSync, constants, fstatSync, fsyncSync, lstatSync, mkdirSync, openSync,
  readSync, realpathSync, writeSync,
} from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';

export const DIGEST = /^[a-f0-9]{64}$/;
export const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
export const OPAQUE_ID = /^[A-Za-z0-9_-]{8,160}$/;
const MAX_POLICY_BYTES = 5_000_000;
const MAX_POLICY_NODES = 100_000;
const MAX_POLICY_DEPTH = 128;
const RECEIPT_KEYS = Object.freeze([
  'schemaVersion', 'authority', 'operation', 'controllerCommit', 'taskPath', 'runId',
  'swarmId', 'coordinationTaskId', 'hiveId', 'consensusId',
  'controllerStoreDigest', 'buildManifestDigest', 'runtimeTreeDigest', 'nodeDigest',
  'gitDigest', 'policyFingerprint', 'policyBlob', 'policyReviewReceiptDigest',
]);

export function parsePolicyReviewReceipt(serialized, expected) {
  if (typeof serialized !== 'string' || Buffer.byteLength(serialized, 'utf8') > 6_000_000) {
    throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  }
  let value;
  try { value = JSON.parse(serialized); }
  catch { throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID'); }
  if (serialized !== `${JSON.stringify(value)}\n`) {
    throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  }
  exactKeys(value, RECEIPT_KEYS, 'HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  const { policyReviewReceiptDigest, ...body } = value;
  const policy = canonicalPolicy(value.policyBlob);
  if (value.schemaVersion !== 1 || value.authority !== 'development-only-no-promotion'
    || value.operation !== 'programme-v6-policy-review'
    || value.controllerCommit !== expected.controllerCommit
    || value.taskPath !== expected.taskPath || value.runId !== expected.runId
    || value.swarmId !== expected.swarmId
    || value.coordinationTaskId !== expected.coordinationTaskId
    || value.hiveId !== expected.hiveId || value.consensusId !== expected.consensusId
    || !validDigest(value.controllerStoreDigest) || !validDigest(value.buildManifestDigest)
    || !validDigest(value.runtimeTreeDigest) || !validDigest(value.nodeDigest)
    || !validDigest(value.gitDigest) || !validDigest(value.policyFingerprint)
    || sha256(value.policyBlob) !== value.policyFingerprint
    || !validDigest(policyReviewReceiptDigest)
    || sha256(canonicalJson(body)) !== policyReviewReceiptDigest
    || !policyReceiptBindingsMatch(value, policy)) {
    throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  }
  return deepFreeze(value);
}

export function stableReadPrivateReceipt(repositoryRoot, path, maximumBytes) {
  const runs = privateRunsDirectory(repositoryRoot, false);
  if (dirname(path) !== runs) throw new Error('HARNESS_OPERATOR_RECEIPT_PATH_INVALID');
  const directory = openSync(runs,
    constants.O_RDONLY | (constants.O_DIRECTORY ?? 0) | (constants.O_NOFOLLOW ?? 0));
  try {
    const directoryStat = fstatSync(directory, { bigint: true });
    if (!sameDirectory(directoryStat, lstatSync(runs, { bigint: true }))) {
      throw new Error('HARNESS_OPERATOR_RECEIPT_ROOT_CHANGED');
    }
    const anchoredPath = `/proc/self/fd/${directory}/${basename(path)}`;
    const pathStat = trustedReceiptStat(anchoredPath, maximumBytes);
    const descriptor = openSync(anchoredPath, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    try {
      const before = fstatSync(descriptor, { bigint: true });
      if (!sameFile(pathStat, before)) throw new Error('HARNESS_OPERATOR_RECEIPT_CHANGED');
      const buffer = Buffer.allocUnsafe(Number(before.size));
      let offset = 0;
      while (offset < buffer.length) {
        const count = readSync(descriptor, buffer, offset, buffer.length - offset, offset);
        if (count === 0) throw new Error('HARNESS_OPERATOR_RECEIPT_CHANGED');
        offset += count;
      }
      const after = fstatSync(descriptor, { bigint: true });
      const finalPathStat = lstatSync(anchoredPath, { bigint: true });
      if (!sameFile(before, after) || !sameFile(after, finalPathStat)
        || !sameDirectory(directoryStat, fstatSync(directory, { bigint: true }))
        || !sameDirectory(directoryStat, lstatSync(runs, { bigint: true }))) {
        throw new Error('HARNESS_OPERATOR_RECEIPT_CHANGED');
      }
      try { return new TextDecoder('utf-8', { fatal: true }).decode(buffer); }
      catch { throw new Error('HARNESS_OPERATOR_RECEIPT_INVALID'); }
    } finally { closeSync(descriptor); }
  } finally { closeSync(directory); }
}

export function writePrivateReceipt(repositoryRoot, path, contents) {
  const runs = privateRunsDirectory(repositoryRoot, true);
  if (dirname(path) !== runs) throw new Error('HARNESS_OPERATOR_RECEIPT_PATH_INVALID');
  const bytes = Buffer.from(contents, 'utf8');
  const directory = openSync(runs,
    constants.O_RDONLY | (constants.O_DIRECTORY ?? 0) | (constants.O_NOFOLLOW ?? 0));
  try {
    const directoryStat = fstatSync(directory, { bigint: true });
    if (!sameDirectory(directoryStat, lstatSync(runs, { bigint: true }))) {
      throw new Error('HARNESS_OPERATOR_RECEIPT_ROOT_CHANGED');
    }
    const anchoredPath = `/proc/self/fd/${directory}/${basename(path)}`;
    assertPathAbsent(anchoredPath, 'HARNESS_OPERATOR_POLICY_RECEIPT_EXISTS');
    const descriptor = openSync(anchoredPath,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | (constants.O_NOFOLLOW ?? 0),
      0o600);
    try {
      let offset = 0;
      while (offset < bytes.length) {
        offset += writeSync(descriptor, bytes, offset, bytes.length - offset, offset);
      }
      fsyncSync(descriptor);
    } finally { closeSync(descriptor); }
    fsyncSync(directory);
    if (!sameDirectory(directoryStat, fstatSync(directory, { bigint: true }))
      || !sameDirectory(directoryStat, lstatSync(runs, { bigint: true }))) {
      throw new Error('HARNESS_OPERATOR_RECEIPT_ROOT_CHANGED');
    }
  }
  finally { closeSync(directory); }
  if (stableReadPrivateReceipt(repositoryRoot, path, bytes.length) !== contents) {
    throw new Error('HARNESS_OPERATOR_RECEIPT_CHANGED');
  }
}

export function assertPathAbsent(path, error) {
  try { lstatSync(path); }
  catch (caught) {
    if (caught?.code === 'ENOENT') return;
    throw caught;
  }
  throw new Error(error);
}

export function receiptPath(repositoryRoot, runId, value, suffix = 'policy-review') {
  if (typeof value !== 'string' || !isAbsolute(value) || resolve(value) !== value) {
    throw new Error('HARNESS_OPERATOR_RECEIPT_PATH_INVALID');
  }
  const parent = join(repositoryRoot, 'coding-harness', '.metaharness', 'runs');
  const name = suffix === 'execution' ? `${runId}.json` : `${runId}.${suffix}.json`;
  if (dirname(value) !== parent || basename(value) !== name) {
    throw new Error('HARNESS_OPERATOR_RECEIPT_PATH_INVALID');
  }
  return value;
}

export function canonicalJson(value) {
  if (Array.isArray(value)) return JSON.stringify(value.map(canonicalValue));
  return JSON.stringify(canonicalValue(value));
}

export function programmeV6ReplayLaunchDigest(value, invocation) {
  return sha256(canonicalJson({
    schemaVersion: 1,
    domain: 'semantic-fabric/programme-v6/replay-launch/v1',
    operation: 'programme-v6-replay-launch',
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    outerPolicyFingerprint: value.policyFingerprint,
    basePolicyFingerprint: value.basePolicyFingerprint,
    envelopeDigest: value.envelopeDigest,
    transactionStatus: value.transactionStatus,
    receiptDigest: value.receiptDigest,
    candidateTransactionEvidenceDigest: value.candidateTransactionEvidenceDigest,
    executionClaimDigest: value.executionClaimDigest,
  }));
}

export function deepFreeze(value) {
  if (value !== null && typeof value === 'object') {
    for (const entry of Object.values(value)) deepFreeze(entry);
    Object.freeze(value);
  }
  return value;
}

export function exactKeys(value, keys, error) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)
    || JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())) {
    throw new Error(error);
  }
}

export function canonicalDirectory(value, label) {
  if (typeof value !== 'string' || !isAbsolute(value) || resolve(value) !== value
    || value.includes('\0')) throw new Error(`HARNESS_OPERATOR_${label}`);
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error(`HARNESS_OPERATOR_${label}`);
  }
  return value;
}

export function privateDirectory(value, label) {
  const path = canonicalDirectory(value, label);
  const stat = lstatSync(path);
  if (stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0) {
    throw new Error(`HARNESS_OPERATOR_${label}`);
  }
  return path;
}

export function taskPathValue(value) {
  if (!/^coding-harness\/config\/[a-z0-9]+(?:[-_][a-z0-9]+)*-acceptance\.json$/.test(value)) {
    throw new Error('HARNESS_OPERATOR_TASK_PATH_INVALID');
  }
  return value;
}

export function gitObject(value) {
  if (typeof value !== 'string' || !GIT_OBJECT.test(value)) {
    throw new Error('HARNESS_OPERATOR_CONTROLLER_COMMIT_INVALID');
  }
  return value;
}

export function opaque(value) {
  if (typeof value !== 'string' || !OPAQUE_ID.test(value)) {
    throw new Error('HARNESS_OPERATOR_IDENTIFIER_INVALID');
  }
  return value;
}

export function validDigest(value) {
  return typeof value === 'string' && DIGEST.test(value) && value !== '0'.repeat(64);
}

export function digest(value) {
  if (!validDigest(value)) throw new Error('HARNESS_OPERATOR_DIGEST_INVALID');
  return value;
}

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

export function trustedExecutable(path, expectedDigest) {
  if (typeof path !== 'string' || !isAbsolute(path) || resolve(path) !== path) {
    throw new Error('HARNESS_OPERATOR_EXECUTABLE_INVALID');
  }
  const pathStat = lstatSync(path, { bigint: true });
  if (!pathStat.isFile() || pathStat.isSymbolicLink() || pathStat.uid !== 0n
    || pathStat.nlink !== 1n || (pathStat.mode & 0o111n) === 0n
    || (pathStat.mode & 0o022n) !== 0n || realpathSync(path) !== path) {
    throw new Error('HARNESS_OPERATOR_EXECUTABLE_UNTRUSTED');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    if (before.size < 1n || before.size > 500_000_000n) {
      throw new Error('HARNESS_OPERATOR_EXECUTABLE_UNTRUSTED');
    }
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < before.size) {
      const count = readSync(
        descriptor, buffer, 0, Math.min(buffer.length, Number(before.size - offset)), Number(offset),
      );
      if (count === 0) throw new Error('HARNESS_OPERATOR_EXECUTABLE_CHANGED');
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameFile(pathStat, before) || !sameFile(before, after)
      || !sameFile(after, lstatSync(path, { bigint: true })) || realpathSync(path) !== path) {
      throw new Error('HARNESS_OPERATOR_EXECUTABLE_CHANGED');
    }
    const actual = hash.digest('hex');
    if (actual !== expectedDigest) throw new Error('HARNESS_OPERATOR_EXECUTABLE_MISMATCH');
    return actual;
  } finally { closeSync(descriptor); }
}

export function stablePrivateFileDigest(path, maximumBytes = 10_000_000_000) {
  const pathStat = lstatSync(path, { bigint: true });
  if (!pathStat.isFile() || pathStat.isSymbolicLink() || pathStat.nlink !== 1n
    || pathStat.uid !== BigInt(process.getuid()) || (pathStat.mode & 0o022n) !== 0n
    || pathStat.size < 1n || pathStat.size > BigInt(maximumBytes)) {
    throw new Error('HARNESS_OPERATOR_CONTROLLER_STORE_INVALID');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    if (!sameFile(pathStat, before)) throw new Error('HARNESS_OPERATOR_CONTROLLER_STORE_CHANGED');
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < before.size) {
      const count = readSync(
        descriptor, buffer, 0, Math.min(buffer.length, Number(before.size - offset)), Number(offset),
      );
      if (count === 0) throw new Error('HARNESS_OPERATOR_CONTROLLER_STORE_CHANGED');
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameFile(before, after)
      || !sameFile(after, lstatSync(path, { bigint: true }))) {
      throw new Error('HARNESS_OPERATOR_CONTROLLER_STORE_CHANGED');
    }
    return hash.digest('hex');
  } finally { closeSync(descriptor); }
}

function privateRunsDirectory(repositoryRoot, create) {
  const repository = canonicalDirectory(repositoryRoot, 'REPOSITORY_INVALID');
  const harness = canonicalDirectory(join(repository, 'coding-harness'), 'RECEIPT_ROOT_INVALID');
  const root = ensurePrivateChild(harness, '.metaharness', create);
  return ensurePrivateChild(root, 'runs', create);
}

function ensurePrivateChild(parent, name, create) {
  const path = join(parent, name);
  try { lstatSync(path); }
  catch (caught) {
    if (caught?.code !== 'ENOENT' || !create) throw caught;
    mkdirSync(path, { mode: 0o700 });
  }
  const result = privateDirectory(path, 'RECEIPT_ROOT_INVALID');
  if (create) {
    fsyncDirectory(parent);
    fsyncDirectory(result);
  }
  return result;
}

function fsyncDirectory(path) {
  const descriptor = openSync(path,
    constants.O_RDONLY | (constants.O_DIRECTORY ?? 0) | (constants.O_NOFOLLOW ?? 0));
  try { fsyncSync(descriptor); }
  finally { closeSync(descriptor); }
}

function trustedReceiptStat(path, maximumBytes) {
  const stat = lstatSync(path, { bigint: true });
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1n
    || stat.uid !== BigInt(process.getuid()) || (stat.mode & 0o777n) !== 0o600n
    || stat.size < 1n || stat.size > BigInt(maximumBytes)) {
    throw new Error('HARNESS_OPERATOR_RECEIPT_INVALID');
  }
  return stat;
}

function policyReceiptBindingsMatch(receipt, policy) {
  const outerKeys = [
    'schemaVersion', 'policyId', 'authority', 'basePolicy',
    'basePolicyFingerprint', 'gateContract',
  ];
  const gateKeys = [
    'schemaVersion', 'contractId', 'authority', 'authoritativeReplaySemantics',
    'baseGateContract', 'baseGateContractDigest', 'attempts', 'nativeEvidence',
    'envelope', 'evaluation',
  ];
  const envelopeKeys = [
    'schemaVersion', 'policyFingerprintBinding', 'candidateEvidenceDigestBinding',
    'baseReceiptAndDiagnosticSemantics',
  ];
  const base = policy?.basePolicy;
  const gate = policy?.gateContract;
  const bootstrap = base?.bootstrap;
  const controller = base?.controller;
  return JSON.stringify(Object.keys(policy).sort()) === JSON.stringify(outerKeys.sort())
    && policy.schemaVersion === 2
    && policy.policyId === 'semantic-fabric-programme-v6-policy-v2'
    && policy.authority === 'development-only-no-promotion'
    && base !== null && typeof base === 'object' && !Array.isArray(base)
    && base.schemaVersion === 1
    && base.policyId === 'semantic-fabric-programme-v5-policy-v1'
    && base.authority === 'development-only-no-promotion'
    && validDigest(policy.basePolicyFingerprint)
    && sha256(canonicalJson(base)) === policy.basePolicyFingerprint
    && gate !== null && typeof gate === 'object' && !Array.isArray(gate)
    && JSON.stringify(Object.keys(gate).sort()) === JSON.stringify(gateKeys.sort())
    && gate.schemaVersion === 2
    && gate.contractId === 'semantic-fabric-programme-gate-contract-v2'
    && gate.authority === 'development-only-no-promotion'
    && gate.authoritativeReplaySemantics === true
    && validDigest(gate.baseGateContractDigest)
    && sha256(canonicalJson(gate.baseGateContract)) === gate.baseGateContractDigest
    && canonicalJson(gate.baseGateContract) === canonicalJson(base.gateContract)
    && gate.envelope !== null && typeof gate.envelope === 'object'
    && !Array.isArray(gate.envelope)
    && JSON.stringify(Object.keys(gate.envelope).sort())
      === JSON.stringify(envelopeKeys.sort())
    && gate.envelope.schemaVersion === 6
    && gate.envelope.policyFingerprintBinding === 'externally-supplied-v2-anchor'
    && gate.envelope.candidateEvidenceDigestBinding
      === 'envelope.candidateTransactionEvidenceDigest-equals-candidateTransactionEvidence.evidenceDigest'
    && gate.envelope.baseReceiptAndDiagnosticSemantics
      === 'programme-gate-contract-v1-unchanged'
    && bootstrap?.controllerStoreDigest === receipt.controllerStoreDigest
    && bootstrap?.nodeDigest === receipt.nodeDigest
    && bootstrap?.gitDigest === receipt.gitDigest
    && controller?.identity?.commit === receipt.controllerCommit
    && controller?.taskPath === receipt.taskPath
    && controller?.buildManifestBlobDigest === receipt.buildManifestDigest
    && controller?.runtimeTreeDigest === receipt.runtimeTreeDigest
    && policyRunIdMatches(base, receipt.runId);
}

function policyRunIdMatches(policy, runId) {
  try {
    const route = JSON.parse(policy.execution.routeSnapshotBlob);
    const decisions = route?.decisions;
    return decisions !== null && typeof decisions === 'object'
      && ['architecture', 'implementation', 'repair'].every((step) =>
        decisions[step]?.stepKind === step && decisions[step]?.runId === runId);
  } catch { return false; }
}

function canonicalPolicy(value) {
  if (typeof value !== 'string' || Buffer.byteLength(value, 'utf8') < 2
    || Buffer.byteLength(value, 'utf8') > MAX_POLICY_BYTES || value.includes('\0')) {
    throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  }
  let parsed;
  try { parsed = JSON.parse(value); }
  catch { throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID'); }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)
    || JSON.stringify(canonicalPolicyValue(parsed)) !== value) {
    throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  }
  return parsed;
}

function canonicalPolicyValue(value, depth = 0, state = { nodes: 0 }) {
  state.nodes += 1;
  if (state.nodes > MAX_POLICY_NODES || depth > MAX_POLICY_DEPTH) {
    throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  }
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (Array.isArray(value)) return value.map((entry) => canonicalPolicyValue(entry, depth + 1, state));
  if (typeof value !== 'object') throw new Error('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  return Object.fromEntries(Object.keys(value).sort()
    .map((key) => [key, canonicalPolicyValue(value[key], depth + 1, state)]));
}

function canonicalValue(value) {
  if (Array.isArray(value)) return value.map(canonicalValue);
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort()
      .map((key) => [key, canonicalValue(value[key])]));
  }
  return value;
}

function sameFile(left, right) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.uid === right.uid && left.nlink === right.nlink && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function sameDirectory(left, right) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.uid === right.uid && left.nlink === right.nlink;
}
