// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { isProxy } from 'node:util/types';
import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  assertExactKeys,
  snapshotUint8Array,
} from './contracts.js';

const MAX_LEAF_BYTES = 131_072;
const MAX_PROOF_NODES = 65;
const MAX_UINT64 = 18_446_744_073_709_551_615n;
const UINT64_DECIMAL_PATTERN = /^(?:0|[1-9][0-9]{0,19})$/;
// The application profile reserves the all-zero value as an invalid sentinel.
const ZERO_DIGEST = '0'.repeat(64);
const EMPTY_ROOT = sha256(new Uint8Array());

export function programmeCaptureSupervisorMerkleLeafHashV1(value: unknown): string {
  const bytes = snapshotUint8Array(value, 'supervisor Merkle leaf bytes', MAX_LEAF_BYTES);
  return hashLeaf(bytes);
}

export function verifyProgrammeCaptureSupervisorMerkleInclusionProofV1(value: unknown): void {
  if (isProxy(value)) throw new TypeError('supervisor Merkle inclusion proof must not be a Proxy');
  const input = asClosedRecord(value, 'supervisor Merkle inclusion proof');
  assertExactKeys(
    input,
    ['leafBytes', 'leafIndex', 'treeSize', 'rootDigest', 'proofDigests'],
    'supervisor Merkle inclusion proof',
  );
  const leafBytes = snapshotUint8Array(
    input.leafBytes, 'supervisor Merkle inclusion leaf bytes', MAX_LEAF_BYTES,
  );
  let leafIndex = parseUint64(input.leafIndex, 'supervisor Merkle inclusion leaf index');
  const treeSize = parseUint64(input.treeSize, 'supervisor Merkle inclusion tree size');
  const rootDigest = parseDigest(input.rootDigest, 'supervisor Merkle inclusion root');
  const proof = parseProof(input.proofDigests, 'supervisor Merkle inclusion path');
  if (treeSize === 0n || leafIndex >= treeSize) {
    throw new RangeError('supervisor Merkle inclusion leaf index is outside the tree');
  }

  let lastNode = treeSize - 1n;
  let calculatedRoot = hashLeaf(leafBytes);
  for (const sibling of proof) {
    if (lastNode === 0n) throw new Error('HARNESS_SUPERVISOR_MERKLE_INCLUSION_PATH_SURPLUS');
    if ((leafIndex & 1n) === 1n || leafIndex === lastNode) {
      calculatedRoot = hashNode(sibling, calculatedRoot);
      if ((leafIndex & 1n) === 0n) {
        while (leafIndex !== 0n && (leafIndex & 1n) === 0n) {
          leafIndex >>= 1n;
          lastNode >>= 1n;
        }
      }
    } else {
      calculatedRoot = hashNode(calculatedRoot, sibling);
    }
    leafIndex >>= 1n;
    lastNode >>= 1n;
  }
  if (lastNode !== 0n || calculatedRoot !== rootDigest) {
    throw new Error('HARNESS_SUPERVISOR_MERKLE_INCLUSION_INVALID');
  }
}

