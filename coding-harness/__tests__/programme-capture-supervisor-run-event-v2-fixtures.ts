// SPDX-License-Identifier: MIT

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign,
} from 'node:crypto';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
  parseProgrammeCaptureSupervisorAuthorityConfigurationV2,
  programmeCaptureSupervisorAuthorityGenesisHeadDigestV2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RESOURCE_CONFLICT_SET_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
  type ProgrammeCaptureSupervisorRunEventKindV2,
  type ProgrammeCaptureSupervisorRunEventV2,
} from '../src/programme-capture-supervisor-run-event-contracts-v2.js';
import {
  parseProgrammeCaptureSupervisorRunEventV2,
  programmeCaptureSupervisorRunEventSigningPayloadV2,
  serializeProgrammeCaptureSupervisorRunEventEnvelopeV2,
} from '../src/programme-capture-supervisor-run-event-codec-v2.js';
import { digestValue } from '../src/receipts.js';

const TEST_SEED = Buffer.from(
  '9d61b19deffd5a60ba844af492ec2cc4'
  + '4449c5697b326919703bac031cae7f60',
  'hex',
);
const TEST_PRIVATE_KEY = createPrivateKey({
  key: Buffer.concat([
    Buffer.from('302e020100300506032b657004220420', 'hex'), TEST_SEED,
  ]),
  format: 'der',
  type: 'pkcs8',
});
export const TEST_SERVICE_PUBLIC_KEY_SPKI = Buffer.from(
  createPublicKey(TEST_PRIVATE_KEY).export({ format: 'der', type: 'spki' }),
);
export const TEST_SERVICE_KEY_FINGERPRINT = sha256(TEST_SERVICE_PUBLIC_KEY_SPKI);

const RUN_ID = 'capture_run_20260829';
const SERVICE_ID = 'supervisor_service_20260829';
const PROJECT_ID = 'project_client_20260829';
const RUNNER_ID = 'controlled_runner_20260829';
const SESSION_ID = 'runner_session_20260829';
const BOOT_ID = 'runner_boot_20260829';
const LEASE_ID = 'single_use_lease_20260829';
const ATTEMPT_ID = 'capture_attempt_20260829';
const ENROLLMENT_DIGEST = digest('runner-enrollment-record');

export interface ValidRunHistoryOptions {
  readonly failure?: boolean;
  readonly includeFinal?: boolean;
  readonly preStartTerminal?: 'registration' | 'pre-lease' | 'leased-pre-start';
}

