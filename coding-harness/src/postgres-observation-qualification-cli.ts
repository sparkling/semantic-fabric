// SPDX-License-Identifier: MIT

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  checkPostgresQualificationReceipts,
  generatePostgresQualificationReceiptPair,
  replayPostgresQualificationReceipt,
} from './postgres-observation-qualification-programme.js';
import type { PostgresQualificationPatch } from
  './postgres-observation-qualification-protocol.js';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

try {
  const command = parseCommand(process.argv.slice(2));
  if (command.name === 'check') {
    const receipts = await checkPostgresQualificationReceipts(repositoryRoot);
    writeSuccess({
      command: 'check',
      patches: receipts.map((receipt) =>
        receipt.candidates[0].observation.serverVersionNum === 160009 ? '16.9' : '16.15'),
    });
  } else if (command.name === 'generate-pair') {
    const receipts = await generatePostgresQualificationReceiptPair(repositoryRoot);
    writeSuccess({
      command: 'generate-pair',
      receiptSha256: receipts.map((receipt) => receipt.receiptSha256),
    });
  } else {
    await replayPostgresQualificationReceipt(repositoryRoot, command.patch);
    writeSuccess({ command: 'replay', patch: command.patch });
  }
} catch (error) {
  process.stderr.write(
    `POSTGRES_OBSERVATION_QUALIFICATION_FAILED:${safeFailureCode(error)}\n`,
  );
  process.exitCode = 1;
}

type Command =
  | Readonly<{ name: 'check' }>
  | Readonly<{ name: 'generate-pair' }>
  | Readonly<{ name: 'replay'; patch: PostgresQualificationPatch }>;

function parseCommand(args: readonly string[]): Command {
  if (args.length === 1 && args[0] === 'check') return Object.freeze({ name: 'check' });
  if (args.length === 1 && args[0] === 'generate-pair') {
    return Object.freeze({ name: 'generate-pair' });
  }
  if (args.length === 3 && args[0] === 'replay' && args[1] === '--patch'
    && (args[2] === '16.9' || args[2] === '16.15')) {
    return Object.freeze({ name: 'replay', patch: args[2] });
  }
  throw new TypeError('POSTGRES_QUALIFICATION_COMMAND_INVALID');
}

function writeSuccess(value: Readonly<Record<string, unknown>>): void {
  process.stdout.write(`${JSON.stringify({ ...value, status: 'pass' })}\n`);
}

function safeFailureCode(error: unknown): string {
  const message = error instanceof Error ? error.message : '';
  return message.match(/\bPOSTGRES_QUALIFICATION_[A-Z0-9_]+/)?.[0] ?? 'UNCLASSIFIED';
}
