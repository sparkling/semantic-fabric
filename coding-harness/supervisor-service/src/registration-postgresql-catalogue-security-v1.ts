// SPDX-License-Identifier: MIT

import type { CatalogueRecordV1 }
  from './registration-postgresql-catalogue-shape-v1.js';
import { policyExpressionV1, type PolicyArgumentsV1 }
  from './registration-postgresql-catalogue-templates-v1.js';
import {
  catalogueArrayV1,
  catalogueBooleanV1,
  catalogueRecordV1,
  catalogueStringV1,
  recordsV1,
  requireEqualJsonV1,
  requireSortedRecordsV1,
  requireV1,
  stringsV1,
} from './registration-postgresql-catalogue-values-v1.js';

const SCHEMA = 'sf_supervisor_v1';
const OWNER = 'sf_supervisor_owner_v1';
const SCOPE = 'sf_supervisor_project_scope_v1';
const LOGIN: Readonly<Record<string, string>> = Object.freeze({
  writer: 'sf_supervisor_writer_login_v1',
  recovery: 'sf_supervisor_recovery_login_v1',
  readiness: 'sf_supervisor_readiness_login_v1',
  migration_owner: OWNER,
});
const CAPABILITY: Readonly<Record<string, string>> = Object.freeze({
  writer: 'sf_supervisor_writer_capability_v1',
  recovery: 'sf_supervisor_recovery_capability_v1',
  readiness: 'sf_supervisor_readiness_capability_v1',
});

export function validatePostgresCatalogueSecurityV1(root: CatalogueRecordV1): void {
  const relations = new Set(recordsV1(root, 'relations').map((value) => text(value, 'name')));
  const domains = new Set(recordsV1(root, 'domains').map((value) => text(value, 'name')));
  const columns = new Set(recordsV1(root, 'columns').map((value) =>
    `${text(value, 'relation')}.${text(value, 'name')}`));
  validateSchemaAcl(root);
  validatePolicies(root, relations);
  validateObjectAcls(root, relations, domains);
  validateColumnAcls(root, columns);
  validateDefaultAcls(root);
  validateImplicitObjects(root);
}

function validateSchemaAcl(root: CatalogueRecordV1): void {
  const schemas = recordsV1(root, 'schemas');
  requireV1(schemas.length === 1, 'schema ACL');
  const schema = schemas[0]!;
  requireV1(text(schema, 'name') === SCHEMA && text(schema, 'owner') === OWNER
    && text(schema, 'aclState') === 'explicit', 'schema ACL');
  const expected = [
    atom(OWNER, 'CREATE'), atom(OWNER, 'USAGE'),
    atom(LOGIN.readiness!, 'USAGE'), atom(LOGIN.recovery!, 'USAGE'),
    atom(LOGIN.writer!, 'USAGE'),
  ];
  requireEqualJsonV1(schema.privileges!, expected, 'schema privileges');
}

function validatePolicies(root: CatalogueRecordV1, relations: Set<string>): void {
  const policies = recordsV1(root, 'policies');
  requireV1(policies.length === 38, 'policies');
  requireSortedRecordsV1(policies, (value) => [
    text(value, 'schema'), text(value, 'relation'), text(value, 'name'),
  ], 'policies');
  const pairs = new Map<string, Set<string>>();
  for (const value of policies) {
    const relation = text(value, 'relation');
    const command = text(value, 'command');
    const match = /^(writer|recovery|readiness|migration_owner)_(select|insert|update)_(permit|scope)_v1$/
      .exec(text(value, 'name'));
    requireV1(match !== null && text(value, 'schema') === SCHEMA
      && relation !== 'schema_migrations' && relations.has(relation), 'policy identity');
    const principal = match![1]!;
    const namedCommand = match![2]!;
    const lane = match![3]!;
    requireV1(command === namedCommand
      && (principal === 'writer' || command === 'select'), 'policy admission');
    const roles = arrayRecords(value, 'roles');
    requireV1(roles.length === 1 && text(roles[0]!, 'kind') === 'role'
      && nullableText(roles[0]!, 'name') === LOGIN[principal], 'policy role');
    const permissive = lane === 'permit';
    requireV1(boolean(value, 'permissive') === permissive, 'policy mode');
    const template = permissive ? 'scope-equality-v1'
      : principal === 'migration_owner' ? 'migration-session-owner-v1'
        : 'scope-capability-v1';
    const args: PolicyArgumentsV1 = {
      scopeRole: SCOPE,
      capabilityRole: permissive || principal === 'migration_owner'
        ? null : CAPABILITY[principal]!,
      sessionLogin: !permissive && principal === 'migration_owner'
        ? 'sf_supervisor_migration_login_v1' : null,
      ownerRole: !permissive && principal === 'migration_owner' ? OWNER : null,
    };
    if (command === 'select' || command === 'update') {
      validatePolicyClause(value, 'using', template, args);
    } else validateNullClause(value, 'using');
    if (command === 'insert' || command === 'update') {
      validatePolicyClause(value, 'withCheck', template, args);
    } else validateNullClause(value, 'withCheck');
    const key = `${relation}.${command}.${principal}`;
    const lanes = pairs.get(key) ?? new Set<string>();
    requireV1(!lanes.has(lane), 'policy pair');
    lanes.add(lane);
    pairs.set(key, lanes);
  }
  requireV1(pairs.size === 19 && [...pairs.values()].every((value) =>
    value.size === 2 && value.has('permit') && value.has('scope')), 'policy closure');
}