export function validRunHistory(
  options: ValidRunHistoryOptions = {},
): ProgrammeCaptureSupervisorRunEventV2[] {
  const configuration = validAuthorityConfiguration();
  const authorityHead = {
    configurationEpoch: configuration.configurationEpoch,
    configurationDigest: configuration.configurationDigest,
    headDigest: programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(configuration),
  };
  const common = {
    authorityHead,
    service: {
      principalId: SERVICE_ID,
      keyEpoch: '1',
      keyFingerprint: TEST_SERVICE_KEY_FINGERPRINT,
    },
    project: {
      projectAuthorityDigest: configuration.project.projectAuthorityDigest,
      principalId: PROJECT_ID,
    },
    runId: RUN_ID,
  };
  let stateHead = digest('controller-state-genesis');
  const events: ProgrammeCaptureSupervisorRunEventV2[] = [];
  const append = (
    eventKind: ProgrammeCaptureSupervisorRunEventKindV2,
    body: Record<string, unknown>,
    resourceTransition: Record<string, unknown> | null,
  ) => {
    const previous = events.at(-1);
    const eventBody = {
      schemaVersion: 2,
      transactionKind: 'programme-capture-v2',
      recordKind: 'supervisor-run-event-v2',
      authority: 'development-only-no-promotion',
      eventKind,
      ...common,
      semanticRequestDigest: digest(`request:${events.length}:${eventKind}`),
      globalSequence: String(events.length + 1),
      runSequence: String(events.length),
      previousGlobal: previous ? {
        kind: 'semantic-event',
        eventDigest: previous.eventDigest,
        semanticReceiptDigest: digest(`semantic-receipt:${events.length}`),
      } : {
        kind: 'authority-genesis',
        eventDigest: null,
        semanticReceiptDigest: digest('semantic-genesis'),
      },
      previousRun: previous ? {
        kind: 'run-event', eventDigest: previous.eventDigest,
      } : { kind: 'run-genesis', eventDigest: null },
      priorControllerStateHeadDigest: stateHead,
      resourceTransition,
      body,
      verificationScope: 'service-signed-structure-only',
      ...nonAuthority(),
    };
    const event = parseProgrammeCaptureSupervisorRunEventV2({
      ...eventBody,
      eventDigest: digestValue({
        domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
        event: eventBody,
      }),
    });
    events.push(event);
    stateHead = digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2,
      priorControllerStateHeadDigest: stateHead,
      eventDigest: event.eventDigest,
    });
    return event;
  };

  const registration = append('claim-registered-v2', {
    claimKeyDigest: digest('claim-key'),
    claimDigest: digest('claim'),
    rootedClaimValidationDigest: digest('rooted-claim-validation'),
  }, null);
  if (options.preStartTerminal === 'registration'
    || options.preStartTerminal === 'pre-lease') {
    append('capture-run-terminal-v2', runTerminalBody(
      options.preStartTerminal, registration.eventDigest,
    ), null);
    return events;
  }

  const leaseResource = resourceTransition('1', null, null);
  const lease = append('runner-lease-granted-v2', leaseBody(registration), leaseResource);
  if (options.preStartTerminal === 'leased-pre-start') {
    append('capture-run-terminal-v2', runTerminalBody(
      'leased-pre-start', registration.eventDigest, lease.eventDigest,
    ), resourceTransition('1', lease.eventDigest, '1'));
    return events;
  }

  const start = append(
    'capture-attempt-start-committed-v2',
    startBody(lease, leaseResource.conflictSetDigest as string),
    resourceTransition('1', lease.eventDigest, '1'),
  );
  const terminal = append(
    'capture-attempt-terminal-v2',
    terminalBody(start, lease, options.failure === true),
    resourceTransition('1', start.eventDigest, '1'),
  );
  if (options.failure !== true || options.includeFinal === true) {
    append('capture-final-witness-v2', finalBody(terminal, lease), null);
  }
  return events;
}

export function signedEnvelope(event: unknown): string {
  const parsed = parseProgrammeCaptureSupervisorRunEventV2(event);
  const signature = sign(
    null, programmeCaptureSupervisorRunEventSigningPayloadV2(parsed), TEST_PRIVATE_KEY,
  ).toString('base64url');
  return serializeProgrammeCaptureSupervisorRunEventEnvelopeV2({
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    envelopeKind: 'supervisor-run-event-envelope-v2',
    event: parsed,
    signature: { algorithm: 'ed25519', valueBase64Url: signature },
  });
}

export function validAuthorityConfiguration() {
  const body = {
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    recordKind: 'supervisor-authority-configuration-v2',
    authority: 'development-only-no-promotion',
    configurationEpoch: '0',
    predecessor: { kind: 'genesis', configurationDigest: null, headDigest: null },
    project: {
      projectAuthorityDigest: digest('project-authority'),
      principal: principal(PROJECT_ID),
      authenticationPolicyDigest: digest('project-authentication-policy'),
    },
    service: {
      principal: principal(SERVICE_ID, TEST_SERVICE_KEY_FINGERPRINT),
      endpointOrigin: 'https://supervisor.example.org',
      tlsSpkiFingerprint: digest('supervisor-tls-spki'),
      clientPolicyDigest: digest('supervisor-client-policy'),
    },
    transparencyLog: {
      principal: principal('transparency_log_20260829'),
      endpointOrigin: 'https://log.example.org',
      tlsSpkiFingerprint: digest('log-tls-spki'),
      publicCommitmentPolicyDigest: digest('public-commitment-policy'),
    },
    checkpointWitnesses: witnessPolicy('checkpoint'),
    semanticWitnesses: witnessPolicy('semantic'),
    initializationAnchor: principal('initialization_anchor_20260829'),
    runnerEnrollment: principal('runner_enrollment_20260829'),
    deploymentAttestor: principal('deployment_attestor_20260829'),
    readinessPolicyDigest: digest('readiness-policy'),
    verificationScope: 'trust-pins-and-quorum-math-only',
    externalAdministrationVerified: false,
    deploymentAttestationVerified: false,
    checkpointWitnessQuorumVerified: false,
    semanticWitnessQuorumVerified: false,
    stateTransitionAuthorized: false,
    attemptStartAuthorized: false,
    captureAuthorized: false,
  };
  return parseProgrammeCaptureSupervisorAuthorityConfigurationV2({
    ...body,
    configurationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
      configuration: body,
    }),
  });
}

