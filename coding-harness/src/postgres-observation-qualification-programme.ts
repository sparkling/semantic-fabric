// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, realpathSync } from 'node:fs';
import { isAbsolute, relative, resolve } from 'node:path';
import { canonical } from '@metaharness/harness';
import { buildPostgresQualificationProbe } from
  './postgres-observation-qualification-build.js';
import type { PostgresQualificationToolchainEvidence } from
  './postgres-observation-qualification-build.js';
import { runPostgresQualificationContainer } from
  './postgres-observation-qualification-docker.js';
import {
  readPostgresQualificationReceiptPair,
  writePostgresQualificationReceiptPair,
} from './postgres-observation-qualification-io.js';
import {
  collectQualificationProvenance,
  requireCleanQualificationSource,
  verifyQualificationSourceProvenance,
} from './postgres-observation-qualification-provenance.js';
import type { QualificationProvenance } from
  './postgres-observation-qualification-provenance.js';
import {
  POSTGRES_QUALIFICATION_PROTOCOL_PATH,
  expectedQualificationBuilderImageIdentity,
  expectedQualificationImageIdentity,
  parsePostgresQualificationProtocol,
  protocolImage,
  type PostgresQualificationImage,
  type PostgresQualificationPatch,
  type PostgresQualificationProtocol,
} from './postgres-observation-qualification-protocol.js';
import type {
  PostgresObservationQualificationCandidate,
  PostgresObservationQualificationReceipt,
} from './postgres-observation-qualification-runner.js';
import { withPostgresQualificationSourceWorktree } from
  './postgres-observation-qualification-source-worktree.js';
import {
  createPostgresObservationQualificationReceipt,
  parsePostgresObservationQualificationCandidate,
  parsePostgresObservationQualificationProbeOutput,
  verifyPostgresObservationQualificationReceipt,
  verifyPostgresObservationQualificationReceiptPair,
} from './postgres-observation-qualification.js';

export async function generatePostgresQualificationReceiptPair(
  repositoryRoot: string,
): Promise<readonly [
  PostgresObservationQualificationReceipt,
  PostgresObservationQualificationReceipt,
]> {
  const root = canonicalRoot(repositoryRoot);
  await requireCleanQualificationSource(root);
  const protocol = readProtocol(root);
  const toolchain = await buildPostgresQualificationProbe({
    repositoryRoot: root, protocol,
  });
  const receipts = await Promise.all(protocol.images.map((image) =>
    executePatch(root, protocol, image, toolchain))) as [
      PostgresObservationQualificationReceipt,
      PostgresObservationQualificationReceipt,
    ];
  verifyPostgresObservationQualificationReceiptPair(receipts);
  for (const receipt of receipts) validateReceiptAgainstProtocol(receipt, protocol);
  writePostgresQualificationReceiptPair(root, receipts);
  return receipts;
}

export async function checkPostgresQualificationReceipts(
  repositoryRoot: string,
): Promise<readonly [
  PostgresObservationQualificationReceipt,
  PostgresObservationQualificationReceipt,
]> {
  const root = canonicalRoot(repositoryRoot);
  const protocol = readProtocol(root);
  const receipts = readPostgresQualificationReceiptPair(root);
  for (const receipt of receipts) {
    validateReceiptAgainstProtocol(receipt, protocol);
    await verifyQualificationSourceProvenance(root, receipt.candidates[0].provenance);
  }
  verifyPostgresObservationQualificationReceiptPair(receipts);
  await withPostgresQualificationSourceWorktree(
    root,
    receipts[0].candidates[0].provenance.source.commit,
    async (sourceRoot) => {
      const sourceProtocol = readProtocol(sourceRoot);
      for (const receipt of receipts) validateReceiptAgainstProtocol(receipt, sourceProtocol);
      await buildAndVerifyProvenance(
        sourceRoot, sourceProtocol, receipts[0].candidates[0].provenance,
      );
    },
  );
  return receipts;
}

