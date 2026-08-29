// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  programmeCaptureSupervisorMerkleLeafHashV1,
  verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1,
  verifyProgrammeCaptureSupervisorMerkleInclusionProofV1,
} from '../src/programme-capture-supervisor-merkle-v1.js';

const EMPTY_ROOT = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';
const ROOT_7 = '73a590fb266b81557040b146b9d479e2a1b5849b125167642f5b64866f1d5c7d';
const leaves = Array.from({ length: 32 }, (_, index) => Buffer.from(`d${index}`, 'utf8'));

describe('programme capture supervisor RFC 9162 Merkle proof verifier V1', () => {
  it('matches fixed seven-leaf inclusion and consistency vectors', () => {
    expect(programmeCaptureSupervisorMerkleLeafHashV1(leaves[0]))
      .toBe('c67f9ffe68e0761021341dd516428f42fbdea633731cbdada03bea6b84c652f7');
    const inclusions = [
      [0, [
        '49b717e4d6ecdd82f6f6648cf8f86fdf4a912600a4557398e1733186fa952c1d',
        'c59e9a6d9575777ba3bdbd3e3086516196cf87ec9760861362aba5cd0f78df1d',
        '3cf05ff16d26c024828e93b3a14c5656e5abcbc5e6f0bce2cf8a169720599674',
      ]],
      [3, [
        'f366df4718ef75064317794ff5300e0963e96dd93fe24203118055fa5a00be13',
        '46c78708413a23175f51faf1c22604bccb44482d553b45943b189130ea8221c8',
        '3cf05ff16d26c024828e93b3a14c5656e5abcbc5e6f0bce2cf8a169720599674',
      ]],
      [4, [
        '6d1bb6bbb111af4a1e9ec0b9fb2613cc2bcb394141cee8c2cd462b5ad3803d78',
        'd750ca922fabc5422eec469d4370779b61d5488186cb871eeea299d8113d20bc',
        '8df3870b33fae650e81938994f98eb4551b143b86c95d3dae4e6444e00715016',
      ]],
      [6, [
        'a4f2a847cce0dce0519b1d6b83e4ca15166193dbb0c8f864e736665edbde1994',
        '8df3870b33fae650e81938994f98eb4551b143b86c95d3dae4e6444e00715016',
      ]],
    ] as const;
    for (const [leafIndex, proofDigests] of inclusions) {
      expect(() => verifyProgrammeCaptureSupervisorMerkleInclusionProofV1({
        leafBytes: leaves[leafIndex], leafIndex: String(leafIndex), treeSize: '7',
        rootDigest: ROOT_7, proofDigests,
      })).not.toThrow();
    }
    const consistencies = [
      ['3', 'c64c5b9326951a2db82d5462565696286659d1c7a4a26a92703568f63462f7ba', [
        'f366df4718ef75064317794ff5300e0963e96dd93fe24203118055fa5a00be13',
        '5e0c4e1130dfa84d27437ba073eb817e1896643d42ea100a0940f8752d496783',
        '46c78708413a23175f51faf1c22604bccb44482d553b45943b189130ea8221c8',
        '3cf05ff16d26c024828e93b3a14c5656e5abcbc5e6f0bce2cf8a169720599674',
      ]],
      ['4', '8df3870b33fae650e81938994f98eb4551b143b86c95d3dae4e6444e00715016', [
        '3cf05ff16d26c024828e93b3a14c5656e5abcbc5e6f0bce2cf8a169720599674',
      ]],
      ['6', 'b65368cd1f024732c21e9db86bcde27d7de95dc2c40d728dd979ffcf943556e3', [
        'a4f2a847cce0dce0519b1d6b83e4ca15166193dbb0c8f864e736665edbde1994',
        'd750ca922fabc5422eec469d4370779b61d5488186cb871eeea299d8113d20bc',
        '8df3870b33fae650e81938994f98eb4551b143b86c95d3dae4e6444e00715016',
      ]],
    ] as const;
    for (const [oldTreeSize, oldRootDigest, proofDigests] of consistencies) {
      expect(() => verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1({
        oldTreeSize, newTreeSize: '7', oldRootDigest, newRootDigest: ROOT_7, proofDigests,
      })).not.toThrow();
    }
  });

  it('verifies every independently generated inclusion proof through 32 leaves', () => {
    for (let treeSize = 1; treeSize <= leaves.length; treeSize += 1) {
      const tree = leaves.slice(0, treeSize);
      for (let leafIndex = 0; leafIndex < treeSize; leafIndex += 1) {
        expect(() => verifyProgrammeCaptureSupervisorMerkleInclusionProofV1({
          leafBytes: tree[leafIndex], leafIndex: String(leafIndex), treeSize: String(treeSize),
          rootDigest: treeHash(tree).toString('hex'),
          proofDigests: inclusionProof(leafIndex, tree).map(hex),
        })).not.toThrow();
      }
    }
  });

  it('verifies empty, equal, power-of-two, and unbalanced consistency cases', () => {
    for (let newSize = 0; newSize <= leaves.length; newSize += 1) {
      const newTree = leaves.slice(0, newSize), newRootDigest = treeHash(newTree).toString('hex');
      for (let oldSize = 0; oldSize <= newSize; oldSize += 1) {
        const oldRootDigest = treeHash(leaves.slice(0, oldSize)).toString('hex');
        const proofDigests = oldSize === 0 || oldSize === newSize
          ? [] : consistencyProof(oldSize, newTree).map(hex);
        expect(() => verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1({
          oldTreeSize: String(oldSize), newTreeSize: String(newSize),
          oldRootDigest, newRootDigest, proofDigests,
        })).not.toThrow();
      }
    }
  });

  it('verifies the maximum uint64 tree size with an exact 65-node proof', () => {
    const oldTreeSize = 9_223_372_036_854_775_807n;
    const newTreeSize = 18_446_744_073_709_551_615n;
    const proofDigests = uniformConsistencyProof(oldTreeSize, newTreeSize).map(hex);
    expect(proofDigests).toHaveLength(65);
    expect(uniformTreeHash(oldTreeSize).toString('hex'))
      .toBe('d36b5152a1e540114ef20d65ff395490cf17de30c78cea4d2e2bd48cce435441');
    expect(uniformTreeHash(newTreeSize).toString('hex'))
      .toBe('0fbf5c96327224045606b94a4e41e182504151bc2c9f43488d3c8e8d8a413dcd');
    expect(hash(Buffer.from(JSON.stringify(proofDigests), 'utf8')).toString('hex'))
      .toBe('eaa7a5e18c7950a55340dac62a2e2a092bf392bd7bfc3d0754185fd599badd33');
    expect(() => verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1({
      oldTreeSize: String(oldTreeSize), newTreeSize: String(newTreeSize),
      oldRootDigest: uniformTreeHash(oldTreeSize).toString('hex'),
      newRootDigest: uniformTreeHash(newTreeSize).toString('hex'), proofDigests,
    })).not.toThrow();
  });

  it('rejects malformed bounds, indices, roots, paths, and loose keys', () => {
    const valid = {
      leafBytes: leaves[3], leafIndex: '3', treeSize: '7', rootDigest: ROOT_7,
      proofDigests: inclusionProof(3, leaves.slice(0, 7)).map(hex),
    };
    const invalid = [
      { ...valid, leafIndex: '7' }, { ...valid, leafIndex: '03' },
      { ...valid, treeSize: 7 }, { ...valid, treeSize: '18446744073709551616' },
      { ...valid, rootDigest: '0'.repeat(64) }, { ...valid, extra: true },
      { ...valid, proofDigests: valid.proofDigests.slice(1) },
      { ...valid, proofDigests: [...valid.proofDigests, '1'.repeat(64)] },
      { ...valid, proofDigests: [...valid.proofDigests].reverse() },
      { ...valid, proofDigests: new Array(1) },
      { ...valid, proofDigests: new Array(66).fill('1'.repeat(64)) },
    ];
    for (const value of invalid) {
      expect(() => verifyProgrammeCaptureSupervisorMerkleInclusionProofV1(value as any))
        .toThrow();
    }
  });

  it('rejects proof-array proxies before invoking any attacker-controlled traps', () => {
    let lengthReads = 0;
    let trapCalls = 0;
    const proofDigests = new Proxy([], {
      get(target, property, receiver) {
        trapCalls += 1;
        if (property === 'length') {
          lengthReads += 1;
          return lengthReads < 4 ? 0 : 1_000;
        }
        if (typeof property === 'string' && /^[0-9]+$/.test(property)) {
          return '1'.repeat(64);
        }
        return Reflect.get(target, property, receiver);
      },
      has(target, property) {
        trapCalls += 1;
        return typeof property === 'string' && /^[0-9]+$/.test(property)
          ? true : Reflect.has(target, property);
      },
    });
    expect(() => verifyProgrammeCaptureSupervisorMerkleInclusionProofV1({
      leafBytes: new Uint8Array([1]), leafIndex: '0', treeSize: '1',
      rootDigest: leafHash(new Uint8Array([1])).toString('hex'), proofDigests,
    })).toThrow();
    expect(trapCalls).toBe(0);
  });

  it('rejects outer-record proxies before invoking any attacker-controlled traps', () => {
    const cases: Array<readonly [Record<string, unknown>, (value: unknown) => void]> = [
      [{
        leafBytes: new Uint8Array([1]), leafIndex: '0', treeSize: '1',
        rootDigest: leafHash(new Uint8Array([1])).toString('hex'), proofDigests: [],
      }, verifyProgrammeCaptureSupervisorMerkleInclusionProofV1],
      [{
        oldTreeSize: '0', newTreeSize: '0', oldRootDigest: EMPTY_ROOT,
        newRootDigest: EMPTY_ROOT, proofDigests: [],
      }, verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1],
    ];
    for (const [target, verify] of cases) {
      let trapCalls = 0;
      const proxy = new Proxy(target, {
        getPrototypeOf(value) { trapCalls += 1; return Reflect.getPrototypeOf(value); },
        ownKeys(value) { trapCalls += 1; return Reflect.ownKeys(value); },
        getOwnPropertyDescriptor(value, property) {
          trapCalls += 1;
          return Reflect.getOwnPropertyDescriptor(value, property);
        },
        get(value, property, receiver) {
          trapCalls += 1;
          return Reflect.get(value, property, receiver);
        },
        has(value, property) { trapCalls += 1; return Reflect.has(value, property); },
      });
      expect(() => verify(proxy)).toThrow();
      expect(trapCalls).toBe(0);
    }
  });

  it('rejects missing, surplus, reordered, bit-flipped, or contradictory consistency proofs', () => {
    const proof = consistencyProof(3, leaves.slice(0, 7)).map(hex);
    const valid = {
      oldTreeSize: '3', newTreeSize: '7',
      oldRootDigest: treeHash(leaves.slice(0, 3)).toString('hex'),
      newRootDigest: ROOT_7, proofDigests: proof,
    };
    const flipped = [...proof];
    flipped[0] = `${flipped[0][0] === '0' ? '1' : '0'}${flipped[0].slice(1)}`;
    const invalid = [
      { ...valid, oldTreeSize: '8' }, { ...valid, oldTreeSize: '03' },
      { ...valid, oldRootDigest: '0'.repeat(64) }, { ...valid, extra: true },
      { ...valid, proofDigests: proof.slice(1) },
      { ...valid, proofDigests: [...proof, '1'.repeat(64)] },
      { ...valid, proofDigests: [...proof].reverse() }, { ...valid, proofDigests: flipped },
      { ...valid, oldTreeSize: '7', proofDigests: [] },
      { ...valid, oldTreeSize: '0', oldRootDigest: '1'.repeat(64), proofDigests: [] },
      { ...valid, oldTreeSize: '0', oldRootDigest: EMPTY_ROOT, proofDigests: proof },
    ];
    for (const value of invalid) {
      expect(() => verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1(value as any))
        .toThrow();
    }
  });
});

