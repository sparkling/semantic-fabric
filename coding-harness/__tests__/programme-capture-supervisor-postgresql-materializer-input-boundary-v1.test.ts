// SPDX-License-Identifier: MIT

import { createHash, createPrivateKey, createPublicKey } from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import { digestValue } from '../src/receipts.js';
import {
  changedReplayMaterializerFixtureV1,
  genesisMaterializerFixtureV1,
} from './programme-capture-supervisor-postgresql-materializer-fixtures-v1.js';
import {
  preparePostgresRegistrationMaterializationV1,
} from '../supervisor-service/src/registration-postgresql-materializer-v1.js';
import {
  parsePostgresAuthorityConfigurationBytesV1,
} from '../supervisor-service/src/registration-postgresql-authority-configuration-v1.js';
import {
  registrationChangedReplayEvidenceDigestV2,
} from '../supervisor-service/src/registration-protocol-v2.js';

type Mutation = readonly [
  name: string,
  mutate: (candidate: any, snapshots: any) => void | Promise<void>,
  error: RegExp,
];

const GENESIS_MUTATIONS: readonly Mutation[] = [
  ['extra candidate capability', (candidate) => {
    candidate.mutationAuthorized = true;
  }, /registration materializer candidate has invalid keys/],
  ['request snapshot digest drift', (candidate) => {
    candidate.request.semanticRequestDigest = '1'.repeat(64);
  }, /candidate request snapshot mismatch/],
  ['registration claim drift', (candidate) => {
    candidate.body = structuredClone(candidate.body);
    candidate.body.claimDigest = '1'.repeat(64);
  }, /registration candidate claim mismatch/],
  ['extra snapshot', (_candidate, value) => { value.extra = null; }, /invalid keys/],
  ['missing snapshot', (_candidate, value) => {
    delete value.lockedRunState;
  }, /invalid keys/],
  ['configuration without LF', (_candidate, value) => {
    const text = Buffer.from(value.lockedConfiguration.serializedConfiguration)
      .toString('utf8').slice(0, -1);
    replaceConfigurationBytes(value, text);
  }, /exact pretty canonical JSON plus LF/],
  ['reordered configuration', (_candidate, value) => {
    const parsed = configurationRecord(value);
    const { schemaVersion, ...rest } = parsed;
    replaceConfigurationBytes(value, `${JSON.stringify({ ...rest, schemaVersion }, null, 2)}\n`);
  }, /member order is noncanonical/],
  ['stale configuration digest', (_candidate, value) => {
    const parsed = configurationRecord(value);
    parsed.readinessPolicyDigest = '1'.repeat(64);
    replaceConfigurationBytes(value, `${JSON.stringify(parsed, null, 2)}\n`);
  }, /configuration digest mismatch/],
  ['authority escalation in configuration', (_candidate, value) => {
    const parsed = configurationRecord(value);
    parsed.captureAuthorized = true;
    replaceConfigurationBytes(value, `${JSON.stringify(parsed, null, 2)}\n`);
  }, /configuration identity is invalid/],
  ['wrong configuration byte hash', (_candidate, value) => {
    flip(value.lockedConfiguration.serializedConfigurationSha256);
  }, /configuration row is inconsistent/],
  ['short service SPKI', (_candidate, value) => {
    value.lockedConfiguration.serviceSigningSpkiDer = new Uint8Array(43);
  }, /exact bounded Uint8Array/],
  ['wrong service SPKI', (_candidate, value) => {
    flip(value.lockedConfiguration.serviceSigningSpkiDer);
  }, /Ed25519 SPKI is invalid/],
  ['valid substituted service key', (_candidate, value) => {
    const spki = alternateServiceSpki();
    value.lockedConfiguration.serviceSigningSpkiDer = spki;
    value.lockedConfiguration.serviceKeyFingerprint = createHash('sha256').update(spki).digest();
  }, /configuration row is inconsistent/],
  ['configuration alias mismatch', (_candidate, value) => {
    value.lockedConfiguration.projectPrincipalId = 'attacker_principal_20260830';
  }, /configuration row is inconsistent/],
  ['authority-state scope mismatch', (_candidate, value) => {
    flip(value.lockedAuthorityState.projectAuthorityDigest);
  }, /candidate\/configuration\/authority-state binding mismatch/],
  ['authority-state configuration mismatch', (_candidate, value) => {
    flip(value.lockedAuthorityState.activeConfigurationDigest);
  }, /candidate\/configuration\/authority-state binding mismatch/],
  ['nonadjacent authority sequence', (_candidate, value) => {
    value.lockedAuthorityState.nextGlobalSequence = '2';
  }, /authority sequence adjacency is invalid/],
  ['nonempty genesis event', (_candidate, value) => {
    value.lockedAuthorityState.lastEventDigest = new Uint8Array(32).fill(1);
  }, /authority genesis state is invalid/],
  ['wrong genesis predecessor receipt', (_candidate, value) => {
    flip(value.lockedPredecessorReceipt.semanticReceiptDigest);
  }, /genesis predecessor binding mismatch/],
  ['foreign predecessor receipt', (_candidate, value) => {
    flip(value.lockedPredecessorReceipt.projectAuthorityDigest);
  }, /predecessor receipt scope mismatch/],
  ['foreign absent run', (_candidate, value) => {
    value.lockedRunState.runId = 'capture_run_attacker_20260830';
  }, /run scope mismatch/],
  ['registration prior head differs from request', (candidate) => {
    candidate.priorControllerStateHeadDigest = '2'.repeat(64);
  }, /registration run-genesis binding mismatch/],
];

