// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  fixedRegistrationTransportResponseV2,
  parseCanonicalRegistrationRequestV2,
  registrationChangedReplayEvidenceDigestV2,
} from '../src/index.js';
import {
  CANONICAL_REQUEST, DIGEST, canonical, canonicalPretty, sha256Text,
} from './registration-fixtures.js';

function withRecomputedRequestDigest(value: Record<string, unknown>): string {
  const { semanticRequestDigest: _ignored, ...body } = value;
  return canonicalPretty({
    ...body,
    semanticRequestDigest: sha256Text(canonical({
      domain: 'semantic-fabric/programme-capture/supervisor-registration-request-digest-v2',
      request: body,
    })),
  });
}

describe('service-local registration protocol V2', () => {
  it('parses the independent canonical request KAT and binds both digest domains', async () => {
    const request = await parseCanonicalRegistrationRequestV2(CANONICAL_REQUEST);

    expect(request).toEqual({
      serialized: CANONICAL_REQUEST,
      serializedSha256: DIGEST.requestBytes,
      semanticRequestDigest: DIGEST.request,
      authorityHead: {
        configurationEpoch: '0',
        configurationDigest: DIGEST.configuration,
        headDigest: DIGEST.head,
      },
      assertedProject: {
        projectAuthorityDigest: DIGEST.project,
        principalId: 'project_client_20260829',
      },
      runId: 'capture_run_20260829',
      priorControllerStateHeadDigest: DIGEST.priorController,
      claim: {
        claimKeyDigest: DIGEST.claimKey,
        claimDigest: DIGEST.claim,
        rootedClaimValidationDigest: DIGEST.rootedClaim,
      },
    });
    expect(sha256Text(request.serialized)).toBe(DIGEST.requestBytes);
    expect(Object.isFrozen(request)).toBe(true);
    expect(Object.isFrozen(request.claim)).toBe(true);

    await expect(registrationChangedReplayEvidenceDigestV2({
      originalRegistrationRequestDigest: sha256Text('kat-original-request'),
      originalRegistrationEventDigest: sha256Text('kat-original-event'),
      changedRegistrationRequestDigest: DIGEST.request,
      project: request.assertedProject,
      authorityHead: request.authorityHead,
    })).resolves.toBe(
      'c2a8e26c2c255d5d55f65a273c54372e8d6869905df1ecdde872d810da00b075',
    );
  });

  it('rejects noncanonical, unbound, unknown, and authority-escalated request bytes', async () => {
    const parsed = JSON.parse(CANONICAL_REQUEST) as Record<string, unknown>;
    const mutations = [
      CANONICAL_REQUEST.slice(0, -1),
      CANONICAL_REQUEST.replace('{\n', '{\n  "schemaVersion": 2,\n'),
      canonicalPretty({ extra: false, ...parsed }),
      canonicalPretty({ ...parsed, semanticRequestDigest: sha256Text('wrong-request') }),
      canonicalPretty({
        ...parsed,
        claim: { ...(parsed.claim as object), claimKeyDigest: sha256Text('wrong-key') },
      }),
      canonicalPretty({
        ...parsed,
        authorityHead: {
          ...(parsed.authorityHead as object), configurationDigest: '0'.repeat(64),
        },
      }),
      withRecomputedRequestDigest({
        ...parsed,
        authorityHead: { ...(parsed.authorityHead as object), configurationEpoch: '01' },
      }),
      withRecomputedRequestDigest({
        ...parsed,
        authorityHead: {
          ...(parsed.authorityHead as object),
          configurationEpoch: '18446744073709551616',
        },
      }),
    ];
    for (const mutation of mutations) {
      await expect(parseCanonicalRegistrationRequestV2(mutation)).rejects.toThrow();
    }
    for (const key of [
      'externalAdministrationVerified', 'deploymentAttestationVerified',
      'authorityActivationVerified', 'projectAuthenticationVerified',
      'serviceSignatureVerified', 'priorGlobalEventVerified', 'globalOrderVerified',
      'priorSemanticReceiptVerified', 'controllerStateHeadVerified',
      'rootedClaimVerified', 'runAdjacencyVerified', 'stateTransitionAuthorized',
      'attemptStartAuthorized', 'captureAuthorized', 'importAuthorized',
      'promotionAuthorized', 'releaseAuthorized',
    ]) {
      await expect(parseCanonicalRegistrationRequestV2(
        canonicalPretty({ ...parsed, [key]: true }),
      ), key).rejects.toThrow();
    }
    const { schemaVersion, transactionKind, ...rest } = parsed;
    await expect(parseCanonicalRegistrationRequestV2(canonicalPretty({
      transactionKind, schemaVersion, ...rest,
    }))).rejects.toThrow(/member order/);
    await expect(parseCanonicalRegistrationRequestV2(
      null as unknown as string,
    )).rejects.toThrow();
    await expect(parseCanonicalRegistrationRequestV2('x'.repeat(32_769)))
      .rejects.toThrow(/bounds/);

    await expect(parseCanonicalRegistrationRequestV2(withRecomputedRequestDigest({
      ...parsed,
      authorityHead: {
        ...(parsed.authorityHead as object),
        configurationEpoch: '18446744073709551615',
      },
    }))).resolves.toMatchObject({
      authorityHead: { configurationEpoch: '18446744073709551615' },
    });
  });

  it('pins all four disclosure-free response byte strings and hashes', () => {
    const expected = {
      'registration-not-admitted-v2': {
        status: 403, recoveryDirective: 'new-authority-bound-request-required',
        digest: '6e9a9390950005c4e4b1a5b3435278b34d07171fb00e835be5ec4babcdb4cf49',
      },
      'registration-authority-pending-v2': {
        status: 503, recoveryDirective: 'new-authority-bound-request-after-ready',
        digest: '3380a4e31017edae8206a0616e7e928e662208ba44ca891332e9de658166dae9',
      },
      'registration-closed-v2': {
        status: 409, recoveryDirective: 'new-run-required',
        digest: '11fbdd37a703d190be8d30db1582ffc7843f686238e6ee0fec88e9adf7fce0da',
      },
      'transaction-resolution-unknown-v2': {
        status: 500, recoveryDirective: 'exact-result-lookup-only',
        digest: 'e23e32a1abfdce0031973a46f6af37fb6099e9f75e4d7128b885992b0c4742f6',
      },
    } as const;

    for (const [outcome, oracle] of Object.entries(expected)) {
      const response = fixedRegistrationTransportResponseV2(
        outcome as keyof typeof expected,
      );
      expect(response, outcome).toEqual({
        outcomeCode: outcome,
        status: oracle.status,
        contentType: 'application/json; charset=utf-8',
        recoveryDirective: oracle.recoveryDirective,
        body: response.body,
      });
      expect(sha256Text(response.body), outcome).toBe(oracle.digest);
      expect(response.body, outcome).not.toMatch(
        /capture_run|project_client|[a-f0-9]{64}|Retry-After/,
      );
      expect(Object.isFrozen(response), outcome).toBe(true);
    }
    expect(() => fixedRegistrationTransportResponseV2('__proto__' as any)).toThrow();
    expect(() => fixedRegistrationTransportResponseV2('unknown' as never)).toThrow();
  });
});
