// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  programmeCaptureSupervisorAuthorityGenesisHeadDigestV2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import {
  attachProgrammeCaptureSupervisorRunEventSignatureV2,
  buildProgrammeCaptureSupervisorRunEventV2,
} from '../src/programme-capture-supervisor-run-event-builder-v2.js';
import {
  parseProgrammeCaptureSupervisorRegistrationRequestBlobV2,
  parseProgrammeCaptureSupervisorRegistrationRequestV2,
  serializeProgrammeCaptureSupervisorRegistrationRequestV2,
  buildProgrammeCaptureSupervisorRegistrationRequestV2,
  programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2,
  PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_CHANGED_REPLAY_EVIDENCE_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_MAX_BYTES_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_DIGEST_DOMAIN_V2,
} from '../src/programme-capture-supervisor-registration-request-v2.js';
import {
  buildProgrammeCaptureSupervisorServiceResultV2,
  parseProgrammeCaptureSupervisorServiceResultBlobV2,
  parseProgrammeCaptureSupervisorServiceResultV2,
  serializeProgrammeCaptureSupervisorServiceResultV2,
  PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_CONTENT_TYPE_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_DIGEST_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_MAX_BYTES_V2,
} from '../src/programme-capture-supervisor-service-result-v2.js';
import {
  programmeCaptureRunClaimKeyDigestV1,
} from '../src/programme-capture-claim-record-v1.js';
import {
  digest,
  signedEnvelope,
  validAuthorityConfiguration,
  validRunHistory,
  withEventDigest,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';

function requestInput() {
  const configuration = validAuthorityConfiguration();
  const runId = 'capture_run_20260829';
  return {
    authorityHead: {
      configurationEpoch: configuration.configurationEpoch,
      configurationDigest: configuration.configurationDigest,
      headDigest: programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(configuration),
    },
    project: {
      projectAuthorityDigest: configuration.project.projectAuthorityDigest,
      principalId: configuration.project.principal.principalId,
    },
    runId,
    expectedRegistration: {
      priorControllerStateHeadDigest: digest('controller-state-genesis'),
    },
    claim: {
      claimKeyDigest: programmeCaptureRunClaimKeyDigestV1({
        projectAuthorityDigest: configuration.project.projectAuthorityDigest,
        runId,
      }),
      claimDigest: digest('service-codec-claim'),
      rootedClaimValidationDigest: digest('service-codec-rooted-validation'),
    },
  };
}

function registrationEvent(semanticRequestDigest: string) {
  const original = validRunHistory()[0];
  return withEventDigest({
    ...original,
    semanticRequestDigest,
    body: requestInput().claim,
  });
}

describe('programme capture supervisor registration request V2', () => {
  it('round-trips one bounded canonical request with a stable semantic digest', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const serialized = serializeProgrammeCaptureSupervisorRegistrationRequestV2(request);

    expect(serialized.endsWith('\n')).toBe(true);
    expect(parseProgrammeCaptureSupervisorRegistrationRequestBlobV2(serialized)).toEqual(request);
    expect(request.semanticRequestDigest).toBe(
      'd836ed3af320f6840976fd070d994c82712b7c7920688049bd730d7b3abec14d',
    );
    expect(request.verificationScope).toBe('canonical-registration-intent-only');
    expect(request.controllerStateHeadVerified).toBe(false);
    expect(request.rootedClaimVerified).toBe(false);
    expect(request.stateTransitionAuthorized).toBe(false);
    expect(Object.isFrozen(request)).toBe(true);
    expect(Object.isFrozen(request.claim)).toBe(true);
  });

  it('rejects noncanonical bytes, duplicate fields, authority injection, and wrong claim keys', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const serialized = serializeProgrammeCaptureSupervisorRegistrationRequestV2(request);

    expect(() => parseProgrammeCaptureSupervisorRegistrationRequestBlobV2(
      serialized.slice(0, -1),
    )).toThrow(/CANONICAL/);
    expect(() => parseProgrammeCaptureSupervisorRegistrationRequestBlobV2(
      serialized.replace('{\n', '{\n  "schemaVersion": 2,\n'),
    )).toThrow(/duplicate/i);
    expect(() => parseProgrammeCaptureSupervisorRegistrationRequestBlobV2(
      `\ufeff${serialized}`,
    )).toThrow(/CANONICAL/);
    expect(() => parseProgrammeCaptureSupervisorRegistrationRequestBlobV2(
      'x'.repeat(PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_MAX_BYTES_V2 + 1),
    )).toThrow(/CANONICAL/);
    expect(() => buildProgrammeCaptureSupervisorRegistrationRequestV2({
      ...requestInput(), globalSequence: '1',
    })).toThrow(/invalid keys/);
    expect(() => buildProgrammeCaptureSupervisorRegistrationRequestV2({
      ...requestInput(), claim: { ...requestInput().claim, claimKeyDigest: digest('wrong-key') },
    })).toThrow(/CLAIM_KEY_MISMATCH/);
  });

  it('rejects proxies, accessors, zero digests, and post-build caller mutation', () => {
    expect(() => buildProgrammeCaptureSupervisorRegistrationRequestV2(
      new Proxy(requestInput(), {}),
    )).toThrow(/Proxy/);
    expect(() => buildProgrammeCaptureSupervisorRegistrationRequestV2({
      ...requestInput(), project: new Proxy(requestInput().project, {}),
    })).toThrow(/Proxy/);
    const accessor = { ...requestInput() } as Record<string, unknown>;
    Object.defineProperty(accessor, 'runId', { enumerable: true, get: () => 'capture_run_20260829' });
    expect(() => buildProgrammeCaptureSupervisorRegistrationRequestV2(accessor))
      .toThrow(/plain own-key/);
    expect(() => buildProgrammeCaptureSupervisorRegistrationRequestV2({
      ...requestInput(), claim: { ...requestInput().claim, claimDigest: '0'.repeat(64) },
    })).toThrow(/non-zero/);

    const input = requestInput();
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(input);
    (input.claim as { claimDigest: string }).claimDigest = digest('mutated-after-build');
    expect(request.claim.claimDigest).toBe(digest('service-codec-claim'));
  });

  it('rejects every request non-authority escalation and changed state intent', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    for (const key of [
      'externalAdministrationVerified', 'deploymentAttestationVerified',
      'authorityActivationVerified', 'projectAuthenticationVerified',
      'serviceSignatureVerified', 'priorGlobalEventVerified', 'globalOrderVerified',
      'priorSemanticReceiptVerified', 'controllerStateHeadVerified',
      'rootedClaimVerified', 'runAdjacencyVerified', 'stateTransitionAuthorized',
      'attemptStartAuthorized', 'captureAuthorized', 'importAuthorized',
      'promotionAuthorized', 'releaseAuthorized',
    ]) {
      expect(() => parseProgrammeCaptureSupervisorRegistrationRequestV2({
        ...request, [key]: true,
      })).toThrow(/AUTHORITY_ESCALATION/);
    }
    expect(buildProgrammeCaptureSupervisorRegistrationRequestV2({
      ...requestInput(),
      expectedRegistration: { priorControllerStateHeadDigest: digest('another-state') },
    }).semanticRequestDigest).not.toBe(request.semanticRequestDigest);
  });
});

