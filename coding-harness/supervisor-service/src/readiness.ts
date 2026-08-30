// SPDX-License-Identifier: MIT

export interface SupervisorServiceReadinessV1 {
  readonly schemaVersion: 1;
  readonly serviceKind: 'programme-capture-supervisor-service-v1';
  readonly authority: 'none';
  readonly operational: false;
  readonly reason: 'registration-transaction-not-installed';
  readonly externallyReachable: false;
  readonly registrationReadEnabled: false;
  readonly registrationMutationEnabled: false;
  readonly databaseAccessEnabled: false;
  readonly signerAccessEnabled: false;
  readonly publicationAccessEnabled: false;
}

const READINESS = Object.freeze({
  schemaVersion: 1 as const,
  serviceKind: 'programme-capture-supervisor-service-v1' as const,
  authority: 'none' as const,
  operational: false as const,
  reason: 'registration-transaction-not-installed' as const,
  externallyReachable: false as const,
  registrationReadEnabled: false as const,
  registrationMutationEnabled: false as const,
  databaseAccessEnabled: false as const,
  signerAccessEnabled: false as const,
  publicationAccessEnabled: false as const,
});

export function supervisorServiceReadinessV1(): SupervisorServiceReadinessV1 {
  return READINESS;
}
