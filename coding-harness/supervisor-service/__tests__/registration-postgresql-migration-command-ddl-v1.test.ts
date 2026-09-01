// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { assertPostgresMigrationCommandDdlV1 }
  from '../src/registration-postgresql-migration-command-ddl-v1.js';
import { copyPostgresMigrationCommandCatalogueV1 }
  from '../src/registration-postgresql-migration-command-catalogue-v1.js';
import { loadSealedPostgresMigrationPlanV1 }
  from '../src/registration-postgresql-migration-plan-v1.js';

const SERVICE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const DDL_TEXT = readFileSync(resolve(
  SERVICE_ROOT, 'migrations/0001-registration-state-v1.sql',
), 'utf8');
const CONTRACT_TEXT = readFileSync(resolve(
  SERVICE_ROOT, 'migrations/catalog-contract-v1.json',
), 'utf8');
const SOURCE = resolve(
  SERVICE_ROOT, 'src/registration-postgresql-migration-command-ddl-v1.ts',
);
const INVALID = 'PostgreSQL migration command DDL is invalid';

interface DomainV1 {
  schema: string;
  name: string;
  baseTypeSchema: string;
  baseTypeName: string;
}
interface ColumnV1 {
  schema: string;
  relation: string;
  ordinal: number;
  name: string;
  typeSchema: string;
  typeName: string;
}
interface ContractV1 {
  domains: DomainV1[];
  columns: ColumnV1[];
}
interface ParameterV1 {
  position: number;
  baseType: string;
  source: string;
  representation: string;
}
interface CommandV1 {
  descriptorKind: string;
  operation: string;
  text: string;
  parameters: ParameterV1[];
}
interface CatalogueV1 {
  insertCommands: CommandV1[];
  [key: string]: unknown;
}
interface CommandShapeV1 {
  schema: string;
  relation: string;
  columns: string[];
  casts: string[];
}
interface PairV1 {
  commandIndex: number;
  left: number;
  right: number;
  schema: string;
  relation: string;
}

describe('PostgreSQL migration INSERT structural DDL assertion V1', () => {
  it('triangulates the branded Plan commands with independent raw DDL and contract facts', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const catalogue = copyPostgresMigrationCommandCatalogueV1(plan) as unknown as CatalogueV1;
    const contract = JSON.parse(CONTRACT_TEXT) as ContractV1;

    expect(() => independentStructuralOracleV1(
      DDL_TEXT, contract, catalogue.insertCommands,
    )).not.toThrow();
    expect(assertPostgresMigrationCommandDdlV1(plan)).toBeUndefined();
  });

  it('rejects every coherent same-base-type DDL/catalogue positional swap', () => {
    const fixture = fixtureV1();
    const pairs = sameBaseTypePairsV1(fixture.contract, fixture.catalogue.insertCommands);
    expect(pairs).toHaveLength(55);

    for (const pair of pairs) {
      const contract = swapContractColumnsV1(fixture.contract, pair);
      const ddl = swapDdlColumnsV1(DDL_TEXT, pair);
      expect(() => independentStructuralOracleV1(
        ddl, contract, fixture.catalogue.insertCommands,
      ), pairLabelV1('ddl/catalogue', pair)).toThrow();
    }
  });

  it('rejects every coherent same-base-type command column/source binding swap', () => {
    const fixture = fixtureV1();
    const pairs = sameBaseTypePairsV1(fixture.contract, fixture.catalogue.insertCommands);
    expect(pairs).toHaveLength(55);

    for (const pair of pairs) {
      const catalogue = swapCommandBindingV1(fixture.catalogue, pair);
      expect(() => independentStructuralOracleV1(
        DDL_TEXT, fixture.contract, catalogue.insertCommands,
      ), pairLabelV1('command', pair)).toThrow();
    }
  });

  it('accepts only the branded Plan and emits one fixed no-echo failure', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const proxy = new Proxy(plan, { get: () => { throw new Error('secret trap'); } });
    for (const candidate of [structuredClone(plan), proxy, new Uint8Array([1]), null]) {
      expect(() => assertPostgresMigrationCommandDdlV1(candidate))
        .toThrowError(INVALID);
    }
  });

  it('has no I/O, driver, execution, store, Promise, or public-bundle surface', async () => {
    const source = readFileSync(SOURCE, 'utf8');
    expect(source).not.toMatch(/node:fs|readFile|writeFile|from ['"](?:pg|postgres)['"]/);
    expect(source).not.toMatch(/\b(?:execute|store|Promise)\b/);
    expect(source.match(/^export /gm)).toHaveLength(1);
    expect(await import('../src/index.js'))
      .not.toHaveProperty('assertPostgresMigrationCommandDdlV1');
  });
});

