// SPDX-License-Identifier: MIT

import {
  parsePostgresAuthorityConfigurationBytesV1,
  postgresAuthorityGenesisHeadDigestV1,
  type PostgresAuthorityConfigurationV2,
} from './registration-postgresql-authority-configuration-v1.js';
import {
  POSTGRES_PROJECT_SCOPE_ROLE_V1,
  deepFreezeV1,
  digestHexFromBytesV1,
  closedRecordV1,
  exactKeysV1,
  parseOpaqueIdV1,
  parseUint64V1,
  rawSha256HexV1,
  snapshotBytesV1,
  successorUint64V1,
  validateEd25519SpkiV1,
} from './registration-postgresql-canonical-v1.js';

export interface NormalizedLockedConfigurationV1 {
  readonly projectAuthorityDigest: string;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly configurationEpoch: string;
  readonly configurationDigest: string;
  readonly genesisAuthorityHeadDigest: string;
  readonly serializedConfiguration: Uint8Array;
  readonly serializedConfigurationSha256: string;
  readonly projectPrincipalId: string;
  readonly projectAuthenticationPolicyDigest: string;
  readonly servicePrincipalId: string;
  readonly serviceKeyEpoch: string;
  readonly serviceKeyFingerprint: string;
  readonly serviceSigningSpkiDer: Uint8Array;
  readonly genesisSemanticReceiptDigest: string;
  readonly configuration: PostgresAuthorityConfigurationV2;
}

export interface NormalizedAuthorityStateV1 {
  readonly projectAuthorityDigest: string;
  readonly projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
  readonly singletonKey: true;
  readonly activeConfigurationEpoch: string;
  readonly activeConfigurationDigest: string;
  readonly authorityHeadDigest: string;
  readonly lastGlobalSequence: string;
  readonly nextGlobalSequence: string;
  readonly lastEventDigest: string | null;
}

export type NormalizedPredecessorReceiptV1 = Readonly<
  | {
    kind: 'authority-genesis'; projectAuthorityDigest: string;
    projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
    configurationEpoch: string; configurationDigest: string;
    semanticReceiptDigest: string;
  }
  | {
    kind: 'semantic-event'; projectAuthorityDigest: string;
    projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1;
    eventDigest: string; semanticReceiptDigest: string;
  }
>;

export type NormalizedRunStateV1 = Readonly<
  | {
    kind: 'absent'; projectAuthorityDigest: string;
    projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1; runId: string;
  }
  | {
    kind: 'registered'; projectAuthorityDigest: string;
    projectScopeRole: typeof POSTGRES_PROJECT_SCOPE_ROLE_V1; runId: string;
    originalRegistrationRequestDigest: string;
    originalRegistrationRequestSha256: string;
    originalRegistrationEventDigest: string;
    lastRunEventDigest: string; lastRunGlobalSequence: string;
    currentControllerStateHeadDigest: string; lastRunSequence: '0';
    firstChangedReplayRequestDigest: null;
  }
>;

