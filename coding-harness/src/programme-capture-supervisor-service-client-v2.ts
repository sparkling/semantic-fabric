// SPDX-License-Identifier: MIT

import {
  DEVELOPMENT_AUTHORITY,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  parseProgrammeCaptureSupervisorRegistrationRequestBlobV2,
  programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2,
} from './programme-capture-supervisor-registration-request-v2.js';
import {
  parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2,
} from './programme-capture-supervisor-run-event-codec-v2.js';
import {
  closedRunEventRecordV2,
  type ProgrammeCaptureSupervisorClaimRegisteredBodyV2,
  type ProgrammeCaptureSupervisorRunTerminalBodyV2,
} from './programme-capture-supervisor-run-event-contracts-v2.js';
import {
  verifyProgrammeCaptureSupervisorRunEventEnvelopeV2,
} from './programme-capture-supervisor-run-event-verifier-v2.js';
import {
  parseProgrammeCaptureSupervisorServiceResultBlobV2,
} from './programme-capture-supervisor-service-result-v2.js';
import { digestValue } from './receipts.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_VALIDATION_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-service-validation-digest-v2';

export function verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2(
  value: unknown,
) {
  const input = closedRunEventRecordV2(value, 'supervisor service-result verification input');
  assertExactKeys(input, [
    'serializedRequest', 'serializedResult', 'serializedAuthorityConfiguration',
    'activeAuthorityHeadDigest', 'activation', 'trustedServicePublicKeySpkiDer',
    'expectedRunId', 'expectedGlobalSequence', 'expectedPreviousGlobal',
    'expectedRunSequence', 'expectedPreviousRun',
    'expectedPriorControllerStateHeadDigest',
    'expectedOriginalRegistrationRequestDigest',
    'expectedOriginalRegistrationEventDigest',
  ], 'supervisor service-result verification input');
  if (typeof input.serializedRequest !== 'string' || typeof input.serializedResult !== 'string') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SERVICE_SERIALIZED_INPUT_REQUIRED');
  }
  const request = parseProgrammeCaptureSupervisorRegistrationRequestBlobV2(
    input.serializedRequest,
  );
  const result = parseProgrammeCaptureSupervisorServiceResultBlobV2(input.serializedResult);
  if (result.semanticRequestDigest !== request.semanticRequestDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESULT_REQUEST_BINDING_MISMATCH');
  }
  const envelope = parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2(
    result.serializedEventEnvelope,
  );
  const event = envelope.event;
  if (event.runId !== request.runId
    || event.project.projectAuthorityDigest !== request.project.projectAuthorityDigest
    || event.project.principalId !== request.project.principalId
    || event.authorityHead.configurationEpoch !== request.authorityHead.configurationEpoch
    || event.authorityHead.configurationDigest !== request.authorityHead.configurationDigest
    || event.authorityHead.headDigest !== request.authorityHead.headDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_REGISTRATION_INTENT_BINDING_MISMATCH');
  }
  const registrationOutcome = verifyRegistrationOutcome(
    request, event, input.expectedOriginalRegistrationRequestDigest,
    input.expectedOriginalRegistrationEventDigest,
  );
  const signatureValidation = verifyProgrammeCaptureSupervisorRunEventEnvelopeV2({
    serializedEnvelope: result.serializedEventEnvelope,
    serializedAuthorityConfiguration: input.serializedAuthorityConfiguration,
    activeAuthorityHeadDigest: input.activeAuthorityHeadDigest,
    activation: input.activation,
    trustedServicePublicKeySpkiDer: input.trustedServicePublicKeySpkiDer,
    expectedRunId: input.expectedRunId,
    expectedGlobalSequence: input.expectedGlobalSequence,
    expectedPreviousGlobal: input.expectedPreviousGlobal,
    expectedRunSequence: input.expectedRunSequence,
    expectedPreviousRun: input.expectedPreviousRun,
    expectedPriorControllerStateHeadDigest: input.expectedPriorControllerStateHeadDigest,
  });
  const body = {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    evidenceKind: 'non-authorizing-supervisor-service-result-validation-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    runId: event.runId,
    eventDigest: event.eventDigest,
    semanticRequestDigest: request.semanticRequestDigest,
    resultDigest: result.resultDigest,
    runEventValidationDigest: signatureValidation.validationDigest,
    registrationOutcome,
    responseStatus: result.responseStatus,
    responseContentType: result.responseContentType,
    verificationScope:
      'offline-registration-result-and-supplied-reference-binding-only' as const,
    canonicalRequestVerified: true as const,
    canonicalResultVerified: true as const,
    semanticRequestBindingVerified: true as const,
    registrationOutcomeBindingVerified: true as const,
    canonicalResponseMetadataBindingVerified: true as const,
    changedReplayEvidenceBindingVerified: registrationOutcome === 'changed-replay',
    serviceSignatureVerified: true as const,
    externalAdministrationVerified: false as const,
    deploymentAttestationVerified: false as const,
    authorityActivationVerified: false as const,
    fullAuthorityHistoryVerified: false as const,
    projectAuthenticationVerified: false as const,
    priorGlobalEventVerified: false as const,
    globalOrderVerified: false as const,
    priorSemanticReceiptVerified: false as const,
    controllerStateHeadVerified: false as const,
    rootedClaimVerified: false as const,
    runAdjacencyVerified: false as const,
    resourceAdjacencyVerified: false as const,
    resourceHighWaterVerified: false as const,
    runnerAdmissionVerified: false as const,
    hostEvidenceVerified: false as const,
    databaseCommitVerified: false as const,
    exactStoredResponseVerified: false as const,
    publicCommitmentVerified: false as const,
    checkpointWitnessQuorumVerified: false as const,
    semanticWitnessQuorumVerified: false as const,
    resourceFencingVerified: false as const,
    stateTransitionAuthorized: false as const,
    attemptStartAuthorized: false as const,
    captureAuthorized: false as const,
    importAuthorized: false as const,
    promotionAuthorized: false as const,
    releaseAuthorized: false as const,
  };
  return deepFreeze({
    ...body,
    validationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_VALIDATION_DIGEST_DOMAIN_V2,
      validation: body,
    }),
  });
}

