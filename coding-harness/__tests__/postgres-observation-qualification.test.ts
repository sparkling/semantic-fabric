import { describe, expect, it } from 'vitest';
import {
  createPostgresObservationQualificationReceipt,
  verifyPostgresObservationQualificationReceipt,
} from '../src/postgres-observation-qualification.js';

const input = {
  image: { repository: 'postgres', digest: 'sha256:' + 'a'.repeat(64), platform: 'linux/amd64' as const },
  serverVersion: 'PostgreSQL 16.15', serverVersionNum: 160015 as const,
  source: { commit: 'a'.repeat(40), tree: 'b'.repeat(40), cargoLock: 'c'.repeat(64) },
  inputs: { adr: 'd'.repeat(64), profile: 'e'.repeat(64), queries: 'f'.repeat(64), tests: '0'.repeat(64) },
  guard: 'pass' as const, rowCounts: { relations: 1, attributes: 1, constraints: 1 },
  overflow: { relations: false, attributes: false, constraints: false },
  identity: { structural: '1'.repeat(64), types: '2'.repeat(64), constraints: '3'.repeat(64) },
  legacyComparison: 'equal' as const, errorCode: null,
};

describe('PostgreSQL observation qualification receipt', () => {
  it('creates and verifies a replayable pass receipt', () => {
    const receipt = createPostgresObservationQualificationReceipt(input);
    expect(receipt.replayStatus).toBe('pass');
    expect(() => verifyPostgresObservationQualificationReceipt(receipt)).not.toThrow();
  });
  it('rejects tampered receipt bytes', () => {
    const receipt = createPostgresObservationQualificationReceipt(input);
    expect(() => verifyPostgresObservationQualificationReceipt({ ...receipt, serverVersion: 'tampered' })).toThrow();
  });
});
