// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  closeSync,
  constants,
  copyFileSync,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
  readSync,
  realpathSync,
} from 'node:fs';
import { join } from 'node:path';
import type { NativeResourceLimits } from './resource-boundary.js';

export const ISSUE_8_SYSTEM_PATHS = Object.freeze({
  cargo: '/home/claude/.rustup/toolchains/1.96.0-x86_64-unknown-linux-gnu/bin/cargo',
  toolchain: '/home/claude/.rustup/toolchains/1.96.0-x86_64-unknown-linux-gnu',
  registry: '/home/claude/.cargo/registry',
  registryKey: 'index.crates.io-1949cf8c6b5b557f',
  cargoLlvmCov: '/home/claude/.cargo/bin/cargo-llvm-cov',
  node: '/usr/bin/node',
  codex: '/home/claude/.codex/packages/standalone/releases/0.149.1-x86_64-unknown-linux-musl/bin/codex',
  claude: '/home/claude/.local/share/claude/versions/2.1.234',
  codexCredential: '/home/claude/.codex/auth.json',
  claudeCredential: '/home/claude/.claude/.credentials.json',
  bwrap: '/usr/bin/bwrap',
  systemdRun: '/usr/bin/systemd-run',
  systemctl: '/usr/bin/systemctl',
  caBundle: '/etc/ssl/certs/ca-certificates.crt',
  agenticQeRoot: '/home/claude/.npm-global/lib/node_modules/agentic-qe',
  agenticQeMcp: '/home/claude/.npm-global/lib/node_modules/agentic-qe/dist/mcp/bundle.js',
});

export const ISSUE_8_FROZEN_LOCK_DIGEST =
  '72916782d4d8fb87b613f61debe2107c160e083ef4969c89c23c7596df5b637d';
export const ISSUE_8_TARGET_TRIPLE = 'x86_64-unknown-linux-gnu';
export const ISSUE_8_LOCKED_REGISTRY_CONTENT_DIGEST =
  '1bb717af28554b8cbb83ff1a219bbbd294ccee98691191bc9f65dc431106e908';

const EXPECTED_SYSTEM_ARTIFACTS = Object.freeze({
  cargo: 'f30f9fd1b1d0b8fd10dc33219eb4cd4bec3543f40e434ac71f5a03fd0359063f',
  cargoLlvmCov: 'c59831d34b46a3e3a3dc5b357fa12f75eb0af3172f8e9e81a6fc1412cdbcaa1a',
  node: '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc',
  codex: '73dc5888888f411c1f0fa7b81d866e721dcc86b527ce8e3b2cf4708661e823ba',
  claude: '3473601ea695d5bf769c5b202844d4cb4fbf723ae995450fcb6973204775c84a',
  bwrap: '52231e1caf55bcbc667b269f49c63599a6f7db4767ae6a039580d0ff853db712',
  systemdRun: 'dbc8b988a849d5c9d7ef2de7068a6f107021bc6c11e0d7864c73f373eef726a7',
  systemctl: 'e0d3d0e9444da1b2b58c792c3f5028b69f049b77d5ca17b3ec0d09f89117225b',
  caBundle: '6602a85a36afc2e51c66a0df5ae3d383c5b7c2fed93339ccef7d37e01faf09e8',
  agenticQeMcp: 'eba7e52a3c86cd57203fb7f1cc079fb627491bee2b566bbb14c8ac4acdcaaa9b',
});

export const ISSUE_8_RUST_LIMITS: NativeResourceLimits = Object.freeze({
  memoryBytes: 32 * 1024 ** 3,
  processCount: 1024,
  cpuQuotaPercent: 1600,
  cpuTimeSeconds: 14_400,
  runtimeSeconds: 1_800,
  fileBytes: 20 * 1024 ** 3,
  openFiles: 8192,
});

export const ISSUE_8_NATIVE_LIMITS: NativeResourceLimits = Object.freeze({
  memoryBytes: 16 * 1024 ** 3,
  processCount: 512,
  cpuQuotaPercent: 800,
  cpuTimeSeconds: 7_200,
  runtimeSeconds: 1_800,
  fileBytes: 1024 ** 3,
  openFiles: 4096,
});

export function attestIssue8SystemTools(): Readonly<Record<string, string>> {
  const evidence: Record<string, string> = {};
  for (const [name, expected] of Object.entries(EXPECTED_SYSTEM_ARTIFACTS)) {
    const path = ISSUE_8_SYSTEM_PATHS[name as keyof typeof ISSUE_8_SYSTEM_PATHS];
    const actual = fileDigest(path);
    if (actual !== expected) throw new Error(`HARNESS_ISSUE_8_SYSTEM_TOOL_MISMATCH:${name}`);
    evidence[name] = `${path}#sha256:${actual}`;
  }
  return Object.freeze(evidence);
}

export function prepareCargoExtension(scratchRoot: string): string {
  const root = join(scratchRoot, 'cargo-extension');
  mkdirSync(root, { mode: 0o700 });
  const target = join(root, 'cargo-llvm-cov');
  copyFileSync(ISSUE_8_SYSTEM_PATHS.cargoLlvmCov, target, constants.COPYFILE_EXCL);
  chmodSync(target, 0o555);
  if (fileDigest(target) !== EXPECTED_SYSTEM_ARTIFACTS.cargoLlvmCov) {
    throw new Error('HARNESS_ISSUE_8_CARGO_EXTENSION_COPY_MISMATCH');
  }
  return root;
}

function fileDigest(path: string): string {
  const stat = lstatSync(path, { bigint: true });
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1n
    || realpathSync(path) !== path || stat.size < 1n || stat.size > 500_000_000n) {
    throw new Error('HARNESS_ISSUE_8_SYSTEM_TOOL_UNTRUSTED');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
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
    if (offset !== before.size || before.dev !== after.dev || before.ino !== after.ino
      || before.mode !== after.mode || before.size !== after.size
      || before.mtimeNs !== after.mtimeNs || before.ctimeNs !== after.ctimeNs) {
      throw new Error('HARNESS_ISSUE_8_SYSTEM_TOOL_CHANGED');
    }
    return hash.digest('hex');
  } finally {
    closeSync(descriptor);
  }
}
