// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  DEVELOPMENT_AUTHORITY,
  asClosedRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  programmeCaptureRunClaimKeyDigestV1,
} from './programme-capture-claim-key-v1.js';
import {
  parseProgrammeCaptureSupervisorAuthorityHeadRefV2,
} from './programme-capture-supervisor-run-event-body-v2.js';
import {
  parseRunEventDigestV2,
  parseRunEventOpaqueIdV2,
  type ProgrammeCaptureSupervisorAuthorityHeadRefV2,
} from './programme-capture-supervisor-run-event-contracts-v2.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_MAX_BYTES_V2 = 32_768;
export const PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-registration-request-digest-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_CHANGED_REPLAY_EVIDENCE_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-registration-changed-replay-evidence-v2';

const NON_AUTHORITY = Object.freeze({
  externalAdministrationVerified: false as const,
  deploymentAttestationVerified: false as const,
  authorityActivationVerified: false as const,
  projectAuthenticationVerified: false as const,
  serviceSignatureVerified: false as const,
  priorGlobalEventVerified: false as const,
  globalOrderVerified: false as const,
  priorSemanticReceiptVerified: false as const,
  controllerStateHeadVerified: false as const,
  rootedClaimVerified: false as const,
  runAdjacencyVerified: false as const,
  stateTransitionAuthorized: false as const,
  attemptStartAuthorized: false as const,
  captureAuthorized: false as const,
  importAuthorized: false as const,
  promotionAuthorized: false as const,
  releaseAuthorized: false as const,
});
const NON_AUTHORITY_KEYS = Object.freeze(Object.keys(NON_AUTHORITY));

export interface ProgrammeCaptureSupervisorRegistrationRequestInputV2 {
  readonly authorityHead: ProgrammeCaptureSupervisorAuthorityHeadRefV2;
  readonly project: Readonly<{
    projectAuthorityDigest: string;
    principalId: string;
  }>;
  readonly runId: string;
  readonly expectedRegistration: Readonly<{
    priorControllerStateHeadDigest: string;
  }>;
  readonly claim: Readonly<{
    claimKeyDigest: string;
    claimDigest: string;
    rootedClaimValidationDigest: string;
  }>;
}

export interface ProgrammeCaptureSupervisorRegistrationRequestV2
extends ProgrammeCaptureSupervisorRegistrationRequestInputV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly requestKind: 'supervisor-claim-registration-request-v2';
  readonly operationKind: 'claim-registered-v2';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly verificationScope: 'canonical-registration-intent-only';
  readonly externalAdministrationVerified: false;
  readonly deploymentAttestationVerified: false;
  readonly authorityActivationVerified: false;
  readonly projectAuthenticationVerified: false;
  readonly serviceSignatureVerified: false;
  readonly priorGlobalEventVerified: false;
  readonly globalOrderVerified: false;
  readonly priorSemanticReceiptVerified: false;
  readonly controllerStateHeadVerified: false;
  readonly rootedClaimVerified: false;
  readonly runAdjacencyVerified: false;
  readonly stateTransitionAuthorized: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
  readonly importAuthorized: false;
  readonly promotionAuthorized: false;
  readonly releaseAuthorized: false;
  readonly semanticRequestDigest: string;
}

export function buildProgrammeCaptureSupervisorRegistrationRequestV2(
  value: unknown,
): ProgrammeCaptureSupervisorRegistrationRequestV2 {
  const input = closed(value, 'supervisor registration request construction input');
  assertExactKeys(
    input, ['authorityHead', 'project', 'runId', 'expectedRegistration', 'claim'],
    'supervisor registration request construction input',
  );
  const body = normalizedRequestBody(input);
  return parseProgrammeCaptureSupervisorRegistrationRequestV2({
    ...body,
    semanticRequestDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_DIGEST_DOMAIN_V2,
      request: body,
    }),
  });
}

export function parseProgrammeCaptureSupervisorRegistrationRequestV2(
  value: unknown,
): ProgrammeCaptureSupervisorRegistrationRequestV2 {
  const input = closed(value, 'supervisor registration request');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'requestKind', 'operationKind', 'authority',
    'authorityHead', 'project', 'runId', 'expectedRegistration', 'claim',
    'verificationScope',
    ...NON_AUTHORITY_KEYS, 'semanticRequestDigest',
  ], 'supervisor registration request');
  assertRequestIdentity(input);
  const body = normalizedRequestBody(input);
  const semanticRequestDigest = parseRunEventDigestV2(
    input.semanticRequestDigest, 'registration semantic request digest',
  );
  if (semanticRequestDigest !== digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_DIGEST_DOMAIN_V2,
    request: body,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_REQUEST_DIGEST_MISMATCH');
  return deepFreeze({ ...body, semanticRequestDigest });
}

