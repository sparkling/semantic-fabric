// SPDX-License-Identifier: MIT

import {
  POSTGRES_PROJECT_SCOPE_ROLE_V1,
  deepFreezeV1,
  digestBytesFromHexV1,
  rawSha256HexV1,
  successorUint64V1,
  utf8BytesV1,
} from './registration-postgresql-canonical-v1.js';
import type {
  NormalizedMaterializationInputsV1,
} from './registration-postgresql-materializer-contract-v1.js';

export interface FinalizedPostgresMaterializationArtifactsV1 {
  readonly eventDigest: string;
  readonly serializedEnvelope: string;
  readonly resultingControllerStateHeadDigest: string;
  readonly serializedResponse: string;
  readonly publicCommitmentLeafBytes: Uint8Array;
}

export interface PostgresSemanticEventRowV1 {
  readonly projectAuthorityDigest: Uint8Array;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly eventDigest: Uint8Array;
  readonly eventKind: 'claim-registered-v2' | 'capture-run-terminal-v2';
  readonly semanticRequestDigest: Uint8Array;
  readonly runId: string;
  readonly authorityConfigurationEpoch: string;
  readonly authorityConfigurationDigest: Uint8Array;
  readonly authorityHeadDigest: Uint8Array;
  readonly globalSequence: string;
  readonly runSequence: '0' | '1';
  readonly previousGlobalKind: 'authority-genesis' | 'semantic-event';
  readonly previousGlobalSequence: string | null;
  readonly previousGlobalEventDigest: Uint8Array | null;
  readonly previousGlobalGenesisConfigurationEpoch: string | null;
  readonly previousGlobalGenesisConfigurationDigest: Uint8Array | null;
  readonly previousGlobalGenesisReceiptDigest: Uint8Array | null;
  readonly previousGlobalEventReceiptDigest: Uint8Array | null;
  readonly previousRunKind: 'run-genesis' | 'run-event';
  readonly previousRunSequence: string | null;
  readonly previousRunGlobalSequence: string | null;
  readonly previousRunEventDigest: Uint8Array | null;
  readonly priorControllerStateHeadDigest: Uint8Array;
  readonly resultingControllerStateHeadDigest: Uint8Array;
  readonly serializedEnvelope: Uint8Array;
  readonly serializedEnvelopeSha256: Uint8Array;
}

interface PostgresRegistrationRunRowBaseV1 {
  readonly projectAuthorityDigest: Uint8Array;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly runId: string;
  readonly originalRegistrationRequestDigest: Uint8Array;
  readonly originalRegistrationRequestSha256: Uint8Array;
  readonly originalRegistrationEventDigest: Uint8Array;
  readonly lastRunEventDigest: Uint8Array;
  readonly lastRunGlobalSequence: string;
  readonly currentControllerStateHeadDigest: Uint8Array;
}

export type PostgresRegistrationRunOpenRowV1 = Readonly<
  PostgresRegistrationRunRowBaseV1 & {
    lastRunSequence: '0'; firstChangedReplayRequestDigest: null;
  }
>;

export type PostgresRegistrationRunRowV1 = Readonly<
  | PostgresRegistrationRunOpenRowV1
  | (PostgresRegistrationRunRowBaseV1 & {
    lastRunSequence: '1'; firstChangedReplayRequestDigest: Uint8Array;
  })
>;

export interface PostgresRegistrationResultRowV1 {
  readonly projectAuthorityDigest: Uint8Array;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly semanticRequestDigest: Uint8Array;
  readonly runId: string;
  readonly originalRegistrationRequestDigest: Uint8Array;
  readonly originalRegistrationRequestSha256: Uint8Array;
  readonly originalRegistrationEventDigest: Uint8Array;
  readonly serializedRequest: Uint8Array;
  readonly serializedRequestSha256: Uint8Array;
  readonly responseStatus: 201 | 409;
  readonly responseContentType: 'application/json; charset=utf-8';
  readonly serializedResponse: Uint8Array;
  readonly serializedResponseSha256: Uint8Array;
  readonly currentEventDigest: Uint8Array;
}