describe('programme capture supervisor run-event builder V2', () => {
  it('constructs the existing closed event format without caller-supplied authority flags', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const expected = registrationEvent(request.semanticRequestDigest);
    const {
      eventDigest: _eventDigest,
      verificationScope: _scope,
      schemaVersion: _schemaVersion,
      transactionKind: _transactionKind,
      recordKind: _recordKind,
      authority: _authority,
      ...input
    } = expected;
    const authorityKeys = [
      'externalAdministrationVerified', 'deploymentAttestationVerified',
      'authorityActivationVerified', 'serviceSignatureVerified', 'priorGlobalEventVerified',
      'priorSemanticReceiptVerified', 'controllerStateHeadVerified', 'rootedClaimVerified',
      'runAdjacencyVerified', 'resourceHighWaterVerified', 'resourceFencingVerified',
      'publicCommitmentVerified', 'checkpointWitnessQuorumVerified',
      'semanticWitnessQuorumVerified', 'stateTransitionAuthorized', 'attemptStartAuthorized',
      'captureAuthorized', 'importAuthorized', 'promotionAuthorized', 'releaseAuthorized',
    ];
    for (const key of authorityKeys) delete (input as Record<string, unknown>)[key];

    const built = buildProgrammeCaptureSupervisorRunEventV2(input);
    expect(built).toEqual(expected);
    expect(built.stateTransitionAuthorized).toBe(false);
    expect(Object.isFrozen(built)).toBe(true);
    expect(() => buildProgrammeCaptureSupervisorRunEventV2({
      ...input, serviceSignatureVerified: true,
    })).toThrow(/invalid keys/);
  });

  it('attaches only a canonical Ed25519 signature to a parsed event', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const event = registrationEvent(request.semanticRequestDigest);
    const existing = JSON.parse(signedEnvelope(event)) as {
      signature: { valueBase64Url: string };
    };
    const envelope = attachProgrammeCaptureSupervisorRunEventSignatureV2({
      event,
      signatureBase64Url: existing.signature.valueBase64Url,
    });

    expect(envelope.event).toEqual(event);
    expect(envelope.signature.algorithm).toBe('ed25519');
    expect(Object.isFrozen(envelope)).toBe(true);
    expect(() => attachProgrammeCaptureSupervisorRunEventSignatureV2({
      event, signatureBase64Url: 'not-a-signature',
    })).toThrow(/SIGNATURE_INVALID/);
  });
});