export async function replayPostgresQualificationReceipt(
  repositoryRoot: string,
  patch: PostgresQualificationPatch,
): Promise<PostgresObservationQualificationReceipt> {
  const root = canonicalRoot(repositoryRoot);
  const receipts = readPostgresQualificationReceiptPair(root);
  const expected = receipts.find((receipt) =>
    receipt.candidates[0].observation.serverVersionNum === (patch === '16.9' ? 160009 : 160015));
  if (expected === undefined) throw new Error('POSTGRES_QUALIFICATION_RECEIPT_PATCH_MISSING');
  const currentProtocol = readProtocol(root);
  for (const receipt of receipts) {
    validateReceiptAgainstProtocol(receipt, currentProtocol);
    await verifyQualificationSourceProvenance(root, receipt.candidates[0].provenance);
  }
  const replayed = await withPostgresQualificationSourceWorktree(
    root,
    expected.candidates[0].provenance.source.commit,
    async (sourceRoot) => {
      const protocol = readProtocol(sourceRoot);
      validateReceiptAgainstProtocol(expected, protocol);
      const toolchain = await buildAndVerifyProvenance(
        sourceRoot, protocol, expected.candidates[0].provenance,
      );
      return executePatch(sourceRoot, protocol, protocolImage(protocol, patch), toolchain);
    },
  );
  if (canonical(replayed.candidates[0]) !== canonical(expected.candidates[0])) {
    throw new Error('POSTGRES_QUALIFICATION_REPLAY_CANDIDATE_MISMATCH');
  }
  const oldContainers = new Set(
    expected.replay.executions.map((entry) => entry.containerIdentitySha256),
  );
  const oldVolumes = new Set(
    expected.replay.executions.map((entry) => entry.volumeIdentitySha256),
  );
  if (replayed.replay.executions.some((entry) =>
    oldContainers.has(entry.containerIdentitySha256)
      || oldVolumes.has(entry.volumeIdentitySha256))) {
    throw new Error('POSTGRES_QUALIFICATION_REPLAY_RESOURCE_REUSE');
  }
  return replayed;
}

async function buildAndVerifyProvenance(
  sourceRoot: string,
  protocol: PostgresQualificationProtocol,
  provenance: QualificationProvenance,
): Promise<PostgresQualificationToolchainEvidence> {
  const toolchain = await buildPostgresQualificationProbe({ repositoryRoot: sourceRoot, protocol });
  if (sha256(toolchain.rustcVv) !== provenance.toolchain.rustcVvSha256
    || sha256(toolchain.cargoVersion) !== provenance.toolchain.cargoVersionSha256
    || canonical(toolchain.builderImage) !== canonical(provenance.toolchain.builderImage)) {
    throw new Error('POSTGRES_QUALIFICATION_TOOLCHAIN_PROVENANCE_MISMATCH');
  }
  const artifactPath = resolve(sourceRoot, protocol.probe.artifactPath);
  const relativeArtifact = relative(sourceRoot, artifactPath);
  const stat = lstatSync(artifactPath);
  if (!relativeArtifact || relativeArtifact.startsWith('../') || isAbsolute(relativeArtifact)
    || !stat.isFile() || stat.isSymbolicLink() || realpathSync(artifactPath) !== artifactPath
    || stat.size < 1 || stat.size > 128 * 1024 * 1024
    || sha256(readFileSync(artifactPath)) !== provenance.source.probeArtifactSha256) {
    throw new Error('POSTGRES_QUALIFICATION_ARTIFACT_PROVENANCE_MISMATCH');
  }
  return toolchain;
}