function fixtureV1() {
  const plan = loadSealedPostgresMigrationPlanV1();
  return {
    catalogue: structuredClone(
      copyPostgresMigrationCommandCatalogueV1(plan),
    ) as unknown as CatalogueV1,
    contract: JSON.parse(CONTRACT_TEXT) as ContractV1,
  };
}

function independentStructuralOracleV1(
  ddl: string, contract: ContractV1, commands: readonly CommandV1[],
): void {
  if (commands.length !== 4) throw new TypeError();
  for (const command of commands) {
    const shape = commandShapeV1(command);
    const contractColumns = relationColumnsV1(contract, shape.schema, shape.relation);
    const ddlColumns = ddlColumnsV1(ddl, shape.schema, shape.relation);
    if (shape.columns.length !== contractColumns.length
      || ddlColumns.length !== contractColumns.length
      || command.parameters.length !== contractColumns.length) throw new TypeError();
    for (let index = 0; index < contractColumns.length; index += 1) {
      const column = contractColumns[index]!;
      const ddlColumn = ddlColumns[index]!;
      const baseType = resolvedBaseTypeV1(contract, column);
      const parameter = command.parameters[index]!;
      if (column.ordinal !== index + 1 || ddlColumn.ordinal !== index + 1
        || ddlColumn.name !== column.name || ddlColumn.typeSchema !== column.typeSchema
        || ddlColumn.typeName !== column.typeName || shape.columns[index] !== column.name
        || shape.casts[index] !== baseType || parameter.position !== index + 1
        || parameter.baseType !== baseType) throw new TypeError();
    }
  }
}

function commandShapeV1(command: CommandV1): CommandShapeV1 {
  const match = /^INSERT INTO ([a-z_][a-z0-9_$]*)\.([a-z_][a-z0-9_$]*) \(\n([\s\S]*?)\n\) VALUES \(\n([\s\S]*?)\n\)\n$/.exec(command.text);
  if (!match) throw new TypeError();
  const columns = match[3]!.split(',\n').map((line) => {
    const column = /^  ([a-z_][a-z0-9_$]*)$/.exec(line);
    if (!column) throw new TypeError();
    return column[1]!;
  });
  const casts = match[4]!.split(',\n').map((line, index) => {
    const cast = /^  \$([1-9][0-9]*)::pg_catalog\.([a-z_][a-z0-9_$]*)$/.exec(line);
    if (!cast || Number(cast[1]) !== index + 1) throw new TypeError();
    return cast[2]!;
  });
  return { schema: match[1]!, relation: match[2]!, columns, casts };
}

function ddlColumnsV1(ddl: string, schema: string, relation: string): ColumnV1[] {
  const header = `CREATE TABLE ${schema}.${relation} (`;
  const lines = ddl.split('\n');
  const start = lines.indexOf(header);
  if (start < 0 || lines.indexOf(header, start + 1) >= 0) throw new TypeError();
  const columns: ColumnV1[] = [];
  for (let index = start + 1; index < lines.length; index += 1) {
    const line = lines[index]!;
    if (line.startsWith('  CONSTRAINT ')) break;
    const match = /^  ([a-z_][a-z0-9_$]*) ([a-z_][a-z0-9_$]*)\.([a-z_][a-z0-9_$]*)(?: COLLATE pg_catalog\."C")?(?: NOT NULL)?,?$/.exec(line);
    if (!match) throw new TypeError();
    columns.push({
      schema, relation, ordinal: columns.length + 1, name: match[1]!,
      typeSchema: match[2]!, typeName: match[3]!,
    });
  }
  if (columns.length === 0) throw new TypeError();
  return columns;
}