function validatePolicyClause(
  value: CatalogueRecordV1,
  prefix: 'using' | 'withCheck',
  template: string,
  expected: PolicyArgumentsV1,
): void {
  const templateKey = `${prefix}Template`;
  const argumentsKey = `${prefix}Arguments`;
  const expressionKey = `${prefix}Expression`;
  requireV1(nullableText(value, templateKey) === template, 'policy template');
  const args = record(value, argumentsKey);
  const actual: PolicyArgumentsV1 = {
    scopeRole: nullableText(args, 'scopeRole'),
    capabilityRole: nullableText(args, 'capabilityRole'),
    sessionLogin: nullableText(args, 'sessionLogin'),
    ownerRole: nullableText(args, 'ownerRole'),
  };
  requireV1(JSON.stringify(actual) === JSON.stringify(expected)
    && nullableText(value, expressionKey) === policyExpressionV1(template, actual), 'policy expression');
}

function validateNullClause(value: CatalogueRecordV1, prefix: 'using' | 'withCheck'): void {
  requireV1(value[`${prefix}Template`] === null && value[`${prefix}Arguments`] === null
    && value[`${prefix}Expression`] === null, 'policy null clause');
}

function validateObjectAcls(
  root: CatalogueRecordV1,
  relations: Set<string>,
  domains: Set<string>,
): void {
  const values = recordsV1(root, 'objectAcls');
  requireV1(values.length === 44, 'object ACLs');
  requireSortedRecordsV1(values, (value) => [
    text(value, 'objectKind'), nullableText(value, 'schema'), text(value, 'object'),
  ], 'object ACLs');
  const expectedTypes = new Set([
    ...[...relations].map((name) => `_${name}`),
    ...[...domains].map((name) => `_${name}`),
    ...relations,
  ]);
  const kindCounts = new Map<string, number>();
  for (const value of values) {
    const kind = text(value, 'objectKind');
    const name = text(value, 'object');
    requireV1(nullableText(value, 'schema') === SCHEMA && text(value, 'owner') === OWNER,
      'object ACL owner');
    kindCounts.set(kind, (kindCounts.get(kind) ?? 0) + 1);
    if (kind === 'domain') requireV1(domains.has(name), 'domain ACL');
    else if (kind === 'table') requireV1(relations.has(name), 'table ACL');
    else requireV1(expectedTypes.has(name), 'type ACL');
    const arrayType = kind === 'type' && name.startsWith('_');
    requireV1(text(value, 'aclState') === (arrayType ? 'null' : 'explicit'), 'object ACL state');
    const privileges = arrayRecords(value, 'privileges');
    validateAtoms(privileges);
    const expected = kind === 'table'
      ? ['DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE']
      : ['USAGE'];
    requireV1(JSON.stringify(privileges.map((item) => text(item, 'privilege')))
      === JSON.stringify(expected)
      && privileges.every((item) => nullableText(item, 'granteeRole') === OWNER), 'object ACL');
  }
  requireV1(JSON.stringify(Object.fromEntries(kindCounts))
    === JSON.stringify({ domain: 10, table: 8, type: 26 }), 'object ACL kinds');
  requireV1(values.flatMap((value) => arrayRecords(value, 'privileges')).length === 92,
    'object ACL atoms');
}