describe('programme capture supervisor exact service result V2', () => {
  it('round-trips a canonical request-bound event envelope without claiming commit', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const serializedEventEnvelope = signedEnvelope(registrationEvent(
      request.semanticRequestDigest,
    ));
    const result = buildProgrammeCaptureSupervisorServiceResultV2({
      semanticRequestDigest: request.semanticRequestDigest,
      serializedEventEnvelope,
    });
    const serialized = serializeProgrammeCaptureSupervisorServiceResultV2(result);

    expect(parseProgrammeCaptureSupervisorServiceResultBlobV2(serialized)).toEqual(result);
    expect(result.responseStatus).toBe(201);
    expect(result.responseContentType).toBe(
      PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_CONTENT_TYPE_V2,
    );
    expect(result.serviceSignatureVerified).toBe(false);
    expect(result.projectAuthenticationVerified).toBe(false);
    expect(result.databaseCommitVerified).toBe(false);
    expect(result.exactStoredResponseVerified).toBe(false);
    expect(result.stateTransitionAuthorized).toBe(false);
    expect(result.resultDigest).toBe(
      '1f6941df83d70842934147ec95488f5aed7f6ee80656fa9843d2914f27d8ec00',
    );
  });

  it('rejects an event for another request and noncanonical result bytes', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const serializedEventEnvelope = signedEnvelope(registrationEvent(
      request.semanticRequestDigest,
    ));
    expect(() => buildProgrammeCaptureSupervisorServiceResultV2({
      semanticRequestDigest: digest('different-request'), serializedEventEnvelope,
    })).toThrow(/REQUEST_BINDING_MISMATCH/);

    const result = buildProgrammeCaptureSupervisorServiceResultV2({
      semanticRequestDigest: request.semanticRequestDigest, serializedEventEnvelope,
    });
    const serialized = serializeProgrammeCaptureSupervisorServiceResultV2(result);
    expect(() => parseProgrammeCaptureSupervisorServiceResultBlobV2(
      serialized.slice(0, -1),
    )).toThrow(/CANONICAL/);
    expect(() => parseProgrammeCaptureSupervisorServiceResultBlobV2(
      serialized.replace('{\n', '{\n  "schemaVersion": 2,\n'),
    )).toThrow(/duplicate/i);
    expect(() => parseProgrammeCaptureSupervisorServiceResultBlobV2(
      'x'.repeat(PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_MAX_BYTES_V2 + 1),
    )).toThrow(/CANONICAL/);
    expect(() => buildProgrammeCaptureSupervisorServiceResultV2(new Proxy({
      semanticRequestDigest: request.semanticRequestDigest, serializedEventEnvelope,
    }, {}))).toThrow(/Proxy/);
    expect(() => parseProgrammeCaptureSupervisorServiceResultV2({
      ...result, responseStatus: 409,
    })).toThrow(/STATUS_MISMATCH/);
    expect(() => parseProgrammeCaptureSupervisorServiceResultV2({
      ...result, responseContentType: 'application/json',
    })).toThrow(/CONTENT_TYPE_MISMATCH/);
  });

  it('keeps a canonical but cryptographically changed signature explicitly unverified', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const changedSignatureEnvelope = mutateEnvelopeSignature(signedEnvelope(registrationEvent(
      request.semanticRequestDigest,
    )));
    const result = buildProgrammeCaptureSupervisorServiceResultV2({
      semanticRequestDigest: request.semanticRequestDigest,
      serializedEventEnvelope: changedSignatureEnvelope,
    });
    expect(result.serviceSignatureVerified).toBe(false);
  });

  it('rejects every service-result non-authority escalation', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const result = buildProgrammeCaptureSupervisorServiceResultV2({
      semanticRequestDigest: request.semanticRequestDigest,
      serializedEventEnvelope: signedEnvelope(registrationEvent(request.semanticRequestDigest)),
    });
    for (const key of [
      'externalAdministrationVerified', 'deploymentAttestationVerified',
      'authorityActivationVerified', 'fullAuthorityHistoryVerified',
      'projectAuthenticationVerified',
      'serviceSignatureVerified', 'priorGlobalEventVerified', 'globalOrderVerified',
      'priorSemanticReceiptVerified', 'controllerStateHeadVerified',
      'rootedClaimVerified', 'runAdjacencyVerified', 'resourceAdjacencyVerified',
      'resourceHighWaterVerified', 'runnerAdmissionVerified', 'hostEvidenceVerified',
      'databaseCommitVerified',
      'exactStoredResponseVerified', 'publicCommitmentVerified',
      'checkpointWitnessQuorumVerified', 'semanticWitnessQuorumVerified',
      'resourceFencingVerified', 'stateTransitionAuthorized', 'attemptStartAuthorized',
      'captureAuthorized', 'importAuthorized', 'promotionAuthorized', 'releaseAuthorized',
    ]) {
      expect(() => parseProgrammeCaptureSupervisorServiceResultV2({
        ...result, [key]: true,
      })).toThrow(/AUTHORITY_ESCALATION/);
    }
  });
});

