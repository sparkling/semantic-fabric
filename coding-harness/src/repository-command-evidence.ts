// SPDX-License-Identifier: MIT

import type { StructuredCommand } from './contracts.js';
import type { ProcessResult } from './process.js';
import { digestValue, type CommandEvidence } from './receipts.js';

export type UnboundCommandEvidence = Omit<
  CommandEvidence,
  'stage' | 'attempt' | 'candidateTree'
>;

export function commandEvidence(
  command: StructuredCommand,
  result: ProcessResult,
): UnboundCommandEvidence {
  return Object.freeze({
    tool: command.tool,
    executable: command.executable,
    argv: [...command.argv],
    cwd: command.cwd,
    exitCode: result.exitCode,
    signal: result.signal,
    durationMs: result.durationMs,
    stdoutDigest: digestValue(result.stdout),
    stderrDigest: digestValue(result.stderr),
    timedOut: result.timedOut,
    cancelled: result.cancelled,
    outputLimitExceeded: result.outputLimitExceeded,
    spawnErrorDigest: result.spawnError === null ? null : digestValue(result.spawnError),
  });
}

export function commandPassed(command: UnboundCommandEvidence): boolean {
  return command.exitCode === 0
    && !command.timedOut
    && !command.cancelled
    && !command.outputLimitExceeded
    && command.spawnErrorDigest === null;
}
