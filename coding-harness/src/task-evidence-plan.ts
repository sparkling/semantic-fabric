// SPDX-License-Identifier: MIT

import type { AcceptanceTask, NamedAcceptanceCommand } from './acceptance-task.js';
import type {
  GeneratedOutputStage,
  TaskGeneratedOutputBinding,
  TaskQeBinding,
} from './acceptance-task-v3.js';
import { assertTaskQeBindings } from './acceptance-task-v3.js';
import type { VerifierStage } from './candidate.js';
import { deepFreeze, type StructuredCommand } from './contracts.js';
import type { AgenticQeProfile } from './evidence.js';
import { digestValue } from './receipts.js';
import type { VerifierGeneratedOutputSpec } from './repository-options.js';

const VERIFIER_STAGES = Object.freeze([
  'public', 'independent', 'regression',
] as const satisfies readonly VerifierStage[]);

export interface LegacyTaskEvidenceBindings {
  readonly taskPath: string;
  readonly taskId: string;
  readonly qeBindings: readonly TaskQeBinding[];
  readonly generatedOutputs: readonly TaskGeneratedOutputBinding[];
}

export interface TaskEvidencePlan {
  readonly qeBindings: readonly TaskQeBinding[];
  readonly requiredQeProfiles: readonly AgenticQeProfile[];
  readonly verifierGeneratedOutputs: Readonly<Partial<Record<
    VerifierStage,
    readonly VerifierGeneratedOutputSpec[]
  >>>;
  readonly declarationDigest: string;
}

export function resolveTaskEvidencePlan(input: Readonly<{
  task: AcceptanceTask;
  taskPath: string;
  legacyV2?: LegacyTaskEvidenceBindings;
}>): TaskEvidencePlan {
  const declarations = evidenceDeclarations(input);
  assertTaskQeBindings(declarations.qeBindings);
  const qeBindings = declarations.qeBindings.map(cloneQeBinding);
  const generatedOutputs = declarations.generatedOutputs.map(cloneGeneratedOutput);
  assertTaskQeBindings(qeBindings);
  const verifierGeneratedOutputs = resolveGeneratedOutputs(input.task, generatedOutputs);
  const declarationDigest = digestValue({
    schemaVersion: 1,
    taskSchemaVersion: input.task.schemaVersion,
    taskPath: input.taskPath,
    taskId: input.task.taskId,
    qeBindings,
    generatedOutputs,
  });
  return deepFreeze({
    qeBindings,
    requiredQeProfiles: qeBindings.map(({ profile }) => profile),
    verifierGeneratedOutputs,
    declarationDigest,
  });
}

function evidenceDeclarations(input: Readonly<{
  task: AcceptanceTask;
  taskPath: string;
  legacyV2?: LegacyTaskEvidenceBindings;
}>): Readonly<{
  qeBindings: readonly TaskQeBinding[];
  generatedOutputs: readonly TaskGeneratedOutputBinding[];
}> {
  if (input.task.schemaVersion === 3) {
    if (input.legacyV2 !== undefined) {
      throw new Error('HARNESS_TASK_EVIDENCE_V3_LEGACY_BINDINGS_FORBIDDEN');
    }
    return {
      qeBindings: input.task.qe.profiles,
      generatedOutputs: input.task.evidence.generatedOutputs,
    };
  }
  const legacy = input.legacyV2;
  if (legacy === undefined
    || legacy.taskPath !== input.taskPath
    || legacy.taskId !== input.task.taskId) {
    throw new Error('HARNESS_TASK_EVIDENCE_V2_COMPATIBILITY_MISMATCH');
  }
  const declaredProfiles = legacy.qeBindings.map(({ profile }) => profile);
  if (JSON.stringify(declaredProfiles) !== JSON.stringify(input.task.qeProfiles)) {
    throw new Error('HARNESS_TASK_EVIDENCE_V2_QE_MISMATCH');
  }
  return legacy;
}

function resolveGeneratedOutputs(
  task: AcceptanceTask,
  bindings: readonly TaskGeneratedOutputBinding[],
): Readonly<Partial<Record<VerifierStage, readonly VerifierGeneratedOutputSpec[]>>> {
  const evidenceIds = new Set<string>();
  const producers = new Set<string>();
  const output: Partial<Record<VerifierStage, VerifierGeneratedOutputSpec[]>> = {};
  for (const stage of VERIFIER_STAGES) {
    const specs = bindings.filter((binding) => binding.stage === stage).map((binding) => {
      if (evidenceIds.has(binding.evidenceId)) {
        throw new Error(`HARNESS_TASK_EVIDENCE_ID_DUPLICATE:${binding.evidenceId}`);
      }
      const producer = `${stage}\0${binding.commandId}`;
      if (producers.has(producer)) {
        throw new Error(`HARNESS_TASK_EVIDENCE_PRODUCER_DUPLICATE:${binding.commandId}`);
      }
      const matches = commandsForStage(task, stage)
        .filter(({ commandId }) => commandId === binding.commandId);
      if (matches.length !== 1) {
        throw new Error(`HARNESS_TASK_EVIDENCE_COMMAND_BINDING_INVALID:${binding.commandId}`);
      }
      if (binding.workspacePaths.length === 0
        || new Set(binding.workspacePaths).size !== binding.workspacePaths.length) {
        throw new Error(`HARNESS_TASK_EVIDENCE_OUTPUT_PATHS_INVALID:${binding.evidenceId}`);
      }
      evidenceIds.add(binding.evidenceId);
      producers.add(producer);
      return {
        evidenceId: binding.evidenceId,
        command: cloneCommand(matches[0].command),
        workspacePaths: [...binding.workspacePaths],
      };
    });
    if (specs.length > 0) output[stage] = specs;
  }
  if (bindings.some(({ stage }) => !VERIFIER_STAGES.includes(stage))) {
    throw new Error('HARNESS_TASK_EVIDENCE_STAGE_INVALID');
  }
  return output;
}

function commandsForStage(
  task: AcceptanceTask,
  stage: GeneratedOutputStage,
): readonly NamedAcceptanceCommand[] {
  return task.commands[stage];
}

function cloneQeBinding(binding: TaskQeBinding): TaskQeBinding {
  if (binding.profile === 'sast' && binding.collector === 'agentic-qe-sast') {
    return { profile: 'sast', collector: 'agentic-qe-sast' };
  }
  if (binding.profile === 'lcov-gap' && binding.collector === 'rust-lcov') {
    return {
        profile: 'lcov-gap',
        collector: 'rust-lcov',
        packageName: binding.packageName,
        testTarget: binding.testTarget,
    };
  }
  throw new Error('HARNESS_TASK_QE_BINDING_INVALID');
}

function cloneGeneratedOutput(binding: TaskGeneratedOutputBinding): TaskGeneratedOutputBinding {
  return {
    stage: binding.stage,
    evidenceId: binding.evidenceId,
    commandId: binding.commandId,
    workspacePaths: [...binding.workspacePaths],
  };
}

function cloneCommand(command: StructuredCommand): StructuredCommand {
  return { ...command, argv: [...command.argv], env: { ...command.env } };
}