export function verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1(value: unknown): void {
  if (isProxy(value)) throw new TypeError('supervisor Merkle consistency proof must not be a Proxy');
  const input = asClosedRecord(value, 'supervisor Merkle consistency proof');
  assertExactKeys(
    input,
    ['oldTreeSize', 'newTreeSize', 'oldRootDigest', 'newRootDigest', 'proofDigests'],
    'supervisor Merkle consistency proof',
  );
  const oldTreeSize = parseUint64(input.oldTreeSize, 'supervisor Merkle old tree size');
  const newTreeSize = parseUint64(input.newTreeSize, 'supervisor Merkle new tree size');
  const oldRootDigest = parseDigest(input.oldRootDigest, 'supervisor Merkle old root');
  const newRootDigest = parseDigest(input.newRootDigest, 'supervisor Merkle new root');
  const suppliedProof = parseProof(input.proofDigests, 'supervisor Merkle consistency path');
  if (oldTreeSize > newTreeSize) {
    throw new RangeError('supervisor Merkle old tree size exceeds the new tree size');
  }
  if (oldTreeSize === 0n) {
    if (oldRootDigest !== EMPTY_ROOT || suppliedProof.length !== 0
      || (newTreeSize === 0n && newRootDigest !== EMPTY_ROOT)) {
      throw new Error('HARNESS_SUPERVISOR_MERKLE_EMPTY_CONSISTENCY_INVALID');
    }
    return;
  }
  if (oldTreeSize === newTreeSize) {
    if (suppliedProof.length !== 0 || oldRootDigest !== newRootDigest) {
      throw new Error('HARNESS_SUPERVISOR_MERKLE_EQUAL_CONSISTENCY_INVALID');
    }
    return;
  }
  if (suppliedProof.length === 0) {
    throw new Error('HARNESS_SUPERVISOR_MERKLE_CONSISTENCY_PATH_MISSING');
  }

  let firstNode = oldTreeSize - 1n;
  let lastNode = newTreeSize - 1n;
  while ((firstNode & 1n) === 1n) {
    firstNode >>= 1n;
    lastNode >>= 1n;
  }
  const proof = isPowerOfTwo(oldTreeSize)
    ? [oldRootDigest, ...suppliedProof]
    : suppliedProof;
  let oldCalculatedRoot = proof[0];
  let newCalculatedRoot = proof[0];
  for (const sibling of proof.slice(1)) {
    if (lastNode === 0n) throw new Error('HARNESS_SUPERVISOR_MERKLE_CONSISTENCY_PATH_SURPLUS');
    if ((firstNode & 1n) === 1n || firstNode === lastNode) {
      oldCalculatedRoot = hashNode(sibling, oldCalculatedRoot);
      newCalculatedRoot = hashNode(sibling, newCalculatedRoot);
      if ((firstNode & 1n) === 0n) {
        while (firstNode !== 0n && (firstNode & 1n) === 0n) {
          firstNode >>= 1n;
          lastNode >>= 1n;
        }
      }
    } else {
      newCalculatedRoot = hashNode(newCalculatedRoot, sibling);
    }
    firstNode >>= 1n;
    lastNode >>= 1n;
  }
  if (lastNode !== 0n || oldCalculatedRoot !== oldRootDigest
    || newCalculatedRoot !== newRootDigest) {
    throw new Error('HARNESS_SUPERVISOR_MERKLE_CONSISTENCY_INVALID');
  }
}

function parseUint64(value: unknown, label: string): bigint {
  if (typeof value !== 'string' || !UINT64_DECIMAL_PATTERN.test(value)) {
    throw new TypeError(`${label} must be a canonical uint64 decimal string`);
  }
  const parsed = BigInt(value);
  if (parsed > MAX_UINT64) throw new RangeError(`${label} exceeds uint64`);
  return parsed;
}

function parseProof(value: unknown, label: string): string[] {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  if (Array.isArray(value) && value.length > MAX_PROOF_NODES) {
    throw new RangeError(`${label} exceeds its node bound`);
  }
  const entries = asDenseArray(value, label);
  const parsed: string[] = [];
  for (let index = 0; index < entries.length; index += 1) {
    const descriptor = Object.getOwnPropertyDescriptor(entries, index);
    parsed.push(parseDigest(descriptor?.value, `${label}[${index}]`));
  }
  return parsed;
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || value === ZERO_DIGEST) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

function isPowerOfTwo(value: bigint): boolean {
  return value > 0n && (value & (value - 1n)) === 0n;
}

function hashLeaf(value: Uint8Array): string {
  return sha256(Buffer.concat([Buffer.from([0]), Buffer.from(value)]));
}

function hashNode(leftDigest: string, rightDigest: string): string {
  return sha256(Buffer.concat([
    Buffer.from([1]), Buffer.from(leftDigest, 'hex'), Buffer.from(rightDigest, 'hex'),
  ]));
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