describe('programme capture supervisor registration wire KAT V2', () => {
  it('pins independent request, changed-replay, result, and serialized-byte oracles', () => {
    const request = buildProgrammeCaptureSupervisorRegistrationRequestV2(requestInput());
    const { semanticRequestDigest: _requestDigest, ...requestBody } = request;
    const serializedRequest = serializeProgrammeCaptureSupervisorRegistrationRequestV2(request);
    const result = buildProgrammeCaptureSupervisorServiceResultV2({
      semanticRequestDigest: request.semanticRequestDigest,
      serializedEventEnvelope: signedEnvelope(registrationEvent(request.semanticRequestDigest)),
    });
    const { resultDigest: _resultDigest, ...resultBody } = result;
    const serializedResult = serializeProgrammeCaptureSupervisorServiceResultV2(result);
    const evidenceInput = {
      originalRegistrationRequestDigest: digest('kat-original-request'),
      originalRegistrationEventDigest: digest('kat-original-event'),
      changedRegistrationRequestDigest: request.semanticRequestDigest,
      project: request.project,
      authorityHead: request.authorityHead,
    };
    const independentRequestDigest = sha256(canonicalOracle({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_REQUEST_DIGEST_DOMAIN_V2,
      request: requestBody,
    }));
    const independentResultDigest = sha256(canonicalOracle({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_DIGEST_DOMAIN_V2,
      result: resultBody,
    }));
    const independentEvidenceDigest = sha256(canonicalOracle({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_CHANGED_REPLAY_EVIDENCE_DOMAIN_V2,
      ...evidenceInput,
    }));

    expect(request.semanticRequestDigest).toBe(independentRequestDigest);
    expect(result.resultDigest).toBe(independentResultDigest);
    expect(programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2(evidenceInput))
      .toBe(independentEvidenceDigest);
    expect({
      requestSemanticDigest: independentRequestDigest,
      serializedRequestSha256: sha256(serializedRequest),
      changedReplayEvidenceDigest: independentEvidenceDigest,
      resultDigest: independentResultDigest,
      serializedResultSha256: sha256(serializedResult),
    }).toEqual({
      requestSemanticDigest:
        'd836ed3af320f6840976fd070d994c82712b7c7920688049bd730d7b3abec14d',
      serializedRequestSha256:
        '9dc587e8b2a3b2210ec765a2bfb10f359a31ab5df69d1abda32737515636d97c',
      changedReplayEvidenceDigest:
        'c2a8e26c2c255d5d55f65a273c54372e8d6869905df1ecdde872d810da00b075',
      resultDigest:
        '1f6941df83d70842934147ec95488f5aed7f6ee80656fa9843d2914f27d8ec00',
      serializedResultSha256:
        '2e6f23d15836031782c8c9a61c3858eaf6cf522895768b2b8a982c0e863775d8',
    });
    expect(sha256(canonicalOracle({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_DIGEST_DOMAIN_V2,
      request: requestBody,
    }))).not.toBe(independentRequestDigest);
    expect(() => parseProgrammeCaptureSupervisorRegistrationRequestBlobV2(
      serializedRequest.replace(request.claim.claimDigest, flipHex(request.claim.claimDigest)),
    )).toThrow(/DIGEST_MISMATCH/);
    expect(() => parseProgrammeCaptureSupervisorServiceResultV2({
      ...result, resultDigest: flipHex(result.resultDigest),
    })).toThrow(/DIGEST_MISMATCH/);
  });
});

function mutateEnvelopeSignature(serialized: string): string {
  const envelope = JSON.parse(serialized) as {
    signature: { valueBase64Url: string };
  };
  const signature = envelope.signature.valueBase64Url;
  envelope.signature.valueBase64Url = `${signature[0] === 'A' ? 'B' : 'A'}${signature.slice(1)}`;
  return `${JSON.stringify(envelope, null, 2)}\n`;
}

function flipHex(value: string): string {
  return `${value[0] === '0' ? '1' : '0'}${value.slice(1)}`;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function canonicalOracle(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalOracle).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const input = value as Record<string, unknown>;
    return `{${Object.keys(input).sort().map((key) =>
      `${JSON.stringify(key)}:${canonicalOracle(input[key])}`).join(',')}}`;
  }
  const encoded = JSON.stringify(value);
  if (encoded === undefined) throw new TypeError('KAT value is not JSON-serializable');
  return encoded;
}
