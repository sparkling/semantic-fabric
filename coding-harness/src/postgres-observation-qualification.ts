import { canonical, hash } from '@metaharness/harness';

export interface PostgresObservationQualificationInput {
  image: { repository: string; digest: string; platform: 'linux/amd64' };
  serverVersion: string;
  serverVersionNum: 160009 | 160015;
  source: { commit: string; tree: string; cargoLock: string };
  inputs: { adr: string; profile: string; queries: string; tests: string };
  guard: 'pass' | 'fail';
  rowCounts: { relations: number; attributes: number; constraints: number };
  overflow: { relations: boolean; attributes: boolean; constraints: boolean };
  identity: { structural: string; types: string; constraints: string } | null;
  legacyComparison: 'equal' | 'unavailable';
  errorCode: string | null;
}

export interface PostgresObservationQualificationReceipt
  extends PostgresObservationQualificationInput {
  receiptKind: 'postgresql-public-observation-qualification-v1';
  replayStatus: 'pass' | 'fail';
  receiptSha256: string;
}

export function createPostgresObservationQualificationReceipt(
  input: PostgresObservationQualificationInput,
): PostgresObservationQualificationReceipt {
  const replayStatus: 'pass' | 'fail' = input.guard === 'pass'
    && input.errorCode === null
    && input.identity !== null
    && input.legacyComparison === 'equal'
    && !Object.values(input.overflow).some(Boolean)
    ? 'pass' : 'fail';
  const body = { receiptKind: 'postgresql-public-observation-qualification-v1' as const, ...input, replayStatus };
  return { ...body, receiptSha256: hash(canonical(body)) };
}

export function verifyPostgresObservationQualificationReceipt(
  receipt: PostgresObservationQualificationReceipt,
): void {
  const { receiptSha256, ...body } = receipt;
  if (receiptSha256 !== hash(canonical(body))) throw new Error('qualification receipt digest mismatch');
  if (receipt.replayStatus === 'pass' && (receipt.guard !== 'pass' || receipt.identity === null)) {
    throw new Error('qualification receipt pass has incomplete evidence');
  }
}
