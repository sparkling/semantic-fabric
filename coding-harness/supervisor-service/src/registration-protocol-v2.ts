// SPDX-License-Identifier: MIT

import {
  asRecord,
  assertExactKeys,
  cloneClosedRecord,
  deepFreeze,
  parseCanonicalPrettyJson,
  parseDigest,
  parseOpaqueId,
  parseUint64,
  sha256CanonicalValue,
  sha256Text,
} from './closed-json.js';
import {
  validateStoredRegistrationEventEnvelopeV2,
  type StoredRegistrationEventExpectationV2,
} from './registration-event-envelope-v2.js';

export const REGISTRATION_REQUEST_MAX_BYTES_V2 = 32_768;
export const REGISTRATION_RESULT_MAX_BYTES_V2 = 196_608;
export const REGISTRATION_CONTENT_TYPE_V2 = 'application/json; charset=utf-8' as const;
export const REGISTRATION_REQUEST_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-registration-request-digest-v2';
export const REGISTRATION_CHANGED_REPLAY_EVIDENCE_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-registration-changed-replay-evidence-v2';
const REGISTRATION_RESULT_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-service-result-digest-v2';

const NON_AUTHORITY_KEYS = Object.freeze([
  'externalAdministrationVerified', 'deploymentAttestationVerified',
  'authorityActivationVerified', 'projectAuthenticationVerified',
  'serviceSignatureVerified', 'priorGlobalEventVerified', 'globalOrderVerified',
  'priorSemanticReceiptVerified', 'controllerStateHeadVerified',
  'rootedClaimVerified', 'runAdjacencyVerified', 'stateTransitionAuthorized',
  'attemptStartAuthorized', 'captureAuthorized', 'importAuthorized',
  'promotionAuthorized', 'releaseAuthorized',
]);
const RESULT_NON_AUTHORITY_KEYS = Object.freeze([
  'externalAdministrationVerified', 'deploymentAttestationVerified',
  'authorityActivationVerified', 'fullAuthorityHistoryVerified',
  'projectAuthenticationVerified', 'serviceSignatureVerified',
  'priorGlobalEventVerified', 'globalOrderVerified',
  'priorSemanticReceiptVerified', 'controllerStateHeadVerified',
  'rootedClaimVerified', 'runAdjacencyVerified', 'resourceAdjacencyVerified',
  'resourceHighWaterVerified', 'runnerAdmissionVerified', 'hostEvidenceVerified',
  'databaseCommitVerified', 'exactStoredResponseVerified',
  'publicCommitmentVerified', 'checkpointWitnessQuorumVerified',
  'semanticWitnessQuorumVerified', 'resourceFencingVerified',
  'stateTransitionAuthorized', 'attemptStartAuthorized', 'captureAuthorized',
  'importAuthorized', 'promotionAuthorized', 'releaseAuthorized',
]);

export interface AuthorityHeadRefV2 {
  readonly configurationEpoch: string;
  readonly configurationDigest: string;
  readonly headDigest: string;
}

export interface ProjectAssertionV2 {
  readonly projectAuthorityDigest: string;
  readonly principalId: string;
}

export interface CanonicalRegistrationRequestV2 {
  readonly serialized: string;
  readonly serializedSha256: string;
  readonly semanticRequestDigest: string;
  readonly authorityHead: AuthorityHeadRefV2;
  readonly assertedProject: ProjectAssertionV2;
  readonly runId: string;
  readonly priorControllerStateHeadDigest: string;
  readonly claim: Readonly<{
    claimKeyDigest: string;
    claimDigest: string;
    rootedClaimValidationDigest: string;
  }>;
}

export type RegistrationTransportOutcomeV2 =
  | 'registration-not-admitted-v2'
  | 'registration-authority-pending-v2'
  | 'registration-closed-v2'
  | 'transaction-resolution-unknown-v2';

export interface FixedRegistrationTransportResponseV2 {
  readonly outcomeCode: RegistrationTransportOutcomeV2;
  readonly status: 403 | 409 | 500 | 503;
  readonly contentType: typeof REGISTRATION_CONTENT_TYPE_V2;
  readonly recoveryDirective: string;
  readonly body: string;
}

const RESPONSE_MAPPING = Object.freeze({
  'registration-not-admitted-v2': Object.freeze({
    status: 403 as const, recoveryDirective: 'new-authority-bound-request-required',
  }),
  'registration-authority-pending-v2': Object.freeze({
    status: 503 as const, recoveryDirective: 'new-authority-bound-request-after-ready',
  }),
  'registration-closed-v2': Object.freeze({
    status: 409 as const, recoveryDirective: 'new-run-required',
  }),
  'transaction-resolution-unknown-v2': Object.freeze({
    status: 500 as const, recoveryDirective: 'exact-result-lookup-only',
  }),
});