export function parseLockedConfigurationV1(value: unknown): NormalizedLockedConfigurationV1 {
  const input = closedRecordV1(value, 'locked authority configuration row');
  exactKeysV1(input, [
    'projectAuthorityDigest', 'projectScopeRole', 'configurationEpoch',
    'configurationDigest', 'genesisAuthorityHeadDigest', 'serializedConfiguration',
    'serializedConfigurationSha256', 'projectPrincipalId',
    'projectAuthenticationPolicyDigest', 'servicePrincipalId', 'serviceKeyEpoch',
    'serviceKeyFingerprint', 'serviceSigningSpkiDer', 'genesisSemanticReceiptDigest',
  ], 'locked authority configuration row');
  const serializedConfiguration = snapshotBytesV1(
    input.serializedConfiguration, 'serialized authority configuration', 131_072,
  );
  const configuration = parsePostgresAuthorityConfigurationBytesV1(serializedConfiguration);
  const projectAuthorityDigest = digestHexFromBytesV1(
    input.projectAuthorityDigest, 'configuration project digest',
  );
  const configurationDigest = digestHexFromBytesV1(
    input.configurationDigest, 'configuration digest row',
  );
  const serviceKeyFingerprint = digestHexFromBytesV1(
    input.serviceKeyFingerprint, 'service key fingerprint row',
  );
  const normalized = {
    projectAuthorityDigest,
    projectScopeRole: parseScope(input.projectScopeRole),
    configurationEpoch: parseUint64V1(input.configurationEpoch, 'configuration epoch'),
    configurationDigest,
    genesisAuthorityHeadDigest: digestHexFromBytesV1(
      input.genesisAuthorityHeadDigest, 'genesis authority head row',
    ),
    serializedConfiguration,
    serializedConfigurationSha256: digestHexFromBytesV1(
      input.serializedConfigurationSha256, 'configuration byte hash row',
    ),
    projectPrincipalId: parseOpaqueIdV1(input.projectPrincipalId, 'configuration project ID'),
    projectAuthenticationPolicyDigest: digestHexFromBytesV1(
      input.projectAuthenticationPolicyDigest, 'configuration authentication policy row',
    ),
    servicePrincipalId: parseOpaqueIdV1(input.servicePrincipalId, 'service principal row'),
    serviceKeyEpoch: parseUint64V1(input.serviceKeyEpoch, 'service key epoch row', 1n),
    serviceKeyFingerprint,
    serviceSigningSpkiDer: validateEd25519SpkiV1(
      input.serviceSigningSpkiDer, serviceKeyFingerprint,
    ),
    genesisSemanticReceiptDigest: digestHexFromBytesV1(
      input.genesisSemanticReceiptDigest, 'genesis semantic receipt row',
    ),
    configuration,
  };
  if (normalized.configurationEpoch !== '0'
    || normalized.configurationEpoch !== configuration.configurationEpoch
    || configurationDigest !== configuration.configurationDigest
    || normalized.genesisAuthorityHeadDigest
      !== postgresAuthorityGenesisHeadDigestV1(configuration)
    || normalized.serializedConfigurationSha256 !== rawSha256HexV1(serializedConfiguration)
    || projectAuthorityDigest !== configuration.project.projectAuthorityDigest
    || normalized.projectPrincipalId !== configuration.project.principal.principalId
    || normalized.projectAuthenticationPolicyDigest
      !== configuration.project.authenticationPolicyDigest
    || normalized.servicePrincipalId !== configuration.service.principal.principalId
    || normalized.serviceKeyEpoch !== configuration.service.principal.keyEpoch
    || serviceKeyFingerprint !== configuration.service.principal.keyFingerprint) {
    throw new TypeError('locked authority configuration row is inconsistent');
  }
  return deepFreezeV1(normalized);
}

export function parseAuthorityStateV1(value: unknown): NormalizedAuthorityStateV1 {
  const input = closedRecordV1(value, 'locked authority state row');
  exactKeysV1(input, [
    'projectAuthorityDigest', 'projectScopeRole', 'singletonKey',
    'activeConfigurationEpoch', 'activeConfigurationDigest', 'authorityHeadDigest',
    'lastGlobalSequence', 'nextGlobalSequence', 'lastEventDigest',
  ], 'locked authority state row');
  if (input.singletonKey !== true) throw new TypeError('authority singleton is invalid');
  const lastGlobalSequence = parseUint64V1(input.lastGlobalSequence, 'last global sequence');
  const nextGlobalSequence = parseUint64V1(input.nextGlobalSequence, 'next global sequence', 1n);
  if (successorUint64V1(lastGlobalSequence, 'last global sequence') !== nextGlobalSequence) {
    throw new TypeError('authority sequence adjacency is invalid');
  }
  const lastEventDigest = input.lastEventDigest === null ? null
    : digestHexFromBytesV1(input.lastEventDigest, 'last authority event');
  if ((lastGlobalSequence === '0') !== (lastEventDigest === null)) {
    throw new TypeError('authority genesis state is invalid');
  }
  return deepFreezeV1({
    projectAuthorityDigest: digestHexFromBytesV1(input.projectAuthorityDigest, 'state project'),
    projectScopeRole: parseScope(input.projectScopeRole),
    singletonKey: true as const,
    activeConfigurationEpoch: parseUint64V1(input.activeConfigurationEpoch, 'active epoch'),
    activeConfigurationDigest: digestHexFromBytesV1(
      input.activeConfigurationDigest, 'active configuration digest',
    ),
    authorityHeadDigest: digestHexFromBytesV1(input.authorityHeadDigest, 'authority head'),
    lastGlobalSequence,
    nextGlobalSequence,
    lastEventDigest,
  });
}