const CHANGED_REPLAY_MUTATIONS: readonly Mutation[] = [
  ['changed-replay outcome drift', (candidate) => {
    candidate.body.outcomeEvidenceDigest = '1'.repeat(64);
  }, /changed-replay candidate body mismatch/],
  ['wrong semantic predecessor', (_candidate, value) => {
    flip(value.lockedPredecessorReceipt.eventDigest);
  }, /semantic predecessor binding mismatch/],
  ['wrong registered-run provenance', (_candidate, value) => {
    flip(value.lockedRunState.originalRegistrationRequestSha256);
  }, /candidate expected run does not match locked run/],
  ['split registered-run event', (_candidate, value) => {
    flip(value.lockedRunState.lastRunEventDigest);
  }, /registered run last event mismatch/],
  ['non-open registered run', (_candidate, value) => {
    value.lockedRunState.firstChangedReplayRequestDigest = new Uint8Array(32).fill(1);
  }, /locked run is not open registration state/],
  ['exhausted global sequence', (candidate, value) => {
    candidate.expectedNextGlobalSequence = '18446744073709551615';
    value.lockedAuthorityState.lastGlobalSequence = '18446744073709551614';
    value.lockedAuthorityState.nextGlobalSequence = '18446744073709551615';
  }, /candidate global sequence cannot advance/],
  ['changed replay prior head differs from locked run', (candidate) => {
    candidate.priorControllerStateHeadDigest = '2'.repeat(64);
  }, /changed-replay run binding mismatch/],
  ['changed replay previous run differs from locked run', (candidate) => {
    candidate.previousRun.eventDigest = '2'.repeat(64);
  }, /changed-replay run binding mismatch/],
  ['current request equals immutable original', async (candidate, value) => {
    const requestDigest = candidate.request.semanticRequestDigest as string;
    const requestSha256 = candidate.request.serializedSha256 as string;
    value.lockedRunState.originalRegistrationRequestDigest = digestBytes(requestDigest);
    value.lockedRunState.originalRegistrationRequestSha256 = digestBytes(requestSha256);
    candidate.expectedRunState.originalRegistrationRequestDigest = requestDigest;
    candidate.expectedRunState.originalRegistrationRequestSha256 = requestSha256;
    candidate.body.outcomeEvidenceDigest = await registrationChangedReplayEvidenceDigestV2({
      originalRegistrationRequestDigest: requestDigest,
      originalRegistrationEventDigest: candidate.body.registrationEventDigest,
      changedRegistrationRequestDigest: requestDigest,
      project: {
        projectAuthorityDigest: candidate.project.projectAuthorityDigest,
        principalId: candidate.project.principalId,
      },
      authorityHead: candidate.authorityHead,
    });
  }, /changed-replay run binding mismatch/],
  ['global sequence is not later than run predecessor', (candidate, value) => {
    candidate.expectedNextGlobalSequence = '1';
    candidate.previousGlobal = {
      kind: 'authority-genesis', eventDigest: null,
      semanticReceiptDigest: hex(value.lockedConfiguration.genesisSemanticReceiptDigest),
    };
    value.lockedAuthorityState.lastGlobalSequence = '0';
    value.lockedAuthorityState.nextGlobalSequence = '1';
    value.lockedAuthorityState.lastEventDigest = null;
    value.lockedPredecessorReceipt = {
      kind: 'authority-genesis',
      projectAuthorityDigest: copyBytes(value.lockedConfiguration.projectAuthorityDigest),
      projectScopeRole: value.lockedConfiguration.projectScopeRole,
      configurationEpoch: value.lockedConfiguration.configurationEpoch,
      configurationDigest: copyBytes(value.lockedConfiguration.configurationDigest),
      semanticReceiptDigest:
        copyBytes(value.lockedConfiguration.genesisSemanticReceiptDigest),
    };
  }, /changed-replay run binding mismatch/],
];

