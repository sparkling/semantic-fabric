// SPDX-License-Identifier: MIT

import type { HarnessConfig, StructuredCommand } from './contracts.js';
import type { OfflineProcessIsolator } from './network.js';
import { runStructuredProcess } from './process.js';
import { digestValue } from './receipts.js';
import {
  commandEvidence,
  type UnboundCommandEvidence,
} from './repository-command-evidence.js';
import type { VerifierGeneratedOutputSpec } from './repository-options.js';
import { prepareWorkspaceFileOverlays } from './writable-overlays.js';
import { normalizeWorkspacePath } from './contracts.js';

export interface RepositoryCommandBatch {
  readonly commands: readonly UnboundCommandEvidence[];
  readonly generatedOutputDigests: Readonly<Record<string, string>>;
}

export async function runRepositoryCommandBatch(input: Readonly<{
  commands: readonly StructuredCommand[];
  workspaceRoot: string;
  controlledRoot: string;
  writablePaths: readonly string[];
  outputRoot: string;
  config: HarnessConfig;
  declaredTools: readonly string[];
  offlineIsolator: OfflineProcessIsolator;
  offlineEnvironment: Readonly<Record<string, string | undefined>>;
  trackedPaths: readonly string[];
  generatedOutputs?: readonly VerifierGeneratedOutputSpec[];
  candidateTree?: string;
  signal?: AbortSignal;
}>): Promise<RepositoryCommandBatch> {
  const specs = validateSpecs(input);
  const evidence: UnboundCommandEvidence[] = [];
  const generatedOutputDigests: Record<string, string> = {};
  for (const command of input.commands) {
    const executed = rustCommand(command, input.outputRoot);
    const spec = specs.get(stableCommand(command));
    const lease = spec === undefined ? null : prepareWorkspaceFileOverlays({
      controlledRoot: input.controlledRoot,
      workspaceRoot: input.workspaceRoot,
      outputRoot: input.outputRoot,
      workspacePaths: spec.workspacePaths,
    });
    let result;
    try {
      result = await runStructuredProcess(executed, {
        workspaceRoot: input.workspaceRoot,
        config: input.config,
        declaredTools: input.declaredTools,
        sourceEnvironment: input.offlineEnvironment,
        signal: input.signal,
        boundary: {
          kind: 'offline-candidate',
          isolator: input.offlineIsolator,
          writablePaths: input.writablePaths,
          writableOverlays: lease?.mounts,
        },
      });
    } catch (error) {
      if (lease !== null) {
        try {
          lease.assertOriginalsStable();
        } catch (stabilityError) {
          throw new AggregateError(
            [error, stabilityError],
            'HARNESS_GENERATED_OUTPUT_EXECUTION_AND_STABILITY_FAILED',
          );
        }
      }
      throw error;
    }
    const commandReceipt = commandEvidence(executed, result);
    evidence.push(commandReceipt);
    if (lease !== null && spec !== undefined) {
      const files = lease.seal();
      generatedOutputDigests[spec.evidenceId] = digestValue({
        evidenceId: spec.evidenceId,
        candidateTree: input.candidateTree,
        producerCommandDigest: digestValue(executed),
        producerResultDigest: digestValue(commandReceipt),
        files,
      });
    }
  }
  return Object.freeze({
    commands: Object.freeze(evidence),
    generatedOutputDigests: Object.freeze(generatedOutputDigests),
  });
}

function validateSpecs(
  input: Parameters<typeof runRepositoryCommandBatch>[0],
): Map<string, VerifierGeneratedOutputSpec> {
  const specs = input.generatedOutputs ?? [];
  if (specs.length > 0 && input.candidateTree === undefined) {
    throw new Error('HARNESS_GENERATED_OUTPUT_CANDIDATE_TREE_REQUIRED');
  }
  const commands = input.commands.map(stableCommand);
  const tracked = new Set(input.trackedPaths);
  const byCommand = new Map<string, VerifierGeneratedOutputSpec>();
  const evidenceIds = new Set<string>();
  for (const [index, raw] of specs.entries()) {
    if (!/^[a-z0-9][a-z0-9-]{0,63}$/.test(raw.evidenceId)
      || evidenceIds.has(raw.evidenceId)) {
      throw new Error('HARNESS_GENERATED_OUTPUT_EVIDENCE_ID_INVALID');
    }
    const command = stableCommand(raw.command);
    if (commands.filter((entry) => entry === command).length !== 1 || byCommand.has(command)) {
      throw new Error('HARNESS_GENERATED_OUTPUT_COMMAND_BINDING_INVALID');
    }
    const workspacePaths = raw.workspacePaths.map((path, pathIndex) =>
      normalizeWorkspacePath(path, `generatedOutputs[${index}].workspacePaths[${pathIndex}]`));
    if (workspacePaths.length === 0 || new Set(workspacePaths).size !== workspacePaths.length
      || workspacePaths.some((path) => !tracked.has(path))) {
      throw new Error('HARNESS_GENERATED_OUTPUT_PATHS_INVALID');
    }
    evidenceIds.add(raw.evidenceId);
    byCommand.set(command, Object.freeze({
      evidenceId: raw.evidenceId,
      command: raw.command,
      workspacePaths: Object.freeze(workspacePaths),
    }));
  }
  return byCommand;
}

function rustCommand(command: StructuredCommand, outputRoot: string): StructuredCommand {
  return command.tool === 'cargo' || command.tool === 'rustc'
    ? {
        ...command,
        env: {
          ...command.env,
          HOME: '/home/harness',
          CARGO_HOME: '/cargo-home',
          CARGO_NET_OFFLINE: 'true',
          CARGO_INCREMENTAL: '0',
          CARGO_TARGET_DIR: outputRoot,
        },
      }
    : command;
}

function stableCommand(command: StructuredCommand): string {
  return JSON.stringify(command);
}