function verifyRegistrationOutcome(
  request: ReturnType<typeof parseProgrammeCaptureSupervisorRegistrationRequestBlobV2>,
  event: ReturnType<typeof parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2>['event'],
  expectedOriginalRequestDigest: unknown,
  expectedOriginalEventDigest: unknown,
): 'accepted' | 'changed-replay' {
  if (event.eventKind === 'claim-registered-v2') {
    const claim = event.body as ProgrammeCaptureSupervisorClaimRegisteredBodyV2;
    if (expectedOriginalRequestDigest !== null || expectedOriginalEventDigest !== null
      || event.priorControllerStateHeadDigest
        !== request.expectedRegistration.priorControllerStateHeadDigest
      || claim.claimKeyDigest !== request.claim.claimKeyDigest
      || claim.claimDigest !== request.claim.claimDigest
      || claim.rootedClaimValidationDigest !== request.claim.rootedClaimValidationDigest) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_REGISTRATION_INTENT_BINDING_MISMATCH');
    }
    return 'accepted';
  }
  if (event.eventKind !== 'capture-run-terminal-v2'
    || typeof expectedOriginalRequestDigest !== 'string'
    || typeof expectedOriginalEventDigest !== 'string') {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_REGISTRATION_OUTCOME_INVALID');
  }
  const terminal = event.body as ProgrammeCaptureSupervisorRunTerminalBodyV2;
  if (expectedOriginalRequestDigest === request.semanticRequestDigest
    || event.runSequence !== '1'
    || event.previousRun.kind !== 'run-event'
    || event.previousRun.eventDigest !== expectedOriginalEventDigest
    || terminal.terminalStage !== 'registration'
    || terminal.outcomeCode !== 'registration-changed-replay-v2'
    || event.resourceTransition !== null
    || terminal.registrationEventDigest !== expectedOriginalEventDigest
    || terminal.outcomeEvidenceDigest
      !== programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2({
        originalRegistrationRequestDigest: expectedOriginalRequestDigest,
        originalRegistrationEventDigest: expectedOriginalEventDigest,
        changedRegistrationRequestDigest: request.semanticRequestDigest,
        project: request.project,
        authorityHead: request.authorityHead,
      })) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_CHANGED_REPLAY_BINDING_MISMATCH');
  }
  return 'changed-replay';
}