function relationColumnsV1(
  contract: ContractV1, schema: string, relation: string,
): ColumnV1[] {
  return contract.columns.filter((column) =>
    column.schema === schema && column.relation === relation)
    .sort((left, right) => left.ordinal - right.ordinal);
}

function resolvedBaseTypeV1(contract: ContractV1, column: ColumnV1): string {
  if (column.typeSchema === 'pg_catalog') return column.typeName;
  const domains = contract.domains.filter((domain) =>
    domain.schema === column.typeSchema && domain.name === column.typeName);
  if (domains.length !== 1 || domains[0]!.baseTypeSchema !== 'pg_catalog') throw new TypeError();
  return domains[0]!.baseTypeName;
}

function sameBaseTypePairsV1(
  contract: ContractV1, commands: readonly CommandV1[],
): PairV1[] {
  const pairs: PairV1[] = [];
  commands.forEach((command, commandIndex) => {
    const shape = commandShapeV1(command);
    const columns = relationColumnsV1(contract, shape.schema, shape.relation);
    for (let left = 0; left < columns.length; left += 1) {
      for (let right = left + 1; right < columns.length; right += 1) {
        if (resolvedBaseTypeV1(contract, columns[left]!)
          === resolvedBaseTypeV1(contract, columns[right]!)) {
          pairs.push({ commandIndex, left, right, schema: shape.schema, relation: shape.relation });
        }
      }
    }
  });
  return pairs;
}

function swapContractColumnsV1(contract: ContractV1, pair: PairV1): ContractV1 {
  const mutant = structuredClone(contract);
  const indexes = mutant.columns.map((column, index) => ({ column, index }))
    .filter(({ column }) => column.schema === pair.schema && column.relation === pair.relation)
    .sort((left, right) => left.column.ordinal - right.column.ordinal);
  const left = indexes[pair.left]!;
  const right = indexes[pair.right]!;
  mutant.columns[left.index] = { ...right.column, ordinal: left.column.ordinal };
  mutant.columns[right.index] = { ...left.column, ordinal: right.column.ordinal };
  return mutant;
}

function swapDdlColumnsV1(ddl: string, pair: PairV1): string {
  const lines = ddl.split('\n');
  const start = lines.indexOf(`CREATE TABLE ${pair.schema}.${pair.relation} (`);
  if (start < 0) throw new TypeError();
  const left = start + 1 + pair.left;
  const right = start + 1 + pair.right;
  [lines[left], lines[right]] = [lines[right]!, lines[left]!];
  return lines.join('\n');
}

function swapCommandBindingV1(catalogue: CatalogueV1, pair: PairV1): CatalogueV1 {
  const mutant = structuredClone(catalogue);
  const command = mutant.insertCommands[pair.commandIndex]!;
  const shape = commandShapeV1(command);
  [shape.columns[pair.left], shape.columns[pair.right]] =
    [shape.columns[pair.right]!, shape.columns[pair.left]!];
  const originalColumnBlock = command.text.match(/^INSERT INTO[^\n]+\n([\s\S]*?)\n\) VALUES/m)?.[1];
  if (!originalColumnBlock) throw new TypeError();
  command.text = command.text.replace(
    originalColumnBlock, shape.columns.map((column) => `  ${column}`).join(',\n'),
  );
  const left = command.parameters[pair.left]!;
  const right = command.parameters[pair.right]!;
  command.parameters[pair.left] = { ...right, position: pair.left + 1 };
  command.parameters[pair.right] = { ...left, position: pair.right + 1 };
  return mutant;
}

function pairLabelV1(kind: string, pair: PairV1): string {
  return `${kind}:${pair.commandIndex}:${pair.schema}.${pair.relation}:${pair.left + 1}<->${pair.right + 1}`;
}
