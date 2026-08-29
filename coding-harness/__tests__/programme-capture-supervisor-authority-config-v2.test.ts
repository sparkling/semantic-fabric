// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_MAX_BYTES_V2,
  parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2,
  parseProgrammeCaptureSupervisorAuthorityConfigurationV2,
  serializeProgrammeCaptureSupervisorAuthorityConfigurationV2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import { digestValue } from '../src/receipts.js';

describe('programme capture V2 provider-free supervisor authority configuration', () => {
  it('matches an independent canonical digest and serialized-byte known-answer vector', () => {
    const body = validBody();
    const configurationDigest = digest(canonicalOracle({
      domain: 'semantic-fabric/programme-capture/supervisor-authority-configuration-digest-v2',
      configuration: body,
    }));
    expect(configurationDigest)
      .toBe('58db018b7b0d1c20572cc4b586b1c4548ee6a1136c2c589d9258b1233c6f25db');

    const serialized = serializeProgrammeCaptureSupervisorAuthorityConfigurationV2({
      ...body, configurationDigest,
    });
    expect(Buffer.byteLength(serialized, 'utf8')).toBe(7_237);
    expect(digest(serialized))
      .toBe('c29109f29525a6e42db72ce4604d448e0a28e20ad46d49764ccad0d38a717454');
  });

  it('canonicalizes exact trust pins without granting runtime authority', () => {
    const configuration = validConfiguration();
    const serialized = serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(configuration);
    const parsed = parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2(serialized);

    expect(parsed).toEqual(configuration);
    expect(serialized).toBe(`${JSON.stringify(configuration, null, 2)}\n`);
    expect(parsed).toMatchObject({
      authority: 'development-only-no-promotion',
      verificationScope: 'trust-pins-and-quorum-math-only',
      externalAdministrationVerified: false,
      deploymentAttestationVerified: false,
      checkpointWitnessQuorumVerified: false,
      semanticWitnessQuorumVerified: false,
      stateTransitionAuthorized: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    expect(Object.isFrozen(parsed)).toBe(true);
    expect(Object.isFrozen(parsed.checkpointWitnesses.members)).toBe(true);
  });

  it('enforces Byzantine quorum intersection for the actual roster size', () => {
    expect(() => validConfiguration({ checkpointWitnesses: witnessPolicy('checkpoint', 4, 1, 3) }))
      .not.toThrow();
    expect(() => validConfiguration({ semanticWitnesses: witnessPolicy('semantic', 7, 2, 5) }))
      .not.toThrow();

    for (const policy of [
      witnessPolicy('checkpoint', 3, 1, 3),
      witnessPolicy('checkpoint', 4, 1, 2),
      witnessPolicy('checkpoint', 5, 1, 3),
      witnessPolicy('checkpoint', 7, 2, 4),
    ]) {
      expect(() => validConfiguration({ checkpointWitnesses: policy }))
        .toThrow(/QUORUM_POLICY_INVALID/);
    }
  });

  it('rejects reused role keys, administrations, identities, and unsorted rosters', () => {
    for (const mutate of [
      (body: any) => { body.semanticWitnesses.members[0].keyFingerprint =
        body.checkpointWitnesses.members[0].keyFingerprint; },
      (body: any) => { body.runnerEnrollment.administrationDigest =
        body.initializationAnchor.administrationDigest; },
      (body: any) => { body.deploymentAttestor.principalId = body.service.principal.principalId; },
      (body: any) => { body.semanticWitnesses.policyId = body.checkpointWitnesses.policyId; },
      (body: any) => { body.transparencyLog.endpointOrigin = body.service.endpointOrigin; },
      (body: any) => { body.semanticWitnesses.members.reverse(); },
    ]) {
      const body = validBody();
      mutate(body);
      expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2(withDigest(body)))
        .toThrow();
    }
  });

  it('enforces canonical uint64 epochs and typed predecessor semantics', () => {
    for (const configurationEpoch of ['00', '01', '-1', '18446744073709551616']) {
      expect(() => validConfiguration({ configurationEpoch })).toThrow(/uint64/);
    }
    expect(() => validConfiguration({
      configurationEpoch: '1',
      predecessor: { kind: 'genesis', configurationDigest: null, transitionDigest: null },
    })).toThrow(/PREDECESSOR_INVALID/);
    expect(() => validConfiguration({
      configurationEpoch: '1',
      predecessor: {
        kind: 'transition', configurationDigest: digest('prior-config'),
        transitionDigest: digest('prior-transition'),
      },
    })).not.toThrow();
    expect(() => validConfiguration({
      configurationEpoch: '0',
      predecessor: {
        kind: 'transition', configurationDigest: digest('prior-config'),
        transitionDigest: digest('prior-transition'),
      },
    })).toThrow(/PREDECESSOR_INVALID/);
    expect(() => validConfiguration({
      configurationEpoch: '18446744073709551615',
      predecessor: {
        kind: 'transition', configurationDigest: digest('prior-config'),
        transitionDigest: digest('prior-transition'),
      },
    })).not.toThrow();
  });

  it('rejects mutable trust-pin and authority escalation mutations even when redigested', () => {
    for (const mutate of [
      (body: any) => { body.service.endpointOrigin = 'http://supervisor.example.org'; },
      (body: any) => { body.service.endpointOrigin = 'https://user@supervisor.example.org'; },
      (body: any) => { body.service.endpointOrigin = 'https://localhost'; },
      (body: any) => { body.service.principal.keyEpoch = '01'; },
      (body: any) => { body.project.projectAuthorityDigest = '0'.repeat(64); },
      (body: any) => { body.captureAuthorized = true; },
      (body: any) => { body.extra = true; },
    ]) {
      const body = validBody();
      mutate(body);
      expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2(withDigest(body)))
        .toThrow();
    }
  });

  it('rejects noncanonical blobs, duplicate keys, oversized arrays, and hostile objects', () => {
    const canonical = serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(
      validConfiguration(),
    );
    for (const invalid of [
      JSON.stringify(JSON.parse(canonical)), `${canonical} `, `\ufeff${canonical}`,
      canonical.replace('"schemaVersion": 2,', '"schemaVersion": 2,\n  "schemaVersion": 2,'),
      ' '.repeat(PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_MAX_BYTES_V2 + 1),
    ]) expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2(invalid)).toThrow();

    const sparse = Array(4);
    sparse[0] = principal('sparse_checkpoint_0');
    sparse[3] = principal('sparse_checkpoint_3');
    expect(() => validConfiguration({
      checkpointWitnesses: {
        policyId: 'checkpoint_policy_sparse', faultThreshold: '1',
        quorumThreshold: '3', members: sparse,
      },
    })).toThrow();
    expect(() => validConfiguration({
      checkpointWitnesses: witnessPolicy('large_checkpoint', 65, 1, 34),
    })).toThrow(/WITNESS_ROSTER_INVALID/);

    let trapCalls = 0;
    const proxy = new Proxy(validConfiguration(), {
      getPrototypeOf() { trapCalls += 1; return Object.prototype; },
      ownKeys() { trapCalls += 1; return []; },
    });
    expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2(proxy)).toThrow(/Proxy/);
    expect(trapCalls).toBe(0);

    const nestedProxy = structuredClone(validConfiguration()) as any;
    nestedProxy.service = new Proxy(nestedProxy.service, {
      getPrototypeOf() { trapCalls += 1; return Object.prototype; },
      ownKeys() { trapCalls += 1; return []; },
    });
    expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2(nestedProxy))
      .toThrow(/Proxy/);
    expect(trapCalls).toBe(0);

    const hostile = structuredClone(validConfiguration()) as any;
    const member = hostile.semanticWitnesses.members[0];
    Object.defineProperty(member, 'keyFingerprint', { enumerable: true, get: () => digest('trap') });
    expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2(hostile))
      .toThrow(/plain own-key object/);

    const hostilePrototype = structuredClone(validConfiguration()) as any;
    Object.setPrototypeOf(hostilePrototype.service, { inheritedAuthority: true });
    expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2(hostilePrototype))
      .toThrow(/plain own-key object/);

    const symbolKey = structuredClone(validConfiguration()) as any;
    Object.defineProperty(symbolKey.project, Symbol('authority'), {
      enumerable: true, value: true,
    });
    expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2(symbolKey))
      .toThrow(/plain own-key object/);
  });

  it('rejects a configuration digest from another domain or altered trust pin', () => {
    const configuration = validConfiguration();
    expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2({
      ...configuration,
      configurationDigest: digestValue({ domain: 'wrong-domain', configuration: validBody() }),
    })).toThrow(/DIGEST_MISMATCH/);
    const altered = structuredClone(configuration) as any;
    altered.readinessPolicyDigest = digest('altered-readiness');
    expect(() => parseProgrammeCaptureSupervisorAuthorityConfigurationV2(altered))
      .toThrow(/DIGEST_MISMATCH/);
  });
});