export interface PostgresPublicationOutboxRowV1 {
  readonly projectAuthorityDigest: Uint8Array;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly eventDigest: Uint8Array;
  readonly publicCommitmentLeafBytes: Uint8Array;
  readonly publicCommitmentDigest: Uint8Array;
  readonly publicationState: 'pending';
}

export interface PostgresAuthorityStateRowV1 {
  readonly projectAuthorityDigest: Uint8Array;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly singletonKey: true;
  readonly activeConfigurationEpoch: string;
  readonly activeConfigurationDigest: Uint8Array;
  readonly authorityHeadDigest: Uint8Array;
  readonly lastGlobalSequence: string;
  readonly nextGlobalSequence: string;
  readonly lastEventDigest: Uint8Array | null;
}

export type PostgresRegistrationRunExpectedOldV1 = Readonly<
  | {
    kind: 'absent'; projectAuthorityDigest: Uint8Array;
    projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1; runId: string;
  }
  | ({ kind: 'registered' } & PostgresRegistrationRunOpenRowV1)
>;

export interface FinalizedPostgresRegistrationMaterializationV1 {
  readonly response: Readonly<{
    status: 201 | 409;
    contentType: 'application/json; charset=utf-8';
    body: string;
  }>;
  readonly semanticEventRow: PostgresSemanticEventRowV1;
  readonly registrationResultRow: PostgresRegistrationResultRowV1;
  readonly registrationRunMutation: Readonly<{
    kind: 'insert' | 'update';
    expectedOld: PostgresRegistrationRunExpectedOldV1;
    resulting: PostgresRegistrationRunRowV1;
  }>;
  readonly publicationOutboxRow: PostgresPublicationOutboxRowV1;
  readonly authorityStateMutation: Readonly<{
    expectedOld: PostgresAuthorityStateRowV1;
    resulting: PostgresAuthorityStateRowV1;
  }>;
}

