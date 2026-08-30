// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import { rawSha256HexV1 }
  from './registration-postgresql-canonical-v1.js';
import { validatePostgresCatalogueCoreV1 }
  from './registration-postgresql-catalogue-core-v1.js';
import { validatePostgresCatalogueQueryV1 }
  from './registration-postgresql-catalogue-query-v1.js';
import { scanPostgresCatalogueBytesV1 }
  from './registration-postgresql-catalogue-scanner-v1.js';
import { validatePostgresCatalogueSecurityV1 }
  from './registration-postgresql-catalogue-security-v1.js';
import {
  type CatalogueRecordV1,
  POSTGRES_CATALOGUE_ROOT_KEYS_V1,
  reconstructPostgresCatalogueShapeV1,
} from './registration-postgresql-catalogue-shape-v1.js';
import { canonicalRootLineJsonV1 }
  from './registration-postgresql-catalogue-values-v1.js';

const INVALID = 'PostgreSQL catalogue contract is invalid';
const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const ZERO_DIGEST = '0'.repeat(64);
const PARSED_HANDLES = new WeakSet<object>();

export interface ParsedPostgresCatalogueContractV1 {
  readonly rawByteLength: number;
  readonly rawSha256: string;
  readonly scan: Readonly<{
    nodes: number;
    records: number;
    maximumDepth: number;
  }>;
  readonly contract: CatalogueRecordV1;
  readonly authority: 'none';
  readonly readinessAuthorized: false;
}

/**
 * Parse and validate the private build-time catalogue oracle.
 *
 * No partial value or digest escapes on failure. The returned handle confers
 * no migration, persistence, observation, or readiness authority.
 */
export function parsePostgresCatalogueContractV1(
  value: unknown,
): ParsedPostgresCatalogueContractV1 {
  try {
    const scanned = scanPostgresCatalogueBytesV1(value);
    let parsed: unknown;
    try {
      parsed = JSON.parse(scanned.text) as unknown;
    } catch {
      throw new TypeError();
    }

    const contract = reconstructPostgresCatalogueShapeV1(parsed);
    validatePostgresCatalogueCoreV1(contract);
    validatePostgresCatalogueSecurityV1(contract);
    validatePostgresCatalogueQueryV1(contract);

    const replay = canonicalRootLineJsonV1(contract, POSTGRES_CATALOGUE_ROOT_KEYS_V1);
    if (replay !== scanned.text) throw new TypeError();

    const replayBytes = new TextEncoder().encode(replay);
    const handle: ParsedPostgresCatalogueContractV1 = Object.freeze({
      rawByteLength: replayBytes.byteLength,
      rawSha256: rawSha256HexV1(replayBytes),
      scan: Object.freeze({
        nodes: scanned.nodes,
        records: scanned.records,
        maximumDepth: scanned.maximumDepth,
      }),
      contract,
      authority: 'none',
      readinessAuthorized: false,
    });
    PARSED_HANDLES.add(handle);
    return handle;
  } catch {
    throw new TypeError(INVALID);
  }
}

/** Compare a branded parsed handle with a caller-supplied manifest digest. */
export function assertPostgresCatalogueDigestV1(
  value: unknown,
  expectedDigest: unknown,
): asserts value is ParsedPostgresCatalogueContractV1 {
  try {
    if (isProxy(value) || value === null || typeof value !== 'object'
      || !PARSED_HANDLES.has(value)
      || typeof expectedDigest !== 'string'
      || !DIGEST_PATTERN.test(expectedDigest)
      || expectedDigest === ZERO_DIGEST
      || (value as ParsedPostgresCatalogueContractV1).rawSha256 !== expectedDigest) {
      throw new TypeError();
    }
  } catch {
    throw new TypeError(INVALID);
  }
}