const FIXED_RESPONSES = Object.freeze(Object.fromEntries(
  Object.entries(RESPONSE_MAPPING).map(([outcomeCode, mapping]) => {
    const wire = {
      schemaVersion: 2,
      transactionKind: 'programme-capture-v2',
      responseKind: 'supervisor-registration-non-semantic-response-v2',
      outcomeCode,
      responseStatus: mapping.status,
      responseContentType: REGISTRATION_CONTENT_TYPE_V2,
      recoveryDirective: mapping.recoveryDirective,
    };
    return [outcomeCode, deepFreeze({
      outcomeCode, status: mapping.status, contentType: REGISTRATION_CONTENT_TYPE_V2,
      recoveryDirective: mapping.recoveryDirective,
      body: `${JSON.stringify(wire, null, 2)}\n`,
    })];
  }),
)) as Readonly<Record<RegistrationTransportOutcomeV2, FixedRegistrationTransportResponseV2>>;

export function fixedRegistrationTransportResponseV2(
  outcome: RegistrationTransportOutcomeV2,
): FixedRegistrationTransportResponseV2 {
  if (!Object.hasOwn(FIXED_RESPONSES, outcome)) {
    throw new TypeError('registration transport outcome is invalid');
  }
  const response = FIXED_RESPONSES[outcome];
  if (response === undefined) throw new TypeError('registration transport outcome is invalid');
  return response;
}

export async function parseCanonicalRegistrationRequestV2(
  serialized: string,
): Promise<CanonicalRegistrationRequestV2> {
  const input = parseCanonicalPrettyJson(
    serialized, REGISTRATION_REQUEST_MAX_BYTES_V2, 'registration request',
  );
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'requestKind', 'operationKind', 'authority',
    'authorityHead', 'project', 'runId', 'expectedRegistration', 'claim',
    'verificationScope', ...NON_AUTHORITY_KEYS, 'semanticRequestDigest',
  ], 'registration request');
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.requestKind !== 'supervisor-claim-registration-request-v2'
    || input.operationKind !== 'claim-registered-v2'
    || input.authority !== 'development-only-no-promotion'
    || input.verificationScope !== 'canonical-registration-intent-only'
    || NON_AUTHORITY_KEYS.some((key) => input[key] !== false)) {
    throw new TypeError('registration request identity or authority is invalid');
  }
  const authorityHead = parseAuthorityHead(input.authorityHead, 'registration authority head');
  const project = parseProject(input.project, 'registration asserted project');
  const expected = asRecord(input.expectedRegistration, 'registration expected state');
  assertExactKeys(expected, ['priorControllerStateHeadDigest'], 'registration expected state');
  const claim = asRecord(input.claim, 'registration claim');
  assertExactKeys(
    claim, ['claimKeyDigest', 'claimDigest', 'rootedClaimValidationDigest'],
    'registration claim',
  );
  const runId = parseOpaqueId(input.runId, 'registration run ID');
  const normalizedClaim = {
    claimKeyDigest: parseDigest(claim.claimKeyDigest, 'registration claim-key digest'),
    claimDigest: parseDigest(claim.claimDigest, 'registration claim digest'),
    rootedClaimValidationDigest: parseDigest(
      claim.rootedClaimValidationDigest, 'registration rooted-claim validation digest',
    ),
  };
  const claimKeyDigest = await sha256CanonicalValue({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    keyKind: 'run-claim-key-v1',
    projectAuthorityDigest: project.projectAuthorityDigest,
    runId,
  });
  if (normalizedClaim.claimKeyDigest !== claimKeyDigest) {
    throw new TypeError('registration claim-key digest mismatch');
  }
  const priorControllerStateHeadDigest = parseDigest(
    expected.priorControllerStateHeadDigest, 'registration prior controller-state digest',
  );
  const body = {
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    requestKind: 'supervisor-claim-registration-request-v2',
    operationKind: 'claim-registered-v2',
    authority: 'development-only-no-promotion',
    authorityHead,
    project,
    runId,
    expectedRegistration: { priorControllerStateHeadDigest },
    claim: normalizedClaim,
    verificationScope: 'canonical-registration-intent-only',
    ...Object.fromEntries(NON_AUTHORITY_KEYS.map((key) => [key, false])),
  };
  const semanticRequestDigest = parseDigest(
    input.semanticRequestDigest, 'registration semantic request digest',
  );
  const expectedDigest = await sha256CanonicalValue({
    domain: REGISTRATION_REQUEST_DIGEST_DOMAIN_V2, request: body,
  });
  if (semanticRequestDigest !== expectedDigest) {
    throw new TypeError('registration semantic request digest mismatch');
  }
  if (`${JSON.stringify({ ...body, semanticRequestDigest }, null, 2)}\n` !== serialized) {
    throw new TypeError('registration request member order is noncanonical');
  }
  return deepFreeze({
    serialized,
    serializedSha256: await sha256Text(serialized),
    semanticRequestDigest,
    authorityHead,
    assertedProject: project,
    runId,
    priorControllerStateHeadDigest,
    claim: normalizedClaim,
  });
}