export function buildPostgresMaterializationRowsV1(
  inputs: NormalizedMaterializationInputsV1,
  artifacts: FinalizedPostgresMaterializationArtifactsV1,
): FinalizedPostgresRegistrationMaterializationV1 {
  const { candidate, configuration, authorityState, predecessorReceipt, runState } = inputs;
  const original = candidate.status === 201 ? {
    requestDigest: candidate.request.semanticRequestDigest,
    requestSha256: candidate.request.serializedSha256,
    eventDigest: artifacts.eventDigest,
  } : registeredOriginal(runState);
  const globalGenesis = authorityState.lastGlobalSequence === '0';
  const previousRun = candidate.status === 201 ? Object.freeze({
    sequence: null, globalSequence: null, eventDigest: null,
  }) : registeredPredecessor(runState);
  const serializedEnvelopeSha256 = rawSha256HexV1(artifacts.serializedEnvelope);
  const serializedResponseSha256 = rawSha256HexV1(artifacts.serializedResponse);
  const semanticEventRow: PostgresSemanticEventRowV1 = deepFreezeV1({
    projectAuthorityDigest: digest(configuration.projectAuthorityDigest),
    projectScopeRole: POSTGRES_PROJECT_SCOPE_ROLE_V1,
    eventDigest: digest(artifacts.eventDigest),
    eventKind: candidate.candidateKind,
    semanticRequestDigest: digest(candidate.request.semanticRequestDigest),
    runId: candidate.request.runId,
    authorityConfigurationEpoch: configuration.configurationEpoch,
    authorityConfigurationDigest: digest(configuration.configurationDigest),
    authorityHeadDigest: digest(configuration.genesisAuthorityHeadDigest),
    globalSequence: candidate.globalSequence,
    runSequence: candidate.runSequence,
    previousGlobalKind: candidate.previousGlobal.kind,
    previousGlobalSequence: globalGenesis ? null : authorityState.lastGlobalSequence,
    previousGlobalEventDigest: globalGenesis ? null : digest(authorityState.lastEventDigest!),
    previousGlobalGenesisConfigurationEpoch: globalGenesis
      ? configuration.configurationEpoch : null,
    previousGlobalGenesisConfigurationDigest: globalGenesis
      ? digest(configuration.configurationDigest) : null,
    previousGlobalGenesisReceiptDigest: globalGenesis
      ? digest(predecessorReceipt.semanticReceiptDigest) : null,
    previousGlobalEventReceiptDigest: globalGenesis
      ? null : digest(predecessorReceipt.semanticReceiptDigest),
    previousRunKind: candidate.previousRun.kind,
    previousRunSequence: previousRun.sequence,
    previousRunGlobalSequence: previousRun.globalSequence,
    previousRunEventDigest: previousRun.eventDigest === null
      ? null : digest(previousRun.eventDigest),
    priorControllerStateHeadDigest: digest(candidate.priorControllerStateHeadDigest),
    resultingControllerStateHeadDigest: digest(
      artifacts.resultingControllerStateHeadDigest,
    ),
    serializedEnvelope: utf8BytesV1(
      artifacts.serializedEnvelope, 'serialized event envelope', 65_536,
    ),
    serializedEnvelopeSha256: digest(serializedEnvelopeSha256),
  });
  const resultingRun = buildResultingRun(inputs, artifacts, original);
  return deepFreezeV1({
    response: Object.freeze({
      status: candidate.status,
      contentType: 'application/json; charset=utf-8' as const,
      body: artifacts.serializedResponse,
    }),
    semanticEventRow,
    registrationResultRow: {
      projectAuthorityDigest: digest(configuration.projectAuthorityDigest),
      projectScopeRole: POSTGRES_PROJECT_SCOPE_ROLE_V1,
      semanticRequestDigest: digest(candidate.request.semanticRequestDigest),
      runId: candidate.request.runId,
      originalRegistrationRequestDigest: digest(original.requestDigest),
      originalRegistrationRequestSha256: digest(original.requestSha256),
      originalRegistrationEventDigest: digest(original.eventDigest),
      serializedRequest: utf8BytesV1(
        candidate.request.serialized, 'serialized registration request', 32_768,
      ),
      serializedRequestSha256: digest(candidate.request.serializedSha256),
      responseStatus: candidate.status,
      responseContentType: 'application/json; charset=utf-8' as const,
      serializedResponse: utf8BytesV1(
        artifacts.serializedResponse, 'serialized registration response', 196_608,
      ),
      serializedResponseSha256: digest(serializedResponseSha256),
      currentEventDigest: digest(artifacts.eventDigest),
    },
    registrationRunMutation: {
      kind: candidate.status === 201 ? 'insert' as const : 'update' as const,
      expectedOld: expectedOldRun(inputs),
      resulting: resultingRun,
    },
    publicationOutboxRow: {
      projectAuthorityDigest: digest(configuration.projectAuthorityDigest),
      projectScopeRole: POSTGRES_PROJECT_SCOPE_ROLE_V1,
      eventDigest: digest(artifacts.eventDigest),
      publicCommitmentLeafBytes: copyBytes(artifacts.publicCommitmentLeafBytes),
      publicCommitmentDigest: digest(rawSha256HexV1(artifacts.publicCommitmentLeafBytes)),
      publicationState: 'pending' as const,
    },
    authorityStateMutation: {
      expectedOld: authorityRow(inputs, false, artifacts.eventDigest),
      resulting: authorityRow(inputs, true, artifacts.eventDigest),
    },
  });
}