export function serializeProgrammeCaptureSupervisorRegistrationRequestV2(
  value: unknown,
): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorRegistrationRequestV2(value), null, 2)}\n`;
}

export function parseProgrammeCaptureSupervisorRegistrationRequestBlobV2(
  serialized: string,
): ProgrammeCaptureSupervisorRegistrationRequestV2 {
  assertCanonicalBlob(serialized);
  const request = parseProgrammeCaptureSupervisorRegistrationRequestV2(
    parseJsonWithoutDuplicateKeys(serialized, 'supervisor registration request'),
  );
  if (serializeProgrammeCaptureSupervisorRegistrationRequestV2(request) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_REQUEST_CANONICAL_REQUIRED');
  }
  return request;
}

export function programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2(
  value: unknown,
): string {
  const input = closed(value, 'registration changed-replay evidence input');
  assertExactKeys(input, [
    'originalRegistrationRequestDigest', 'originalRegistrationEventDigest',
    'changedRegistrationRequestDigest', 'project', 'authorityHead',
  ], 'registration changed-replay evidence input');
  const project = closed(input.project, 'registration changed-replay project');
  assertExactKeys(
    project, ['projectAuthorityDigest', 'principalId'],
    'registration changed-replay project',
  );
  return digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_CHANGED_REPLAY_EVIDENCE_DOMAIN_V2,
    originalRegistrationRequestDigest: parseRunEventDigestV2(
      input.originalRegistrationRequestDigest, 'original registration request digest',
    ),
    originalRegistrationEventDigest: parseRunEventDigestV2(
      input.originalRegistrationEventDigest, 'original registration event digest',
    ),
    changedRegistrationRequestDigest: parseRunEventDigestV2(
      input.changedRegistrationRequestDigest, 'changed registration request digest',
    ),
    project: {
      projectAuthorityDigest: parseRunEventDigestV2(
        project.projectAuthorityDigest, 'changed-replay project authority digest',
      ),
      principalId: parseRunEventOpaqueIdV2(
        project.principalId, 'changed-replay project principal ID',
      ),
    },
    authorityHead: parseProgrammeCaptureSupervisorAuthorityHeadRefV2(input.authorityHead),
  });
}

function normalizedRequestBody(input: Record<string, unknown>) {
  const project = closed(input.project, 'supervisor registration request project');
  assertExactKeys(
    project, ['projectAuthorityDigest', 'principalId'],
    'supervisor registration request project',
  );
  const runId = parseRunEventOpaqueIdV2(input.runId, 'registration request run ID');
  const expectedRegistration = closed(
    input.expectedRegistration, 'supervisor registration expected state',
  );
  assertExactKeys(
    expectedRegistration, ['priorControllerStateHeadDigest'],
    'supervisor registration expected state',
  );
  const projectAuthorityDigest = parseRunEventDigestV2(
    project.projectAuthorityDigest, 'registration project authority digest',
  );
  const claim = closed(input.claim, 'supervisor registration request claim');
  assertExactKeys(
    claim, ['claimKeyDigest', 'claimDigest', 'rootedClaimValidationDigest'],
    'supervisor registration request claim',
  );
  const claimKeyDigest = parseRunEventDigestV2(
    claim.claimKeyDigest, 'registration claim-key digest',
  );
  if (claimKeyDigest !== programmeCaptureRunClaimKeyDigestV1({
    projectAuthorityDigest, runId,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_CLAIM_KEY_MISMATCH');
  return {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    requestKind: 'supervisor-claim-registration-request-v2' as const,
    operationKind: 'claim-registered-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    authorityHead: parseProgrammeCaptureSupervisorAuthorityHeadRefV2(input.authorityHead),
    project: {
      projectAuthorityDigest,
      principalId: parseRunEventOpaqueIdV2(
        project.principalId, 'registration project principal ID',
      ),
    },
    runId,
    expectedRegistration: {
      priorControllerStateHeadDigest: parseRunEventDigestV2(
        expectedRegistration.priorControllerStateHeadDigest,
        'expected registration controller state-head digest',
      ),
    },
    claim: {
      claimKeyDigest,
      claimDigest: parseRunEventDigestV2(claim.claimDigest, 'registration claim digest'),
      rootedClaimValidationDigest: parseRunEventDigestV2(
        claim.rootedClaimValidationDigest, 'registration rooted-claim validation digest',
      ),
    },
    verificationScope: 'canonical-registration-intent-only' as const,
    ...NON_AUTHORITY,
  };
}

function assertRequestIdentity(input: Record<string, unknown>): void {
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.requestKind !== 'supervisor-claim-registration-request-v2'
    || input.operationKind !== 'claim-registered-v2'
    || input.authority !== DEVELOPMENT_AUTHORITY
    || input.verificationScope !== 'canonical-registration-intent-only') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_REQUEST_IDENTITY_INVALID');
  }
  if (NON_AUTHORITY_KEYS.some(
    (key) => input[key] !== NON_AUTHORITY[key as keyof typeof NON_AUTHORITY],
  )) throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_REQUEST_AUTHORITY_ESCALATION');
}

function closed(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

function assertCanonicalBlob(serialized: string): void {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8')
      > PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_MAX_BYTES_V2
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_REQUEST_CANONICAL_INVALID');
  }
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}