function validConfiguration(overrides: Record<string, unknown> = {}) {
  return parseProgrammeCaptureSupervisorAuthorityConfigurationV2(
    withDigest(validBody(overrides)),
  );
}

function withDigest(body: Record<string, unknown>) {
  return {
    ...body,
    configurationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
      configuration: body,
    }),
  };
}

function validBody(overrides: Record<string, unknown> = {}): any {
  return {
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    recordKind: 'supervisor-authority-configuration-v2',
    authority: 'development-only-no-promotion',
    configurationEpoch: '0',
    predecessor: { kind: 'genesis', configurationDigest: null, transitionDigest: null },
    project: {
      projectAuthorityDigest: digest('project-authority'),
      principal: principal('project_client_20260829'),
      authenticationPolicyDigest: digest('project-authentication-policy'),
    },
    service: {
      principal: principal('supervisor_service_20260829'),
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
    checkpointWitnesses: witnessPolicy('checkpoint', 4, 1, 3),
    semanticWitnesses: witnessPolicy('semantic', 4, 1, 3),
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
    ...overrides,
  };
}

function witnessPolicy(prefix: string, count: number, fault: number, quorum: number) {
  return {
    policyId: `${prefix}_witness_policy_20260829`,
    faultThreshold: String(fault),
    quorumThreshold: String(quorum),
    members: Array.from({ length: count }, (_, index) =>
      principal(`${prefix}_witness_${String(index).padStart(2, '0')}_20260829`)),
  };
}

function principal(principalId: string) {
  return {
    principalId,
    keyEpoch: '1',
    keyFingerprint: digest(`${principalId}:key`),
    policyDigest: digest(`${principalId}:policy`),
    administrationDigest: digest(`${principalId}:administration`),
  };
}

function digest(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

function canonicalOracle(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalOracle).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const input = value as Record<string, unknown>;
    return `{${Object.keys(input).sort().map((key) =>
      `${JSON.stringify(key)}:${canonicalOracle(input[key])}`).join(',')}}`;
  }
  const encoded = JSON.stringify(value);
  if (encoded === undefined) throw new TypeError('KAT oracle value is not JSON-serializable');
  return encoded;
}