export async function registrationChangedReplayEvidenceDigestV2(value: unknown): Promise<string> {
  const input = cloneClosedRecord(value, 'changed-replay evidence');
  assertExactKeys(input, [
    'originalRegistrationRequestDigest', 'originalRegistrationEventDigest',
    'changedRegistrationRequestDigest', 'project', 'authorityHead',
  ], 'changed-replay evidence');
  return sha256CanonicalValue({
    domain: REGISTRATION_CHANGED_REPLAY_EVIDENCE_DOMAIN_V2,
    originalRegistrationRequestDigest: parseDigest(
      input.originalRegistrationRequestDigest, 'original registration request digest',
    ),
    originalRegistrationEventDigest: parseDigest(
      input.originalRegistrationEventDigest, 'original registration event digest',
    ),
    changedRegistrationRequestDigest: parseDigest(
      input.changedRegistrationRequestDigest, 'changed registration request digest',
    ),
    project: parseProject(input.project, 'changed-replay project'),
    authorityHead: parseAuthorityHead(input.authorityHead, 'changed-replay authority head'),
  });
}

export async function validateStoredRegistrationResultV2(
  serialized: string,
  expected: StoredRegistrationEventExpectationV2,
  status: 201 | 409,
): Promise<void> {
  const result = parseCanonicalPrettyJson(
    serialized, REGISTRATION_RESULT_MAX_BYTES_V2, 'stored registration result',
  );
  const keys = [
    'schemaVersion', 'transactionKind', 'resultKind', 'authority',
    'semanticRequestDigest', 'serializedEventEnvelope', 'responseStatus',
    'responseContentType', 'verificationScope', ...RESULT_NON_AUTHORITY_KEYS, 'resultDigest',
  ];
  assertExactKeys(result, keys, 'stored registration result');
  if (JSON.stringify(Object.keys(result)) !== JSON.stringify(keys)
    || result.schemaVersion !== 2 || result.transactionKind !== 'programme-capture-v2'
    || result.resultKind !== 'supervisor-registration-result-v2'
    || result.authority !== 'development-only-no-promotion'
    || result.semanticRequestDigest !== expected.semanticRequestDigest
    || typeof result.serializedEventEnvelope !== 'string'
    || result.serializedEventEnvelope.length === 0
    || result.responseStatus !== status
    || result.responseContentType !== REGISTRATION_CONTENT_TYPE_V2
    || result.verificationScope
      !== 'canonical-envelope-and-semantic-request-digest-binding-only'
    || RESULT_NON_AUTHORITY_KEYS.some((key) => result[key] !== false)) {
    throw new TypeError('stored registration result contract is invalid');
  }
  await validateStoredRegistrationEventEnvelopeV2(
    result.serializedEventEnvelope as string, expected, status,
  );
  const resultDigest = parseDigest(result.resultDigest, 'stored registration result digest');
  const { resultDigest: _ignored, ...body } = result;
  if (resultDigest !== await sha256CanonicalValue({
    domain: REGISTRATION_RESULT_DIGEST_DOMAIN_V2, result: body,
  })) throw new TypeError('stored registration result digest mismatch');
}

export function parseAuthorityHead(value: unknown, label: string): AuthorityHeadRefV2 {
  const head = asRecord(value, label);
  assertExactKeys(
    head, ['configurationEpoch', 'configurationDigest', 'headDigest'], label,
  );
  return deepFreeze({
    configurationEpoch: parseUint64(head.configurationEpoch, `${label} epoch`),
    configurationDigest: parseDigest(head.configurationDigest, `${label} configuration digest`),
    headDigest: parseDigest(head.headDigest, `${label} digest`),
  });
}

export function parseProject(value: unknown, label: string): ProjectAssertionV2 {
  const project = asRecord(value, label);
  assertExactKeys(project, ['projectAuthorityDigest', 'principalId'], label);
  return deepFreeze({
    projectAuthorityDigest: parseDigest(
      project.projectAuthorityDigest, `${label} authority digest`,
    ),
    principalId: parseOpaqueId(project.principalId, `${label} principal ID`),
  });
}
