// SPDX-License-Identifier: MIT

import type {
  AcceptanceTaskV3,
  NamedAcceptanceCommand,
} from './acceptance-task.js';
import { deepFreeze, type StructuredCommand } from './contracts.js';
import { digestValue } from './receipts.js';

export const PROGRAMME_RUST_COMMAND_PROFILE_V1 = deepFreeze({
  cargoExecutable: '/toolchain/bin/cargo' as const,
  environment: {
    PATH: '/cargo-home/bin:/toolchain/bin:/usr/bin',
    HOME: '/home/harness',
    CARGO_HOME: '/cargo-home',
    CARGO_NET_OFFLINE: 'true',
    CARGO_INCREMENTAL: '0',
  },
});

export interface ProgrammeCommandReceiptProjectionV1 {
  readonly commandId: string;
  readonly stage: 'red-baseline' | 'build' | 'public' | 'independent' | 'regression' | 'mutation';
  readonly tool: string;
  readonly executable: string;
  readonly argv: readonly string[];
  readonly cwd: string;
  readonly environmentDigest: string;
  readonly timeoutMs: number;
  readonly maxOutputBytes: number;
  readonly mutationId?: string;
}

export function bindProgrammeTaskRuntimeV1(task: AcceptanceTaskV3): AcceptanceTaskV3 {
  const bound = structuredClone(task);
  const bindNamed = (entries: NamedAcceptanceCommand[]) => entries.map((entry) => ({
    ...entry,
    command: bindCommand(entry.command),
  }));
  bound.redBaseline.commands = bindNamed(bound.redBaseline.commands);
  bound.commands.build = bindNamed(bound.commands.build);
  bound.commands.public = bindNamed(bound.commands.public);
  bound.commands.independent = bindNamed(bound.commands.independent);
  bound.commands.regression = bindNamed(bound.commands.regression);
  bound.commands.mutation = bound.commands.mutation.map((entry) => ({
    ...entry,
    command: bindCommand(entry.command),
  }));
  return deepFreeze(bound);
}

export function programmeBoundTaskDigestV1(task: AcceptanceTaskV3): string {
  return digestValue(bindProgrammeTaskRuntimeV1(task));
}

export function programmeCommandReceiptProjectionsV1(
  task: AcceptanceTaskV3,
): readonly ProgrammeCommandReceiptProjectionV1[] {
  const bound = bindProgrammeTaskRuntimeV1(task);
  const named = (
    stage: ProgrammeCommandReceiptProjectionV1['stage'],
    entries: readonly NamedAcceptanceCommand[],
  ) => entries.map(({ commandId, command }) => projection(stage, commandId, command));
  return deepFreeze([
    ...named('red-baseline', bound.redBaseline.commands),
    ...named('build', bound.commands.build),
    ...named('public', bound.commands.public),
    ...named('independent', bound.commands.independent),
    ...named('regression', bound.commands.regression),
    ...bound.commands.mutation.map(({ mutationId, command }) => ({
      ...projection('mutation', mutationId, command),
      mutationId,
    })),
  ]);
}

function bindCommand(command: StructuredCommand): StructuredCommand {
  if (command.tool !== 'cargo' || !command.argv.includes('--offline')) {
    throw new Error('HARNESS_PROGRAMME_TASK_RUNTIME_COMMAND_INVALID');
  }
  const subcommand = command.argv.find((argument) => !argument.startsWith('-'));
  if (subcommand !== 'fmt' && !command.argv.includes('--locked')) {
    throw new Error('HARNESS_PROGRAMME_TASK_RUNTIME_COMMAND_INVALID');
  }
  return {
    ...command,
    executable: PROGRAMME_RUST_COMMAND_PROFILE_V1.cargoExecutable,
    env: { ...command.env, ...PROGRAMME_RUST_COMMAND_PROFILE_V1.environment },
  };
}

function projection(
  stage: ProgrammeCommandReceiptProjectionV1['stage'],
  commandId: string,
  command: StructuredCommand,
): ProgrammeCommandReceiptProjectionV1 {
  return {
    commandId,
    stage,
    tool: command.tool,
    executable: command.executable,
    argv: [...command.argv],
    cwd: command.cwd,
    environmentDigest: digestValue(command.env),
    timeoutMs: command.timeoutMs,
    maxOutputBytes: command.maxOutputBytes,
  };
}