function validateColumnAcls(root: CatalogueRecordV1, columns: Set<string>): void {
  const values = recordsV1(root, 'columnAcls');
  requireV1(values.length === 88, 'column ACLs');
  requireSortedRecordsV1(values, (value) => [
    text(value, 'schema'), text(value, 'relation'), text(value, 'column'),
  ], 'column ACLs');
  for (const value of values) {
    requireV1(text(value, 'schema') === SCHEMA
      && columns.has(`${text(value, 'relation')}.${text(value, 'column')}`)
      && text(value, 'aclState') === 'explicit', 'column ACL');
    const privileges = arrayRecords(value, 'privileges');
    validateAtoms(privileges);
    requireV1(privileges.every((item) => ['SELECT', 'INSERT', 'UPDATE'].includes(
      text(item, 'privilege'),
    )), 'column privilege');
  }
  requireV1(values.flatMap((value) => arrayRecords(value, 'privileges')).length === 178,
    'column ACL atoms');
}

function validateDefaultAcls(root: CatalogueRecordV1): void {
  const values = recordsV1(root, 'defaultAcls');
  requireV1(values.length === 5, 'default ACLs');
  requireSortedRecordsV1(values, (value) => [
    text(value, 'owner'), nullableText(value, 'schema'), text(value, 'objectClass'),
  ], 'default ACLs');
  const states: Readonly<Record<string, string>> = Object.freeze({
    function: 'explicit', schema: 'absent', sequence: 'absent', table: 'absent', type: 'explicit',
  });
  for (const value of values) {
    requireV1(text(value, 'owner') === OWNER && value.schema === null
      && text(value, 'rowState') === states[text(value, 'objectClass')], 'default ACL');
    validateAtoms(arrayRecords(value, 'privileges'));
  }
  requireV1(values.flatMap((value) => arrayRecords(value, 'privileges')).length === 14,
    'default ACL atoms');
}

function validateImplicitObjects(root: CatalogueRecordV1): void {
  const value = record(root, 'implicitObjects');
  requireV1(JSON.stringify(stringsV1(value.allowedDerivedKinds!, 'derived kinds'))
    === JSON.stringify(['array-type', 'composite-row-type', 'constraint-index',
      'foreign-key-trigger', 'toast-index', 'toast-relation']), 'derived kinds');
  requireV1(JSON.stringify(stringsV1(value.forbiddenOwnedKinds!, 'forbidden kinds'))
    === JSON.stringify(['aggregate', 'cast', 'collation', 'conversion', 'extension',
      'foreign-table', 'function', 'materialized-view', 'operator', 'partition', 'procedure',
      'publication', 'rule', 'sequence', 'subscription', 'text-search-object', 'user-trigger',
      'view']), 'forbidden kinds');
  requireEqualJsonV1(value.rules!, {
    arrayTypes: 'raw-null-effective-element-acl-v1',
    compositeRowTypes: 'explicit-owner-only-acl-v1',
    constraintIndexes: 'enumerated-constraint-index-v1',
    foreignKeyTriggers: 'four-internal-ri-triggers-v1',
    toastObjects: 'parent-linked-unnamed-v1',
  }, 'implicit rules');
}

function validateAtoms(values: CatalogueRecordV1[]): void {
  requireSortedRecordsV1(values, (value) => [
    text(value, 'grantorRole'), text(value, 'granteeKind'), nullableText(value, 'granteeRole'),
    text(value, 'privilege'), boolean(value, 'grantable'),
  ], 'privileges');
  for (const value of values) {
    const kind = text(value, 'granteeKind');
    const role = nullableText(value, 'granteeRole');
    requireV1(text(value, 'grantorRole') === OWNER && !boolean(value, 'grantable')
      && ((kind === 'public' && role === null) || (kind === 'role' && role !== null))
      && (role === null || !role.includes('_capability_')), 'privilege atom');
  }
}

function atom(role: string, privilege: string): CatalogueRecordV1 {
  return { grantorRole: OWNER, granteeKind: 'role', granteeRole: role, privilege, grantable: false };
}
function text(value: CatalogueRecordV1, key: string): string {
  return catalogueStringV1(value[key]!, key);
}
function boolean(value: CatalogueRecordV1, key: string): boolean {
  return catalogueBooleanV1(value[key]!, key);
}
function nullableText(value: CatalogueRecordV1, key: string): string | null {
  return value[key] === null ? null : text(value, key);
}
function record(value: CatalogueRecordV1, key: string): CatalogueRecordV1 {
  return catalogueRecordV1(value[key]!, key);
}
function arrayRecords(value: CatalogueRecordV1, key: string): CatalogueRecordV1[] {
  return catalogueArrayV1(value[key]!, key).map((item) => catalogueRecordV1(item, key));
}