const REHASHED_CONFIGURATION_MUTATIONS = [
  ['invalid checkpoint quorum', (configuration: any) => {
    configuration.checkpointWitnesses.members.pop();
  }, /quorum math is invalid/],
  ['duplicate role identity', (configuration: any) => {
    configuration.service.principal.principalId =
      configuration.transparencyLog.principal.principalId;
  }, /principal identity must be unique/],
  ['duplicate role key', (configuration: any) => {
    configuration.service.principal.keyFingerprint =
      configuration.transparencyLog.principal.keyFingerprint;
  }, /role key fingerprint must be unique/],
  ['unsafe service origin', (configuration: any) => {
    configuration.service.endpointOrigin = 'https://127.0.0.1';
  }, /public credential-free HTTPS origin/],
] as const;

const SNAPSHOT_COMPONENTS = [
  'lockedConfiguration', 'lockedAuthorityState',
  'lockedPredecessorReceipt', 'lockedRunState',
] as const;

describe('PostgreSQL registration materializer V1 input boundary', () => {
  it.each(GENESIS_MUTATIONS)('rejects genesis mutant: %s', async (_name, mutate, error) => {
    const fixture = await genesisMaterializerFixtureV1();
    const candidate = structuredClone(fixture.candidate) as any;
    const snapshots = structuredClone(fixture.lockedSnapshots) as any;
    await mutate(candidate, snapshots);
    await expect(preparePostgresRegistrationMaterializationV1(candidate, snapshots))
      .rejects.toThrow(error);
  });

  it.each(CHANGED_REPLAY_MUTATIONS)(
    'rejects changed-replay mutant: %s', async (_name, mutate, error) => {
      const fixture = await changedReplayMaterializerFixtureV1();
      const candidate = structuredClone(fixture.candidate) as any;
      const snapshots = structuredClone(fixture.lockedSnapshots) as any;
      await mutate(candidate, snapshots);
      await expect(preparePostgresRegistrationMaterializationV1(candidate, snapshots))
        .rejects.toThrow(error);
    },
  );

  it.each(SNAPSHOT_COMPONENTS)('rejects %s Proxy without traps', async (component) => {
    const fixture = await genesisMaterializerFixtureV1();
    const snapshots = structuredClone(fixture.lockedSnapshots) as any;
    let traps = 0;
    snapshots[component] = new Proxy(snapshots[component], {
      get() { traps += 1; throw new Error('snapshot Proxy trap invoked'); },
    });
    await expect(preparePostgresRegistrationMaterializationV1(
      fixture.candidate, snapshots,
    )).rejects.toThrow(/acyclic data graph/);
    expect(traps).toBe(0);
  });

  it.each(SNAPSHOT_COMPONENTS)('rejects %s accessor without reads', async (component) => {
    const fixture = await genesisMaterializerFixtureV1();
    const snapshots = structuredClone(fixture.lockedSnapshots) as any;
    const key = Object.keys(snapshots[component])[0]!;
    let reads = 0;
    Object.defineProperty(snapshots[component], key, {
      enumerable: true, configurable: true,
      get() { reads += 1; throw new Error('snapshot accessor invoked'); },
    });
    await expect(preparePostgresRegistrationMaterializationV1(
      fixture.candidate, snapshots,
    )).rejects.toThrow(/enumerable data fields only/);
    expect(reads).toBe(0);
  });

  it('rejects a nested candidate Proxy without traps', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const candidate = structuredClone(fixture.candidate) as any;
    let traps = 0;
    candidate.project = new Proxy(candidate.project, {
      get() { traps += 1; throw new Error('candidate Proxy trap invoked'); },
    });
    await expect(preparePostgresRegistrationMaterializationV1(
      candidate, fixture.lockedSnapshots,
    )).rejects.toThrow(/acyclic data graph/);
    expect(traps).toBe(0);
  });

  it('rejects a candidate accessor without reads', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const candidate = structuredClone(fixture.candidate) as any;
    let reads = 0;
    Object.defineProperty(candidate.request, 'semanticRequestDigest', {
      enumerable: true, configurable: true,
      get() { reads += 1; throw new Error('candidate accessor invoked'); },
    });
    await expect(preparePostgresRegistrationMaterializationV1(
      candidate, fixture.lockedSnapshots,
    )).rejects.toThrow(/enumerable data fields only/);
    expect(reads).toBe(0);
  });

  it('rejects repeated byte aliases at the aggregate ingress budget', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const snapshots = structuredClone(fixture.lockedSnapshots) as any;
    const leaf = new Uint8Array(1_025);
    snapshots.lockedConfiguration = Array.from({ length: 1_025 }, () => leaf);
    await expect(preparePostgresRegistrationMaterializationV1(
      fixture.candidate, snapshots,
    )).rejects.toThrow(/exact bounded Uint8Array/);
  });

  it('rejects a wide dense array before enumerating its keys', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const candidate = structuredClone(fixture.candidate) as any;
    const wide = new Array(8_193).fill(null);
    candidate.body = wide;
    const ownKeys = Reflect.ownKeys;
    let wideKeysEnumerated = false;
    const spy = vi.spyOn(Reflect, 'ownKeys').mockImplementation((value) => {
      if (value === wide) wideKeysEnumerated = true;
      return ownKeys(value);
    });
    try {
      await expect(preparePostgresRegistrationMaterializationV1(
        candidate, fixture.lockedSnapshots,
      )).rejects.toThrow(/too deeply nested or large/);
      expect(wideKeysEnumerated).toBe(false);
    } finally {
      spy.mockRestore();
    }
  });

  it('rejects a wide record before materializing all descriptors', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const candidate = structuredClone(fixture.candidate) as any;
    const wide = Object.fromEntries(
      Array.from({ length: 8_193 }, (_, index) => [`field${index}`, null]),
    );
    candidate.body = wide;
    const getDescriptors = Object.getOwnPropertyDescriptors;
    let descriptorsMaterialized = false;
    const spy = vi.spyOn(Object, 'getOwnPropertyDescriptors').mockImplementation((value) => {
      if (value === wide) descriptorsMaterialized = true;
      return getDescriptors(value);
    });
    try {
      await expect(preparePostgresRegistrationMaterializationV1(
        candidate, fixture.lockedSnapshots,
      )).rejects.toThrow(/too deeply nested or large/);
      expect(descriptorsMaterialized).toBe(false);
    } finally {
      spy.mockRestore();
    }
  });

  it('rejects a graph beyond the direct depth limit', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const candidate = structuredClone(fixture.candidate) as any;
    let nested: unknown = null;
    for (let depth = 0; depth < 34; depth += 1) nested = [nested];
    candidate.body = nested;
    await expect(preparePostgresRegistrationMaterializationV1(
      candidate, fixture.lockedSnapshots,
    )).rejects.toThrow(/too deeply nested or large/);
  });

  it.each(REHASHED_CONFIGURATION_MUTATIONS)(
    'rejects fully rehashed authority configuration: %s',
    async (_name, mutate, error) => {
      const fixture = await genesisMaterializerFixtureV1();
      const configuration = configurationRecord(fixture.lockedSnapshots);
      mutate(configuration);
      await expect(Promise.resolve().then(() =>
        parsePostgresAuthorityConfigurationBytesV1(rehashedConfiguration(configuration))))
        .rejects.toThrow(error);
    },
  );

  it('rejects a registered row on the registration path', async () => {
    const genesis = await genesisMaterializerFixtureV1();
    const changed = await changedReplayMaterializerFixtureV1();
    const snapshots = structuredClone(genesis.lockedSnapshots) as any;
    snapshots.lockedRunState = structuredClone(changed.lockedSnapshots.lockedRunState);
    await expect(preparePostgresRegistrationMaterializationV1(
      genesis.candidate, snapshots,
    )).rejects.toThrow(/registration run-genesis binding mismatch/);
  });
});