export function withEventDigest(value: Record<string, unknown>) {
  const { eventDigest: _ignored, ...body } = value;
  return {
    ...body,
    eventDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
      event: body,
    }),
  };
}

export function digest(value: string): string {
  return sha256(Buffer.from(value));
}

function leaseBody(registration: ProgrammeCaptureSupervisorRunEventV2) {
  return {
    registrationEventDigest: registration.eventDigest,
    claimDigest: digest('claim'),
    admissionChallengeDigest: digest('admission-challenge'),
    admissionEvidenceDigest: digest('admission-evidence'),
    runner: {
      runnerId: RUNNER_ID,
      enrollmentRecordDigest: ENROLLMENT_DIGEST,
      sessionId: SESSION_ID,
      bootId: BOOT_ID,
      keyEpoch: '1',
      keyFingerprint: digest('runner-key'),
      possessionProofDigest: digest('runner-possession-proof'),
    },
    hostEvidenceDigest: digest('positive-host-evidence'),
    runnerProfileDigest: digest('runner-profile'),
    controlPolicyDigest: digest('control-policy'),
    preReview: {
      codexReceiptDigest: digest('codex-pre-review'),
      claudeReceiptDigest: digest('claude-pre-review'),
    },
    lease: {
      leaseId: LEASE_ID,
      fence: '1',
      serviceIssuedAt: '2026-08-29T12:00:00.000Z',
      notAfter: '2026-08-29T12:05:00.000Z',
      maxAttempts: 1,
      renew: false,
      releaseForReuse: false,
      reassign: false,
      reclaim: false,
      retry: false,
    },
  };
}

function startBody(lease: ProgrammeCaptureSupervisorRunEventV2, conflictSetDigest: string) {
  return {
    leaseEventDigest: lease.eventDigest,
    leaseId: LEASE_ID,
    fence: '1',
    runner: { runnerId: RUNNER_ID, sessionId: SESSION_ID, bootId: BOOT_ID },
    resourceConflictSetDigest: conflictSetDigest,
    quiescenceDigest: digest('quiescence'),
    freshHostPreflightDigest: digest('fresh-host-preflight'),
    heldSourceDigest: digest('held-source'),
    producerAgreementDigest: digest('producer-agreement'),
    producerArtifactDigest: digest('producer-artifact'),
    producerRuntimeClosureDigest: digest('producer-runtime-closure'),
    commandDigest: digest('command'),
    environmentDigest: digest('environment'),
    outputSlotDigest: digest('output-slot'),
    captureNonceDigest: digest('capture-nonce'),
    attemptId: ATTEMPT_ID,
  };
}

function terminalBody(
  start: ProgrammeCaptureSupervisorRunEventV2,
  lease: ProgrammeCaptureSupervisorRunEventV2,
  failure: boolean,
) {
  return {
    startEventDigest: start.eventDigest,
    leaseEventDigest: lease.eventDigest,
    leaseId: LEASE_ID,
    fence: '1',
    attemptId: ATTEMPT_ID,
    outcomeCode: failure ? 'process-failed-v2' : 'capture-candidate-complete-v2',
    outcomeEvidenceDigest: digest('attempt-outcome'),
    processDisposition: {
      kind: failure ? 'exited-nonzero' : 'exited-zero',
      evidenceDigest: digest('process-disposition'),
    },
    egressDisposition: {
      kind: 'isolated-no-violation', evidenceDigest: digest('egress-disposition'),
    },
    leaseDisposition: {
      kind: 'spent-never-reusable', evidenceDigest: digest('lease-disposition'),
    },
    resourceDisposition: {
      kind: 'released-after-cleanup', evidenceDigest: digest('resource-disposition'),
    },
    cleanup: {
      processCleanupDigest: digest('process-cleanup'),
      egressCleanupDigest: digest('egress-cleanup'),
      resourceCleanupDigest: digest('resource-cleanup'),
    },
    outputEnvelopeDigest: digest('output-envelope'),
    captureRecordDigest: digest('capture-record'),
    finalStateDigest: digest('final-state'),
  };
}