async function executePatch(
  root: string,
  protocol: PostgresQualificationProtocol,
  image: PostgresQualificationImage,
  toolchain: PostgresQualificationToolchainEvidence,
): Promise<PostgresObservationQualificationReceipt> {
  const runs = await Promise.all(([1, 2] as const).map((slot) =>
    runPostgresQualificationContainer({
      repositoryRoot: root, protocol, image, slot,
    })));
  const outputDigest = sha256(runs[0].probeStdout);
  if (sha256(runs[1].probeStdout) !== outputDigest
    || canonical(runs[0].image) !== canonical(runs[1].image)) {
    throw new Error('POSTGRES_QUALIFICATION_EXECUTION_REPLAY_DRIFT');
  }
  const provenance = await collectQualificationProvenance({
    repositoryRoot: root,
    artifactPath: protocol.probe.artifactPath,
    rustcVv: toolchain.rustcVv,
    cargoVersion: toolchain.cargoVersion,
    builderImage: toolchain.builderImage,
    probeStdoutSha256: outputDigest,
    image: runs[0].image,
  });
  const candidates = runs.map((run) =>
    parsePostgresObservationQualificationCandidate({
      provenance, ...parseProbeBytes(run.probeStdout),
    })) as [
    PostgresObservationQualificationCandidate,
    PostgresObservationQualificationCandidate,
  ];
  const receipt = createPostgresObservationQualificationReceipt({
    candidates,
    executions: runs.map((run) => run.execution),
  });
  validateReceiptAgainstProtocol(receipt, protocol);
  return receipt;
}

function parseProbeBytes(value: Buffer) {
  if (value.length < 2 || value.includes(0) || value.includes(0x0a) || value.includes(0x0d)) {
    throw new Error('POSTGRES_QUALIFICATION_PROBE_BYTES_INVALID');
  }
  const text = value.toString('utf8');
  if (!value.equals(Buffer.from(text, 'utf8'))) {
    throw new Error('POSTGRES_QUALIFICATION_PROBE_UTF8_INVALID');
  }
  const parsed = parsePostgresObservationQualificationProbeOutput(text);
  if (JSON.stringify(JSON.parse(text)) !== text) {
    throw new Error('POSTGRES_QUALIFICATION_PROBE_SERIALIZATION_INVALID');
  }
  return parsed;
}

function validateReceiptAgainstProtocol(
  receipt: PostgresObservationQualificationReceipt,
  protocol: PostgresQualificationProtocol,
): void {
  verifyPostgresObservationQualificationReceipt(receipt);
  const serverVersion = receipt.candidates[0].observation.serverVersionNum;
  let patch: PostgresQualificationPatch;
  if (serverVersion === 160009) patch = '16.9';
  else if (serverVersion === 160015) patch = '16.15';
  else throw new Error('POSTGRES_QUALIFICATION_RECEIPT_SERVER_VERSION_UNSUPPORTED');
  const expected = protocolImage(protocol, patch);
  for (const candidate of receipt.candidates) {
    if (candidate.observation.serverVersionNum !== expected.serverVersionNum
      || canonical(candidate.provenance.image)
        !== canonical(expectedQualificationImageIdentity(expected))
      || canonical(candidate.provenance.toolchain.builderImage)
        !== canonical(expectedQualificationBuilderImageIdentity(protocol.builder))) {
      throw new Error('POSTGRES_QUALIFICATION_RECEIPT_PROTOCOL_MISMATCH');
    }
  }
}

function readProtocol(root: string): PostgresQualificationProtocol {
  const path = resolve(root, POSTGRES_QUALIFICATION_PROTOCOL_PATH);
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || realpathSync(path) !== path
    || stat.size < 2 || stat.size > 65_536) {
    throw new Error('POSTGRES_QUALIFICATION_PROTOCOL_FILE_INVALID');
  }
  const bytes = readFileSync(path);
  const text = bytes.toString('utf8');
  if (!bytes.equals(Buffer.from(text, 'utf8'))) {
    throw new Error('POSTGRES_QUALIFICATION_PROTOCOL_UTF8_INVALID');
  }
  return parsePostgresQualificationProtocol(text);
}

function canonicalRoot(root: string): string {
  if (!isAbsolute(root) || resolve(root) !== root || realpathSync(root) !== root
    || !lstatSync(root).isDirectory()) {
    throw new Error('POSTGRES_QUALIFICATION_REPOSITORY_ROOT_INVALID');
  }
  return root;
}

function sha256(value: Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}