function configurationRecord(snapshots: any): Record<string, any> {
  return JSON.parse(Buffer.from(
    snapshots.lockedConfiguration.serializedConfiguration,
  ).toString('utf8')) as Record<string, any>;
}

function replaceConfigurationBytes(snapshots: any, text: string): void {
  const bytes = Buffer.from(text, 'utf8');
  snapshots.lockedConfiguration.serializedConfiguration = bytes;
  snapshots.lockedConfiguration.serializedConfigurationSha256 =
    createHash('sha256').update(bytes).digest();
}

function flip(bytes: Uint8Array): void {
  bytes[bytes.length - 1] ^= 1;
}

function alternateServiceSpki(): Buffer {
  const seed = Buffer.alloc(32, 0x5a);
  const privateKey = createPrivateKey({
    key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), seed]),
    format: 'der', type: 'pkcs8',
  });
  return Buffer.from(createPublicKey(privateKey).export({ format: 'der', type: 'spki' }));
}

function rehashedConfiguration(configuration: Record<string, any>): Buffer {
  const { configurationDigest: _ignored, ...body } = configuration;
  configuration.configurationDigest = digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
    configuration: body,
  });
  return Buffer.from(`${JSON.stringify(configuration, null, 2)}\n`, 'utf8');
}

function digestBytes(value: string): Uint8Array {
  return new Uint8Array(Buffer.from(value, 'hex'));
}

function copyBytes(value: Uint8Array): Uint8Array {
  return new Uint8Array(value);
}

function hex(value: Uint8Array): string {
  return Buffer.from(value).toString('hex');
}
