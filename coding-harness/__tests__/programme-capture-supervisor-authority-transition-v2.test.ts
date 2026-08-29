// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
  parseProgrammeCaptureSupervisorAuthorityConfigurationV2,
  programmeCaptureSupervisorAuthorityGenesisHeadDigestV2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_MAX_BYTES_V2,
  parseProgrammeCaptureSupervisorAuthorityTransitionBlobV2,
  parseProgrammeCaptureSupervisorAuthorityTransitionV2,
  serializeProgrammeCaptureSupervisorAuthorityTransitionV2,
  verifyProgrammeCaptureSupervisorAuthorityTransitionV2,
} from '../src/programme-capture-supervisor-authority-transition-v2.js';
import { digestValue } from '../src/receipts.js';

describe('programme capture V2 provider-free supervisor authority transition', () => {
  it('matches an independent prior-head, transition, and byte known-answer vector', () => {
    const genesis = configuration();
    const genesisHead = programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(genesis);
    const successor = successorConfiguration(genesis, genesisHead, '1');
    const body = transitionBody(genesis, genesisHead, successor, '1');
    const transitionDigest = digest(canonicalOracle({
      domain: 'semantic-fabric/programme-capture/supervisor-authority-transition-digest-v2',
      transition: body,
    }));
    const transition = parseProgrammeCaptureSupervisorAuthorityTransitionV2({
      ...body, transitionDigest,
    });
    const serialized = serializeProgrammeCaptureSupervisorAuthorityTransitionV2(transition);

    expect(PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_DOMAIN_V2)
      .toBe('semantic-fabric/programme-capture/supervisor-authority-transition-digest-v2');
    expect(genesis.configurationDigest)
      .toBe('19c06ca5059572bd5373679ad64fca5e7f90062d0f5962d5c2e0f43dcdb61433');
    expect(genesisHead)
      .toBe('24c22899e78b41d5e58151971e9a44e5dda0148428f6f4c4bba350f63f36d17d');
    expect(successor.configurationDigest)
      .toBe('b1064b3df06c24868cb9315ab370004957a6d6e6ff203f87dc78c972eea2be84');
    expect(transitionDigest)
      .toBe('a4da36e1bef7a4a6e8905b9e85d7b9804864ee38989afc39a37fbc77af9a8dc9');
    expect(Buffer.byteLength(serialized, 'utf8')).toBe(1_015);
    expect(digest(serialized))
      .toBe('df6d66dfca77b9dd024d8cf9680c29a0f1493de51c1141dbe4192a0c70122725');
    const verified = verifyTransition(transition, genesis, genesisHead, successor);
    expect(verified).toEqual(transition);
    expect(Object.isFrozen(verified)).toBe(true);
    expect(Object.isFrozen(verified.predecessorHead)).toBe(true);
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2({
      ...transition,
      transitionDigest: digestValue({ domain: 'wrong-transition-domain', transition: body }),
    })).toThrow(/DIGEST_MISMATCH/);
    const alteredWithoutRehash = structuredClone(transition) as any;
    alteredWithoutRehash.globalSequence = '2';
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(alteredWithoutRehash))
      .toThrow(/DIGEST_MISMATCH/);
  });

  it('chains two hops against the current external head, never the grandparent head', () => {
    const genesis = configuration();
    const genesisHead = programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(genesis);
    const firstConfiguration = successorConfiguration(genesis, genesisHead, '1');
    const first = transition(genesis, genesisHead, firstConfiguration, '1');
    const secondConfiguration = successorConfiguration(
      firstConfiguration, first.transitionDigest, '2',
    );
    const second = transition(
      firstConfiguration, first.transitionDigest, secondConfiguration, '2',
    );

    expect(verifyTransition(
      second, firstConfiguration, first.transitionDigest, secondConfiguration,
    )).toEqual(second);
    const grandparentBoundConfiguration = successorConfiguration(
      firstConfiguration, genesisHead, '2',
    );
    const grandparentBound = transition(
      firstConfiguration, genesisHead, grandparentBoundConfiguration, '2',
    );
    expect(() => verifyTransition(
      grandparentBound, firstConfiguration, first.transitionDigest,
      grandparentBoundConfiguration,
    )).toThrow(/PREDECESSOR_HEAD_MISMATCH/);
  });

  it('rejects skipped, reversed, and overflowing configuration epochs', () => {
    const genesis = configuration();
    const genesisHead = programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(genesis);
    const skipped = successorConfiguration(genesis, genesisHead, '2');
    expect(() => verifyTransition(
      transition(genesis, genesisHead, skipped), genesis, genesisHead, skipped,
    )).toThrow(/EPOCH_SEQUENCE_INVALID/);

    const firstConfiguration = successorConfiguration(genesis, genesisHead, '1');
    const secondConfiguration = successorConfiguration(
      firstConfiguration, digest('first-activation-head'), '2',
    );
    expect(() => verifyTransition(
      transition(secondConfiguration, digest('second-activation-head'), firstConfiguration),
      secondConfiguration, digest('second-activation-head'), firstConfiguration,
    )).toThrow(/EPOCH_SEQUENCE_INVALID/);

    const maximum = configuration(
      '18446744073709551615',
      {
        kind: 'configuration-head',
        configurationDigest: digest('maximum-predecessor'),
        headDigest: digest('maximum-head'),
      },
    );
    const impossible = configuration(
      '18446744073709551615',
      {
        kind: 'configuration-head',
        configurationDigest: maximum.configurationDigest,
        headDigest: digest('maximum-current-head'),
      },
    );
    expect(() => verifyTransition(
      transition(maximum, digest('maximum-current-head'), impossible),
      maximum, digest('maximum-current-head'), impossible,
    )).toThrow(/EPOCH_OVERFLOW/);
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(
      rehashTransition(transition(genesis, genesisHead, skipped), (body) => {
        body.globalSequence = '01';
      }),
    )).toThrow(/canonical uint64/);
  });

  it('rejects self-selected, stale, zero, and wrong-domain predecessor heads', () => {
    const genesis = configuration();
    const genesisHead = programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(genesis);
    const successor = successorConfiguration(genesis, genesisHead, '1');
    const valid = transition(genesis, genesisHead, successor);
    expect(() => verifyTransition(valid, genesis, genesisHead, successor, '2'))
      .toThrow(/GLOBAL_SEQUENCE_MISMATCH/);
    expect(() => verifyTransition(valid, genesis, digest('stale-head'), successor))
      .toThrow(/PREDECESSOR_HEAD_MISMATCH/);

    const wrongDomainHead = digestValue({
      domain: 'wrong-genesis-head-domain', configurationDigest: genesis.configurationDigest,
    });
    const rebound = successorConfiguration(genesis, wrongDomainHead, '1');
    expect(() => verifyTransition(
      transition(genesis, wrongDomainHead, rebound), genesis, wrongDomainHead, rebound,
    )).toThrow(/GENESIS_HEAD_MISMATCH/);
    const configurationDigestBound = successorConfiguration(
      genesis, genesis.configurationDigest, '1',
    );
    expect(() => verifyTransition(
      transition(genesis, genesis.configurationDigest, configurationDigestBound),
      genesis, genesis.configurationDigest, configurationDigestBound,
    )).toThrow(/GENESIS_HEAD_MISMATCH/);

    const zeroHead = rehashTransition(valid, (body) => {
      body.predecessorHead.headDigest = '0'.repeat(64);
    });
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(zeroHead))
      .toThrow(/non-zero lowercase SHA-256/);
  });

  it('rejects predecessor, successor, and event binding substitutions', () => {
    const genesis = configuration();
    const genesisHead = programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(genesis);
    const successor = successorConfiguration(genesis, genesisHead, '1');
    const valid = transition(genesis, genesisHead, successor);
    const alternateGenesis = configuration('0', undefined, {
      readinessPolicyDigest: digest('alternate-readiness-policy'),
    });
    expect(() => verifyTransition(valid, alternateGenesis, genesisHead, successor))
      .toThrow(/PREDECESSOR_CONFIGURATION_MISMATCH/);

    const alternateSuccessor = successorConfiguration(
      genesis, genesisHead, '1', { readinessPolicyDigest: digest('successor-substitution') },
    );
    expect(() => verifyTransition(valid, genesis, genesisHead, alternateSuccessor))
      .toThrow(/SUCCESSOR_CONFIGURATION_MISMATCH/);

    const reboundEvent = rehashTransition(valid, (body) => {
      body.predecessorHead.configurationDigest = digest('event-predecessor-substitution');
    });
    expect(() => verifyTransition(reboundEvent, genesis, genesisHead, successor))
      .toThrow(/PREDECESSOR_CONFIGURATION_MISMATCH/);
  });

  it('forces every authority claim false and rejects identity or shape expansion', () => {
    const { genesis, genesisHead, successor, value } = fixture();
    expect(parseProgrammeCaptureSupervisorAuthorityTransitionV2(value)).toMatchObject({
      authority: 'development-only-no-promotion',
      verificationScope: 'configuration-adjacency-only',
      externalAdministrationVerified: false,
      deploymentAttestationVerified: false,
      checkpointWitnessQuorumVerified: false,
      semanticWitnessQuorumVerified: false,
      stateTransitionAuthorized: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    for (const field of NON_AUTHORITY_FIELDS) {
      expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(
        rehashTransition(value, (body) => { body[field] = true; }),
      )).toThrow(/AUTHORITY_ESCALATION/);
    }
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(
      rehashTransition(value, (body) => { body.authority = 'promotion-authority'; }),
    )).toThrow(/IDENTITY_INVALID/);
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2({
      ...value, signature: 'forbidden',
    })).toThrow(/invalid keys/);
    expect(verifyTransition(value, genesis, genesisHead, successor)).toEqual(value);
  });

  it('requires bounded duplicate-free canonical blobs', () => {
    const { value } = fixture();
    const canonical = serializeProgrammeCaptureSupervisorAuthorityTransitionV2(value);
    expect(parseProgrammeCaptureSupervisorAuthorityTransitionBlobV2(canonical)).toEqual(value);
    for (const invalid of [
      JSON.stringify(value),
      canonical.replace('"schemaVersion": 2,', '"schemaVersion": 2,\n  "schemaVersion": 2,'),
      canonical.replace('"configurationEpoch": "0",',
        '"configurationEpoch": "0",\n    "configurationEpoch": "0",'),
      `\uFEFF${canonical}`,
      `${canonical}\n`,
      ' '.repeat(PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_MAX_BYTES_V2 + 1),
    ]) {
      expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionBlobV2(invalid)).toThrow();
    }
  });

  it('rejects Proxies, accessors, symbols, and inherited state without invoking traps', () => {
    const { genesis, genesisHead, successor, value } = fixture();
    let trapCalls = 0;
    const proxy = new Proxy(value, {
      getPrototypeOf() { trapCalls += 1; return Object.prototype; },
      ownKeys() { trapCalls += 1; return []; },
    });
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(proxy)).toThrow(/Proxy/);
    expect(trapCalls).toBe(0);
    const nestedProxy = structuredClone(value) as any;
    nestedProxy.predecessorHead = new Proxy(nestedProxy.predecessorHead, {
      getPrototypeOf() { trapCalls += 1; return Object.prototype; },
      ownKeys() { trapCalls += 1; return []; },
    });
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(nestedProxy))
      .toThrow(/Proxy/);
    expect(trapCalls).toBe(0);

    const contextProxy = new Proxy({
      predecessorConfiguration: genesis,
      expectedPredecessorHeadDigest: genesisHead,
      successorConfiguration: successor,
    }, {
      getPrototypeOf() { trapCalls += 1; return Object.prototype; },
      ownKeys() { trapCalls += 1; return []; },
    });
    expect(() => verifyProgrammeCaptureSupervisorAuthorityTransitionV2(value, contextProxy))
      .toThrow(/Proxy/);
    expect(trapCalls).toBe(0);

    const hostile = structuredClone(value) as any;
    Object.defineProperty(hostile.predecessorHead, 'headDigest', {
      enumerable: true, get: () => digest('trap'),
    });
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(hostile))
      .toThrow(/plain own-key object/);
    const inherited = structuredClone(value) as any;
    Object.setPrototypeOf(inherited.successorConfiguration, { captureAuthorized: true });
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(inherited))
      .toThrow(/plain own-key object/);
    const symbol = structuredClone(value) as any;
    Object.defineProperty(symbol.predecessorHead, Symbol('authority'), {
      enumerable: true, value: true,
    });
    expect(() => parseProgrammeCaptureSupervisorAuthorityTransitionV2(symbol))
      .toThrow(/plain own-key object/);
  });
});