function buildResultingRun(
  inputs: NormalizedMaterializationInputsV1,
  artifacts: FinalizedPostgresMaterializationArtifactsV1,
  original: Readonly<{ requestDigest: string; requestSha256: string; eventDigest: string }>,
): PostgresRegistrationRunRowV1 {
  const { candidate, configuration } = inputs;
  const common: PostgresRegistrationRunRowBaseV1 = deepFreezeV1({
    projectAuthorityDigest: digest(configuration.projectAuthorityDigest),
    projectScopeRole: POSTGRES_PROJECT_SCOPE_ROLE_V1,
    runId: candidate.request.runId,
    originalRegistrationRequestDigest: digest(original.requestDigest),
    originalRegistrationRequestSha256: digest(original.requestSha256),
    originalRegistrationEventDigest: digest(original.eventDigest),
    lastRunEventDigest: digest(artifacts.eventDigest),
    lastRunGlobalSequence: candidate.globalSequence,
    currentControllerStateHeadDigest: digest(
      artifacts.resultingControllerStateHeadDigest,
    ),
  });
  return candidate.status === 201
    ? deepFreezeV1({ ...common, lastRunSequence: '0' as const,
      firstChangedReplayRequestDigest: null })
    : deepFreezeV1({ ...common, lastRunSequence: '1' as const,
      firstChangedReplayRequestDigest: digest(candidate.request.semanticRequestDigest) });
}

function expectedOldRun(
  inputs: NormalizedMaterializationInputsV1,
): PostgresRegistrationRunExpectedOldV1 {
  const run = inputs.runState;
  if (run.kind === 'absent') return deepFreezeV1({
    kind: 'absent' as const,
    projectAuthorityDigest: digest(run.projectAuthorityDigest),
    projectScopeRole: POSTGRES_PROJECT_SCOPE_ROLE_V1,
    runId: run.runId,
  });
  return deepFreezeV1({
    kind: 'registered' as const,
    projectAuthorityDigest: digest(run.projectAuthorityDigest),
    projectScopeRole: POSTGRES_PROJECT_SCOPE_ROLE_V1,
    runId: run.runId,
    originalRegistrationRequestDigest: digest(run.originalRegistrationRequestDigest),
    originalRegistrationRequestSha256: digest(run.originalRegistrationRequestSha256),
    originalRegistrationEventDigest: digest(run.originalRegistrationEventDigest),
    lastRunEventDigest: digest(run.lastRunEventDigest),
    lastRunGlobalSequence: run.lastRunGlobalSequence,
    currentControllerStateHeadDigest: digest(run.currentControllerStateHeadDigest),
    lastRunSequence: run.lastRunSequence,
    firstChangedReplayRequestDigest: null,
  });
}

function authorityRow(
  inputs: NormalizedMaterializationInputsV1, resulting: boolean, eventDigest: string,
): PostgresAuthorityStateRowV1 {
  const state = inputs.authorityState;
  return deepFreezeV1({
    projectAuthorityDigest: digest(state.projectAuthorityDigest),
    projectScopeRole: POSTGRES_PROJECT_SCOPE_ROLE_V1,
    singletonKey: true as const,
    activeConfigurationEpoch: state.activeConfigurationEpoch,
    activeConfigurationDigest: digest(state.activeConfigurationDigest),
    authorityHeadDigest: digest(state.authorityHeadDigest),
    lastGlobalSequence: resulting ? inputs.candidate.globalSequence : state.lastGlobalSequence,
    nextGlobalSequence: resulting
      ? successorUint64V1(inputs.candidate.globalSequence, 'resulting global sequence')
      : state.nextGlobalSequence,
    lastEventDigest: resulting ? digest(eventDigest)
      : state.lastEventDigest === null ? null : digest(state.lastEventDigest),
  });
}

function registeredOriginal(run: NormalizedMaterializationInputsV1['runState']) {
  if (run.kind !== 'registered') throw new TypeError('registered run snapshot required');
  return Object.freeze({
    requestDigest: run.originalRegistrationRequestDigest,
    requestSha256: run.originalRegistrationRequestSha256,
    eventDigest: run.originalRegistrationEventDigest,
  });
}

function registeredPredecessor(run: NormalizedMaterializationInputsV1['runState']) {
  if (run.kind !== 'registered') throw new TypeError('registered run snapshot required');
  return Object.freeze({
    sequence: run.lastRunSequence,
    globalSequence: run.lastRunGlobalSequence,
    eventDigest: run.lastRunEventDigest,
  });
}

function digest(value: string): Uint8Array {
  return digestBytesFromHexV1(value, 'PostgreSQL row digest');
}

function copyBytes(value: Uint8Array): Uint8Array {
  return new Uint8Array(value);
}
