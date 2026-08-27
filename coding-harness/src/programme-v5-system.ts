// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
} from 'node:fs';
import { captureAgenticQePackage } from './agentic-qe-mcp-identity.js';
import { ISSUE_8_SYSTEM_PATHS } from './issue-8-system.js';
import { assertTaskQeRunnerIdentity } from './task-qe.js';

export const PROGRAMME_V5_AGENTIC_QE_VERSION =
  '3.13.12#sast-only-flat-v1+lcov-gap' as const;
export const PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY = Object.freeze({
  version: '3.13.12',
  treeSha256: '0e7497a02997c9c43c2dbe9c200ce016c8a6c345b8fdb5d5ee99d61ff8722884',
});

const PROGRAMME_V5_AGENTIC_QE_MCP_DIGEST =
  'a07f22e29ff2dd074e05b30ccdaf76ce042418e6a879d83807e9fdd722dfa483';
const MAX_TOOL_BYTES = 500_000_000n;
const MAX_PACKAGE_FILES = 20_000;
const MAX_PACKAGE_BYTES = 500_000_000;

const SYSTEM_FILES = Object.freeze([
  ['cargo', ISSUE_8_SYSTEM_PATHS.cargo,
    'f30f9fd1b1d0b8fd10dc33219eb4cd4bec3543f40e434ac71f5a03fd0359063f'],
  ['cargoLlvmCov', ISSUE_8_SYSTEM_PATHS.cargoLlvmCov,
    'c59831d34b46a3e3a3dc5b357fa12f75eb0af3172f8e9e81a6fc1412cdbcaa1a'],
  ['node', ISSUE_8_SYSTEM_PATHS.node,
    '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc'],
  ['codexExecutable', ISSUE_8_SYSTEM_PATHS.codex,
    '73dc5888888f411c1f0fa7b81d866e721dcc86b527ce8e3b2cf4708661e823ba'],
  ['claudeExecutable', ISSUE_8_SYSTEM_PATHS.claude,
    '3473601ea695d5bf769c5b202844d4cb4fbf723ae995450fcb6973204775c84a'],
  ['bwrap', ISSUE_8_SYSTEM_PATHS.bwrap,
    '52231e1caf55bcbc667b269f49c63599a6f7db4767ae6a039580d0ff853db712'],
  ['systemdRun', ISSUE_8_SYSTEM_PATHS.systemdRun,
    'dbc8b988a849d5c9d7ef2de7068a6f107021bc6c11e0d7864c73f373eef726a7'],
  ['systemctl', ISSUE_8_SYSTEM_PATHS.systemctl,
    'e0d3d0e9444da1b2b58c792c3f5028b69f049b77d5ca17b3ec0d09f89117225b'],
  ['caBundle', ISSUE_8_SYSTEM_PATHS.caBundle,
    '6602a85a36afc2e51c66a0df5ae3d383c5b7c2fed93339ccef7d37e01faf09e8'],
] as const);

export function attestProgrammeV5SystemTools(): Readonly<Record<string, string>> {
  const evidence: Record<string, string> = {};
  for (const [name, path, expected] of SYSTEM_FILES) {
    const actual = stableProgrammeV5SystemFileDigest(path, name !== 'caBundle');
    if (actual !== expected) {
      throw new Error(`HARNESS_PROGRAMME_V5_SYSTEM_TOOL_MISMATCH:${name}`);
    }
    evidence[name] = `${path}#sha256:${actual}`;
  }

  const agenticQe = captureAgenticQePackage(
    ISSUE_8_SYSTEM_PATHS.agenticQeRoot,
    ISSUE_8_SYSTEM_PATHS.agenticQeMcp,
    { maxFiles: MAX_PACKAGE_FILES, maxBytes: MAX_PACKAGE_BYTES },
  );
  assertTaskQeRunnerIdentity(
    { package: agenticQe },
    PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY,
  );
  if (agenticQe.entrySha256 !== PROGRAMME_V5_AGENTIC_QE_MCP_DIGEST) {
    throw new Error('HARNESS_PROGRAMME_V5_AGENTIC_QE_ENTRY_MISMATCH');
  }
  evidence.agenticQeMcp = `${agenticQe.entryPath}#sha256:${agenticQe.entrySha256}`;
  evidence.agenticQe = PROGRAMME_V5_AGENTIC_QE_VERSION;
  evidence.agenticQePackageTreeDigest = agenticQe.treeSha256;
  return Object.freeze(evidence);
}

export function stableProgrammeV5SystemFileDigest(path: string, executable: boolean): string {
  const pathStat = lstatSync(path, { bigint: true });
  const uid = BigInt(process.getuid?.() ?? Number(pathStat.uid));
  if (!pathStat.isFile() || pathStat.isSymbolicLink() || pathStat.nlink !== 1n
    || realpathSync(path) !== path || pathStat.size < 1n || pathStat.size > MAX_TOOL_BYTES
    || (executable && (pathStat.mode & 0o111n) === 0n)
    || (pathStat.uid !== 0n && pathStat.uid !== uid) || (pathStat.mode & 0o022n) !== 0n) {
    throw new Error('HARNESS_PROGRAMME_V5_SYSTEM_TOOL_UNTRUSTED');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    if (!sameFile(pathStat, before)) {
      throw new Error('HARNESS_PROGRAMME_V5_SYSTEM_TOOL_CHANGED');
    }
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < before.size) {
      const count = readSync(
        descriptor,
        buffer,
        0,
        Math.min(buffer.length, Number(before.size - offset)),
        Number(offset),
      );
      if (count === 0) break;
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (offset !== before.size || !sameFile(before, after)) {
      throw new Error('HARNESS_PROGRAMME_V5_SYSTEM_TOOL_CHANGED');
    }
    return hash.digest('hex');
  } finally {
    closeSync(descriptor);
  }
}

function sameFile(
  left: Readonly<{ dev: bigint; ino: bigint; mode: bigint; size: bigint; mtimeNs: bigint; ctimeNs: bigint }>,
  right: Readonly<{ dev: bigint; ino: bigint; mode: bigint; size: bigint; mtimeNs: bigint; ctimeNs: bigint }>,
): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.size === right.size && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}
