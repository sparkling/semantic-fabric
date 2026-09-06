// SPDX-License-Identifier: MIT

import { randomBytes } from 'node:crypto';
import {
  closeSync,
  constants,
  existsSync,
  fchmodSync,
  fsyncSync,
  lstatSync,
  openSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { dirname, isAbsolute, relative, resolve } from 'node:path';
import { canonical } from '@metaharness/harness';
import {
  parsePostgresObservationQualificationReceiptPairJson,
  verifyPostgresObservationQualificationReceiptPair,
} from './postgres-observation-qualification.js';
import type { PostgresObservationQualificationReceipt } from
  './postgres-observation-qualification-runner.js';

const MAX_RECEIPT_PAIR_BYTES = 2_097_155;

export const POSTGRES_QUALIFICATION_RECEIPT_PAIR_PATH =
  'tests/postgresql/postgresql-16-observation-qualification-receipt-pair-v1.json';

function readReceiptPairFile(
  path: string,
): readonly [PostgresObservationQualificationReceipt, PostgresObservationQualificationReceipt] {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || realpathSync(path) !== path
    || stat.size < 2 || stat.size > MAX_RECEIPT_PAIR_BYTES) {
    throw new Error('POSTGRES_QUALIFICATION_RECEIPT_FILE_INVALID');
  }
  const bytes = readFileSync(path);
  if (bytes.at(-1) !== 0x0a || bytes.subarray(0, -1).includes(0x0a)
    || bytes.includes(0x0d) || bytes.includes(0)) {
    throw new Error('POSTGRES_QUALIFICATION_RECEIPT_BYTES_INVALID');
  }
  const body = bytes.subarray(0, -1).toString('utf8');
  if (!bytes.subarray(0, -1).equals(Buffer.from(body, 'utf8'))) {
    throw new Error('POSTGRES_QUALIFICATION_RECEIPT_UTF8_INVALID');
  }
  return parsePostgresObservationQualificationReceiptPairJson(body);
}

export function readPostgresQualificationReceiptPair(
  repositoryRoot: string,
): readonly [PostgresObservationQualificationReceipt, PostgresObservationQualificationReceipt] {
  const root = canonicalRoot(repositoryRoot);
  return readReceiptPairFile(receiptPairPath(root));
}

export function writePostgresQualificationReceiptPair(
  repositoryRoot: string,
  receipts: readonly [
    PostgresObservationQualificationReceipt,
    PostgresObservationQualificationReceipt,
  ],
): void {
  verifyPostgresObservationQualificationReceiptPair(receipts);
  const root = canonicalRoot(repositoryRoot);
  const ordered = orderReceipts(receipts);
  const destination = receiptPairPath(root);
  requireReplaceableReceiptPair(destination);
  const token = randomBytes(16).toString('hex');
  const temporary = `${destination}.tmp-${token}`;
  const bytes = Buffer.from(`${canonical(ordered)}\n`, 'utf8');
  if (bytes.length > MAX_RECEIPT_PAIR_BYTES) {
    throw new Error('POSTGRES_QUALIFICATION_RECEIPT_PAIR_TOO_LARGE');
  }
  parsePostgresObservationQualificationReceiptPairJson(bytes.subarray(0, -1).toString('utf8'));
  try {
    writeDurableTemporary(temporary, bytes);
    readReceiptPairFile(temporary);
    renameSync(temporary, destination);
    fsyncDirectory(dirname(destination));
    readPostgresQualificationReceiptPair(root);
  } finally {
    if (existsSync(temporary)) rmSync(temporary);
  }
}

function orderReceipts(
  receipts: readonly [
    PostgresObservationQualificationReceipt,
    PostgresObservationQualificationReceipt,
  ],
): readonly [PostgresObservationQualificationReceipt, PostgresObservationQualificationReceipt] {
  const byVersion = new Map(receipts.map((receipt) => [
    receipt.candidates[0].observation.serverVersionNum, receipt,
  ]));
  const first = byVersion.get(160009);
  const second = byVersion.get(160015);
  if (first === undefined || second === undefined || byVersion.size !== 2) {
    throw new Error('POSTGRES_QUALIFICATION_RECEIPT_PAIR_INVALID');
  }
  return [first, second];
}

function requireReplaceableReceiptPair(path: string): void {
  if (!existsSync(path)) return;
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || realpathSync(path) !== path
    || stat.size < 2 || stat.size > MAX_RECEIPT_PAIR_BYTES) {
    throw new Error('POSTGRES_QUALIFICATION_EXISTING_RECEIPT_INVALID');
  }
}

function receiptPairPath(root: string): string {
  const path = resolve(root, POSTGRES_QUALIFICATION_RECEIPT_PAIR_PATH);
  const rel = relative(root, path);
  if (rel.startsWith('..') || isAbsolute(rel)) {
    throw new Error('POSTGRES_QUALIFICATION_RECEIPT_PATH_INVALID');
  }
  const parent = dirname(path);
  const stat = lstatSync(parent);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(parent) !== parent) {
    throw new Error('POSTGRES_QUALIFICATION_RECEIPT_DIRECTORY_INVALID');
  }
  return path;
}

function writeDurableTemporary(path: string, bytes: Buffer): void {
  const descriptor = openSync(
    path,
    constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | constants.O_NOFOLLOW,
    0o600,
  );
  try {
    writeFileSync(descriptor, bytes);
    fchmodSync(descriptor, 0o644);
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
}

function fsyncDirectory(path: string): void {
  const descriptor = openSync(
    path, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  );
  try {
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
}

function canonicalRoot(root: string): string {
  if (!isAbsolute(root) || resolve(root) !== root || realpathSync(root) !== root
    || !lstatSync(root).isDirectory()) {
    throw new Error('POSTGRES_QUALIFICATION_REPOSITORY_ROOT_INVALID');
  }
  return root;
}
