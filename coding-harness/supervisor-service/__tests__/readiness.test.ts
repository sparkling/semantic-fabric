// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  supervisorServiceReadinessV1,
} from '../src/index.js';

describe('supervisor-service fail-closed readiness', () => {
  it('exposes one frozen nonoperational state without ambient overrides', () => {
    const before = process.env.SEMANTIC_FABRIC_SUPERVISOR_OPERATIONAL;
    process.env.SEMANTIC_FABRIC_SUPERVISOR_OPERATIONAL = 'true';
    try {
      const readiness = supervisorServiceReadinessV1();
      expect(readiness).toEqual({
        schemaVersion: 1,
        serviceKind: 'programme-capture-supervisor-service-v1',
        authority: 'none',
        operational: false,
        reason: 'registration-transaction-not-installed',
        externallyReachable: false,
        registrationReadEnabled: false,
        registrationMutationEnabled: false,
        databaseAccessEnabled: false,
        signerAccessEnabled: false,
        publicationAccessEnabled: false,
      });
      expect(Object.isFrozen(readiness)).toBe(true);
      expect(supervisorServiceReadinessV1()).toBe(readiness);
    } finally {
      if (before === undefined) delete process.env.SEMANTIC_FABRIC_SUPERVISOR_OPERATIONAL;
      else process.env.SEMANTIC_FABRIC_SUPERVISOR_OPERATIONAL = before;
    }
  });
});