const NON_AUTHORITY_FIELDS = [
  'externalAdministrationVerified', 'deploymentAttestationVerified',
  'checkpointWitnessQuorumVerified', 'semanticWitnessQuorumVerified',
  'stateTransitionAuthorized', 'attemptStartAuthorized', 'captureAuthorized',
] as const;

function fixture() {
  const genesis = configuration();
  const genesisHead = programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(genesis);
  const successor = successorConfiguration(genesis, genesisHead, '1');
  return { genesis, genesisHead, successor, value: transition(genesis, genesisHead, successor) };
}

function verifyTransition(value: unknown, predecessorConfiguration: unknown,
  expectedPredecessorHeadDigest: string, successorConfiguration: unknown,
  expectedGlobalSequence = (value as any).globalSequence) {
  return verifyProgrammeCaptureSupervisorAuthorityTransitionV2(value, {
    predecessorConfiguration, expectedPredecessorHeadDigest,
    expectedGlobalSequence, successorConfiguration,
  });
}

function transition(predecessor: any, headDigest: string, successor: any,
  globalSequence = '1') {
  const body = transitionBody(predecessor, headDigest, successor, globalSequence);
  return parseProgrammeCaptureSupervisorAuthorityTransitionV2({
    ...body,
    transitionDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_DOMAIN_V2,
      transition: body,
    }),
  });
}