function hash(bytes: Uint8Array): Buffer {
  return createHash('sha256').update(bytes).digest();
}
function leafHash(bytes: Uint8Array): Buffer {
  return hash(Buffer.concat([Buffer.from([0]), Buffer.from(bytes)]));
}
function nodeHash(left: Uint8Array, right: Uint8Array): Buffer {
  return hash(Buffer.concat([Buffer.from([1]), Buffer.from(left), Buffer.from(right)]));
}
function treeHash(tree: readonly Uint8Array[]): Buffer {
  if (tree.length === 0) return hash(Buffer.alloc(0));
  if (tree.length === 1) return leafHash(tree[0]);
  const split = largestPowerOfTwoLessThan(tree.length);
  return nodeHash(treeHash(tree.slice(0, split)), treeHash(tree.slice(split)));
}
function inclusionProof(index: number, tree: readonly Uint8Array[]): Buffer[] {
  if (tree.length === 1) return [];
  const split = largestPowerOfTwoLessThan(tree.length);
  return index < split
    ? [...inclusionProof(index, tree.slice(0, split)), treeHash(tree.slice(split))]
    : [...inclusionProof(index - split, tree.slice(split)), treeHash(tree.slice(0, split))];
}
function consistencyProof(oldSize: number, tree: readonly Uint8Array[]): Buffer[] {
  return consistencySubproof(oldSize, tree, true);
}
function consistencySubproof(
  oldSize: number, tree: readonly Uint8Array[], complete: boolean,
): Buffer[] {
  if (oldSize === tree.length) return complete ? [] : [treeHash(tree)];
  const split = largestPowerOfTwoLessThan(tree.length);
  return oldSize <= split
    ? [...consistencySubproof(oldSize, tree.slice(0, split), complete), treeHash(tree.slice(split))]
    : [...consistencySubproof(oldSize - split, tree.slice(split), false),
      treeHash(tree.slice(0, split))];
}
function largestPowerOfTwoLessThan(value: number): number {
  let result = 1;
  while (result * 2 < value) result *= 2;
  return result;
}
function hex(value: Uint8Array): string { return Buffer.from(value).toString('hex'); }