function finalBody(
  terminal: ProgrammeCaptureSupervisorRunEventV2,
  lease: ProgrammeCaptureSupervisorRunEventV2,
) {
  return {
    attemptTerminalEventDigest: terminal.eventDigest,
    leaseEventDigest: lease.eventDigest,
    leaseId: LEASE_ID,
    fence: '1',
    attemptId: ATTEMPT_ID,
    frozenEnvelopeDigest: digest('output-envelope'),
    captureRecordDigest: digest('capture-record'),
    finalStateDigest: digest('final-state'),
    replayValidationDigest: digest('replay-validation'),
    postReview: {
      codexReceiptDigest: digest('codex-post-review'),
      claudeReceiptDigest: digest('claude-post-review'),
    },
  };
}

function runTerminalBody(stage: ValidRunHistoryOptions['preStartTerminal'], registration: string,
  leaseEventDigest: string | null = null) {
  const leased = stage === 'leased-pre-start';
  return {
    terminalStage: stage,
    outcomeCode: stage === 'registration' ? 'registration-authenticated-denial-v2'
      : stage === 'pre-lease' ? 'pre-lease-admission-failed-v2'
        : 'leased-pre-start-expired-v2',
    registrationEventDigest: registration,
    outcomeEvidenceDigest: digest(`terminal:${stage}`),
    leaseEventDigest: leased ? leaseEventDigest : null,
    leaseId: leased ? LEASE_ID : null,
    fence: leased ? '1' : null,
    resourceDisposition: leased ? {
      kind: 'released-unstarted', evidenceDigest: digest('released-unstarted'),
    } : null,
    attemptId: null,
    captureRecordDigest: null,
    outputEnvelopeDigest: null,
    cleanupEvidenceDigest: null,
  };
}

function resourceTransition(fence: string, eventDigest: string | null, priorFence: string | null) {
  const resourceIds = ['numa_parent_20260829', 'resource_cpu_20260829'];
  const conflictSetDigest = digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_RESOURCE_CONFLICT_SET_DOMAIN_V2,
    runnerEnrollmentRecordDigest: ENROLLMENT_DIGEST,
    physicalParentId: resourceIds[0],
    resourceIds,
  });
  return {
    runnerEnrollmentRecordDigest: ENROLLMENT_DIGEST,
    physicalParentId: resourceIds[0],
    conflictSetDigest,
    fence,
    members: resourceIds.map((resourceId) => ({
      resourceId,
      priorState: eventDigest === null
        ? { kind: 'resource-genesis', eventDigest: null, fence: null }
        : { kind: 'resource-event', eventDigest, fence: priorFence },
    })),
  };
}

function nonAuthority() {
  return {
    externalAdministrationVerified: false,
    deploymentAttestationVerified: false,
    authorityActivationVerified: false,
    serviceSignatureVerified: false,
    priorGlobalEventVerified: false,
    priorSemanticReceiptVerified: false,
    controllerStateHeadVerified: false,
    rootedClaimVerified: false,
    runAdjacencyVerified: false,
    resourceHighWaterVerified: false,
    resourceFencingVerified: false,
    publicCommitmentVerified: false,
    checkpointWitnessQuorumVerified: false,
    semanticWitnessQuorumVerified: false,
    stateTransitionAuthorized: false,
    attemptStartAuthorized: false,
    captureAuthorized: false,
    importAuthorized: false,
    promotionAuthorized: false,
    releaseAuthorized: false,
  };
}

function witnessPolicy(prefix: string) {
  return {
    policyId: `${prefix}_witness_policy_20260829`,
    faultThreshold: '1',
    quorumThreshold: '3',
    members: Array.from({ length: 4 }, (_, index) =>
      principal(`${prefix}_witness_${String(index).padStart(2, '0')}_20260829`)),
  };
}

function principal(principalId: string, keyFingerprint = digest(`${principalId}:key`)) {
  return {
    principalId,
    keyEpoch: '1',
    keyFingerprint,
    policyDigest: digest(`${principalId}:policy`),
    administrationDigest: digest(`${principalId}:administration`),
  };
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