function transitionBody(predecessor: any, headDigest: string, successor: any,
  globalSequence = '1'): any {
  return {
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    recordKind: 'supervisor-authority-transition-v2',
    authority: 'development-only-no-promotion',
    globalSequence,
    predecessorHead: {
      configurationEpoch: predecessor.configurationEpoch,
      configurationDigest: predecessor.configurationDigest,
      headDigest,
    },
    successorConfiguration: {
      configurationEpoch: successor.configurationEpoch,
      configurationDigest: successor.configurationDigest,
    },
    verificationScope: 'configuration-adjacency-only',
    externalAdministrationVerified: false,
    deploymentAttestationVerified: false,
    checkpointWitnessQuorumVerified: false,
    semanticWitnessQuorumVerified: false,
    stateTransitionAuthorized: false,
    attemptStartAuthorized: false,
    captureAuthorized: false,
  };
}

function rehashTransition(value: any, mutate: (body: any) => void): any {
  const body = structuredClone(value);
  delete body.transitionDigest;
  mutate(body);
  return {
    ...body,
    transitionDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_DOMAIN_V2,
      transition: body,
    }),
  };
}

function successorConfiguration(predecessor: any, headDigest: string, epoch: string,
  overrides: Record<string, unknown> = {}) {
  return configuration(epoch, {
    kind: 'configuration-head',
    configurationDigest: predecessor.configurationDigest,
    headDigest,
  }, overrides);
}

function configuration(configurationEpoch = '0', predecessor: unknown = undefined,
  overrides: Record<string, unknown> = {}) {
  const body = configurationBody(configurationEpoch, predecessor, overrides);
  return parseProgrammeCaptureSupervisorAuthorityConfigurationV2({
    ...body,
    configurationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
      configuration: body,
    }),
  });
}

function configurationBody(configurationEpoch: string, predecessor: unknown,
  overrides: Record<string, unknown>): any {
  return {
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    recordKind: 'supervisor-authority-configuration-v2',
    authority: 'development-only-no-promotion',
    configurationEpoch,
    predecessor: predecessor ?? {
      kind: 'genesis', configurationDigest: null, headDigest: null,
    },
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
    ...overrides,
  };
}

function witnessPolicy(prefix: string) {
  return {
    policyId: `${prefix}_witness_policy_20260829`,
    faultThreshold: '1', quorumThreshold: '3',
    members: Array.from({ length: 4 }, (_, index) =>
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