const UNIFORM_LEAF = Buffer.from('x', 'utf8');
const UNIFORM_TREE_HASHES = new Map<bigint, Buffer>();
function uniformTreeHash(size: bigint): Buffer {
  const cached = UNIFORM_TREE_HASHES.get(size);
  if (cached) return cached;
  let result: Buffer;
  if (size === 0n) result = hash(Buffer.alloc(0));
  else if (size === 1n) result = leafHash(UNIFORM_LEAF);
  else {
    const split = largestPowerOfTwoLessThanBigInt(size);
    result = nodeHash(uniformTreeHash(split), uniformTreeHash(size - split));
  }
  UNIFORM_TREE_HASHES.set(size, result);
  return result;
}
function uniformConsistencyProof(
  oldSize: bigint, newSize: bigint, complete = true,
): Buffer[] {
  if (oldSize === newSize) return complete ? [] : [uniformTreeHash(newSize)];
  const split = largestPowerOfTwoLessThanBigInt(newSize);
  return oldSize <= split
    ? [...uniformConsistencyProof(oldSize, split, complete), uniformTreeHash(newSize - split)]
    : [...uniformConsistencyProof(oldSize - split, newSize - split, false),
      uniformTreeHash(split)];
}
function largestPowerOfTwoLessThanBigInt(value: bigint): bigint {
  let result = 1n;
  while ((result << 1n) < value) result <<= 1n;
  return result;
}