export function parsePredecessorReceiptV1(value: unknown): NormalizedPredecessorReceiptV1 {
  const input = closedRecordV1(value, 'locked predecessor receipt');
  if (input.kind === 'authority-genesis') {
    exactKeysV1(input, [
      'kind', 'projectAuthorityDigest', 'projectScopeRole', 'configurationEpoch',
      'configurationDigest', 'semanticReceiptDigest',
    ], 'locked genesis receipt');
    return deepFreezeV1({
      kind: 'authority-genesis' as const,
      projectAuthorityDigest: digestHexFromBytesV1(input.projectAuthorityDigest, 'receipt project'),
      projectScopeRole: parseScope(input.projectScopeRole),
      configurationEpoch: parseUint64V1(input.configurationEpoch, 'receipt config epoch'),
      configurationDigest: digestHexFromBytesV1(input.configurationDigest, 'receipt config'),
      semanticReceiptDigest: digestHexFromBytesV1(
        input.semanticReceiptDigest, 'genesis semantic receipt',
      ),
    });
  }
  exactKeysV1(input, [
    'kind', 'projectAuthorityDigest', 'projectScopeRole', 'eventDigest',
    'semanticReceiptDigest',
  ], 'locked semantic receipt');
  if (input.kind !== 'semantic-event') throw new TypeError('predecessor receipt kind invalid');
  return deepFreezeV1({
    kind: 'semantic-event' as const,
    projectAuthorityDigest: digestHexFromBytesV1(input.projectAuthorityDigest, 'receipt project'),
    projectScopeRole: parseScope(input.projectScopeRole),
    eventDigest: digestHexFromBytesV1(input.eventDigest, 'receipt event'),
    semanticReceiptDigest: digestHexFromBytesV1(
      input.semanticReceiptDigest, 'semantic receipt digest',
    ),
  });
}

export function parseRunStateV1(value: unknown): NormalizedRunStateV1 {
  const input = closedRecordV1(value, 'locked registration run');
  if (input.kind === 'absent') {
    exactKeysV1(input, ['kind', 'projectAuthorityDigest', 'projectScopeRole', 'runId'],
      'locked absent run');
    return deepFreezeV1({
      kind: 'absent' as const,
      projectAuthorityDigest: digestHexFromBytesV1(input.projectAuthorityDigest, 'run project'),
      projectScopeRole: parseScope(input.projectScopeRole),
      runId: parseOpaqueIdV1(input.runId, 'absent run ID'),
    });
  }
  exactKeysV1(input, [
    'kind', 'projectAuthorityDigest', 'projectScopeRole', 'runId',
    'originalRegistrationRequestDigest', 'originalRegistrationRequestSha256',
    'originalRegistrationEventDigest', 'lastRunEventDigest', 'lastRunGlobalSequence',
    'currentControllerStateHeadDigest', 'lastRunSequence',
    'firstChangedReplayRequestDigest',
  ], 'locked registered run');
  if (input.kind !== 'registered' || input.lastRunSequence !== '0'
    || input.firstChangedReplayRequestDigest !== null) {
    throw new TypeError('locked run is not open registration state');
  }
  const event = digestHexFromBytesV1(input.originalRegistrationEventDigest, 'registration event');
  const lastEvent = digestHexFromBytesV1(input.lastRunEventDigest, 'last run event');
  if (event !== lastEvent) throw new TypeError('registered run last event mismatch');
  return deepFreezeV1({
    kind: 'registered' as const,
    projectAuthorityDigest: digestHexFromBytesV1(input.projectAuthorityDigest, 'run project'),
    projectScopeRole: parseScope(input.projectScopeRole),
    runId: parseOpaqueIdV1(input.runId, 'registered run ID'),
    originalRegistrationRequestDigest: digestHexFromBytesV1(
      input.originalRegistrationRequestDigest, 'original request',
    ),
    originalRegistrationRequestSha256: digestHexFromBytesV1(
      input.originalRegistrationRequestSha256, 'original request bytes',
    ),
    originalRegistrationEventDigest: event,
    lastRunEventDigest: lastEvent,
    lastRunGlobalSequence: parseUint64V1(input.lastRunGlobalSequence, 'last run global', 1n),
    currentControllerStateHeadDigest: digestHexFromBytesV1(
      input.currentControllerStateHeadDigest, 'current run head',
    ),
    lastRunSequence: '0' as const,
    firstChangedReplayRequestDigest: null,
  });
}

function parseScope(value: unknown): typeof POSTGRES_PROJECT_SCOPE_ROLE_V1 {
  if (value !== POSTGRES_PROJECT_SCOPE_ROLE_V1) throw new TypeError('project scope role invalid');
  return POSTGRES_PROJECT_SCOPE_ROLE_V1;
}
