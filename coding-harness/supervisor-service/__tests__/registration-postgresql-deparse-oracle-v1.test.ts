// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { validatePostgresCatalogueCoreV1 }
  from '../src/registration-postgresql-catalogue-core-v1.js';
import { reconstructPostgresCatalogueShapeV1 }
  from '../src/registration-postgresql-catalogue-shape-v1.js';
import {
  constraintDefinitionV1,
  domainCheckExpressionV1,
  tableCheckExpressionV1,
} from '../src/registration-postgresql-catalogue-templates-v1.js';
import {
  type PostgresDeparseFactV1,
  POSTGRES_16_15_DEPARSE_ORACLE_V1,
} from './registration-postgresql-deparse-oracle-v1.js';

const CONTRACT_TEXT = readFileSync(
  new URL('../migrations/catalog-contract-v1.json', import.meta.url),
  'utf8',
);

interface DomainCheck {
  name: string;
  template: string;
  expression: string;
}
interface Domain {
  schema: string;
  name: string;
  checks: DomainCheck[];
}
interface Constraint {
  schema: string;
  relation: string;
  name: string;
  kind: string;
  columns: string[];
  referencedSchema: string | null;
  referencedRelation: string | null;
  referencedColumns: string[] | null;
  updateAction: string | null;
  deleteAction: string | null;
  deferrable: boolean;
  initiallyDeferred: boolean;
  definition: string;
  checkTemplate: string | null;
  expression: string | null;
}
interface Catalogue {
  domains: Domain[];
  constraints: Constraint[];
}
interface Candidate {
  readonly identity: string;
  readonly value: string;
}

describe('PostgreSQL 16.15 independently captured deparse oracle V1', () => {
  it('pins the version, settings, disjoint counts, and 85 unique identities', () => {
    const oracle = POSTGRES_16_15_DEPARSE_ORACLE_V1;
    const facts = allFacts();

    expect(oracle).toMatchObject({
      serverVersionNum: 160_015,
      searchPath: 'pg_catalog',
      pretty: false,
    });
    expect([
      oracle.domainCheckExpressions.length,
      oracle.constraintDefinitions.length,
      oracle.tableCheckExpressions.length,
      facts.length,
      new Set(facts.map((fact) => fact.identity)).size,
    ]).toEqual([10, 60, 15, 85, 85]);
  });

  it('binds all ten domain templates to literal PostgreSQL 16.15 facts', () => {
    const contract = parseContract();
    const candidates = contract.domains.flatMap((domain) => domain.checks.map((check) => {
      const generated = domainCheckExpressionV1(check.template);
      expect(generated, check.name).toBe(check.expression);
      return {
        identity: `domain-check-expression:${domain.schema}.${domain.name}.${check.name}`,
        value: generated,
      };
    }));

    expectCandidates(candidates, POSTGRES_16_15_DEPARSE_ORACLE_V1.domainCheckExpressions);
  });

  it('binds all 60 generated constraint definitions to literal 16.15 facts', () => {
    const contract = parseContract();
    const candidates = contract.constraints.map((constraint) => {
      const generated = constraintDefinitionV1({
        kind: constraint.kind,
        columns: constraint.columns,
        referencedSchema: constraint.referencedSchema,
        referencedRelation: constraint.referencedRelation,
        referencedColumns: constraint.referencedColumns,
        updateAction: constraint.updateAction,
        deleteAction: constraint.deleteAction,
        deferrable: constraint.deferrable,
        initiallyDeferred: constraint.initiallyDeferred,
        expression: constraint.expression,
      });
      expect(generated, constraint.name).toBe(constraint.definition);
      return {
        identity: `constraint-definition:${constraint.schema}.${constraint.relation}.${constraint.name}`,
        value: generated,
      };
    });

    expectCandidates(candidates, POSTGRES_16_15_DEPARSE_ORACLE_V1.constraintDefinitions);
  });

  it('binds all 15 table-check templates to separate literal expression facts', () => {
    const contract = parseContract();
    const candidates = contract.constraints
      .filter((constraint) => constraint.kind === 'check')
      .map((constraint) => {
        expect(constraint.checkTemplate, constraint.name).not.toBeNull();
        const generated = tableCheckExpressionV1(constraint.checkTemplate!);
        expect(generated, constraint.name).toBe(constraint.expression);
        return {
          identity: `table-check-expression:${constraint.schema}.${constraint.relation}.${constraint.name}`,
          value: generated,
        };
      });

    expectCandidates(candidates, POSTGRES_16_15_DEPARSE_ORACLE_V1.tableCheckExpressions);
  });

  it('makes every one of the 85 stored deparse facts semantically load-bearing', () => {
    const baseline = parseContract();
    expectCoreValid(baseline);

    for (let index = 0; index < baseline.domains.length; index += 1) {
      const mutant = parseContract();
      mutant.domains[index]!.checks[0]!.expression += ' ';
      expectCoreInvalid(mutant, `domain expression ${index}`);
    }
    for (let index = 0; index < baseline.constraints.length; index += 1) {
      const mutant = parseContract();
      mutant.constraints[index]!.definition += ' ';
      expectCoreInvalid(mutant, `constraint definition ${index}`);
    }
    for (const [index, constraint] of baseline.constraints.entries()) {
      if (constraint.kind !== 'check') continue;
      const mutant = parseContract();
      mutant.constraints[index]!.expression += ' ';
      expectCoreInvalid(mutant, `table check expression ${index}`);
    }
  });
});

function parseContract(): Catalogue {
  return JSON.parse(CONTRACT_TEXT) as Catalogue;
}

function allFacts(): readonly PostgresDeparseFactV1[] {
  const oracle = POSTGRES_16_15_DEPARSE_ORACLE_V1;
  return [
    ...oracle.domainCheckExpressions,
    ...oracle.constraintDefinitions,
    ...oracle.tableCheckExpressions,
  ];
}

function expectCandidates(
  candidates: readonly Candidate[],
  facts: readonly PostgresDeparseFactV1[],
): void {
  expect(candidates.map(({ identity }) => identity))
    .toEqual(facts.map(({ identity }) => identity));
  for (const [index, candidate] of candidates.entries()) {
    const fact = facts[index]!;
    expect(Buffer.byteLength(candidate.value, 'utf8'), fact.identity).toBe(fact.byteLength);
    expect(sha256(candidate.value), fact.identity).toBe(fact.sha256);
  }
}

function expectCoreValid(value: Catalogue): void {
  expect(() => validatePostgresCatalogueCoreV1(
    reconstructPostgresCatalogueShapeV1(value),
  )).not.toThrow();
}

function expectCoreInvalid(value: Catalogue, label: string): void {
  expect(() => validatePostgresCatalogueCoreV1(
    reconstructPostgresCatalogueShapeV1(value),
  ), label).toThrowError('PostgreSQL catalogue contract is invalid');
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}
