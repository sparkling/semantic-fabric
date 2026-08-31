// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';

export const FINAL_WHERE_MUTATION_SOURCE_PIN_V1 = Object.freeze({
  bytes: 6_859,
  sha256: '0e3ad724f4ce85191564c245c51dd7665b6d9aa704c355067a0056cdbfe95232',
});

const RAW_SPECS = [
  ['delete-schema-where-public-grantee', 'schema', 'kill', null, 829, 847,
    'a.grantee = 0 AND ', 'edc3367c0e8a3ac92c2031b3a5745ab987441ccb1e8ed4aa60917da93b7b5509'],
  ['delete-schema-where-excluded-namespace', 'schema', 'kill', null, 842, 894,
    " AND n.nspname NOT IN ('public', 'sf_supervisor_v1')",
    'a1699a676732fbacd863c0f4c65e6326e06af4e0e616fb82a61a3ec39b417818'],
  ['delete-relation-where-supported-relkind', 'relation', 'guard-equivalent',
    'relationUnsupported', 1624, 1672,
    "c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S') AND ",
    '867c92776cafd21ef213acfa80c4b994ce28a6b4dd822eca0d72ca0b55753f35'],
  ['delete-relation-where-public-grantee', 'relation', 'kill', null, 1667, 1685,
    ' AND a.grantee = 0', '1c13c5791460fdbf7c53c16887cc9cc2661a625e5f254d204d0f851ee5aa39e2'],
  ['delete-relation-where-excluded-namespace', 'relation', 'kill', null, 1686, 1742,
    "    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')\n",
    'e379e71814be89433b4594243f1dddb2fdc7267e4f76c51c2c9c970702cb7841'],
  ['delete-column-where-positive-attnum', 'column', 'kill', null, 2456, 2475,
    'att.attnum > 0 AND ', '3333b8672b3d9124aa6ca6558c5c1b89ab5b821332af5c6c8e2ba04d5298540c'],
  ['delete-column-where-not-dropped', 'column', 'kill', null, 2470, 2495,
    ' AND NOT att.attisdropped', 'a767b46920bdb68ef503f9b262700ad0ba63f805960a381ed8c6e35b1460d273'],
  ['delete-column-where-supported-parent-relkind', 'column', 'guard-equivalent',
    'columnUnsupportedParent', 2503, 2546,
    " c.relkind IN ('r', 'p', 'v', 'm', 'f') AND",
    '94b0d8d2c91cc6a63dfc47a401e6a1c4e0a421024091ad906548a8b138580aab'],
  ['delete-column-where-public-grantee', 'column', 'kill', null, 2542, 2560,
    ' AND a.grantee = 0', '479a0248e4428380d4d7a70d56f2b29c3e52150e526cc24bff4f128077f9cb48'],
  ['delete-column-where-excluded-namespace', 'column', 'kill', null, 2561, 2617,
    "    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')\n",
    '8e6e6a25c5f4ad3511e3c8050367a659b53d226de765ec8ea9f4797281ad1f2d'],
  ['delete-routine-where-supported-prokind', 'routine', 'guard-equivalent',
    'routineUnsupported', 3270, 3308, "p.prokind IN ('f', 'p', 'a', 'w') AND ",
    '91291cdc20788acf6cc3ce68a79c33efb7bb8073421d4ccd3f5d4cce4aeced58'],
  ['delete-routine-where-public-grantee', 'routine', 'kill', null, 3303, 3321,
    ' AND a.grantee = 0', '3fe642de2c741ceb0fc388034fa4d7b1b6c91c0f2a1f3993aa3aa4ad2ab2ae52'],
  ['delete-routine-where-excluded-namespace', 'routine', 'kill', null, 3322, 3378,
    "    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')\n",
    '4c4e72fa3d00c0a6dc92cc0f8c8becb21985cd54490e58b3e1aecae26fef55b1'],
  ['delete-type-where-supported-typtype', 'type', 'guard-equivalent',
    'typeUnsupported', 4739, 4792,
    "t.typtype IN ('b', 'c', 'd', 'e', 'p', 'r', 'm') AND ",
    'c01051e8b0624536acbd9a0eec675757497ed74a253561a298609c1a33420e3a'],
  ['delete-type-where-public-grantee', 'type', 'kill', null, 4787, 4805,
    ' AND a.grantee = 0', '6d7fdde6c69b3651e38a54ddae9bbd2f6f63d1be0d310674e4d89ccbad89620f'],
  ['delete-type-where-excluded-namespace', 'type', 'kill', null, 4806, 4862,
    "    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')\n",
    '1b53568593fac521266a748a0740fc7812ba16c73871bbd5661ae87fb3c01c5c'],
  ['delete-language-where-public-grantee', 'language', 'kill', null, 5259, 5279,
    ' WHERE a.grantee = 0', '834039dcac2fb85b5027f34f324e4dcc8cc916fa17c1073b1b6a11bece20261b'],
  ['delete-foreign-data-wrapper-where-public-grantee', 'foreign-data-wrapper', 'kill', null,
    5713, 5733, ' WHERE a.grantee = 0',
    'c0d1dcb6fc033d2663e0b39abf62f45d07dfe1c00985030d298f4e564960fdbc'],
  ['delete-foreign-server-where-public-grantee', 'foreign-server', 'kill', null, 6149, 6169,
    ' WHERE a.grantee = 0', '77e8b71d2cab9beeeae464ac6e6df5379a16402d9b81887cde5caf44fb222e6d'],
];

export const FINAL_WHERE_MUTATION_SPECS_V1 = Object.freeze(RAW_SPECS.map((value, index) =>
  Object.freeze({
    sequence: index + 1, id: value[0], objectClass: value[1], expectedOutcome: value[2],
    guardWitness: value[3], start: value[4], end: value[5], removed: value[6],
    sourceSha256: value[7],
  })));

export function buildPublicAclProjectionFinalWhereMutantsV1(source) {
  validateSource(source);
  const mutants = FINAL_WHERE_MUTATION_SPECS_V1.map((spec) => {
    assert(source.slice(spec.start, spec.end) === spec.removed,
      'ACL_FINAL_WHERE_ANCHOR_INVALID');
    const mutantSource = source.slice(0, spec.start) + source.slice(spec.end);
    const sourceBytes = Buffer.byteLength(mutantSource, 'utf8');
    assert(sourceBytes === FINAL_WHERE_MUTATION_SOURCE_PIN_V1.bytes
      - Buffer.byteLength(spec.removed, 'utf8')
      && sha256(mutantSource) === spec.sourceSha256,
    'ACL_FINAL_WHERE_MUTANT_PIN_INVALID');
    return Object.freeze({ ...spec, source: mutantSource, sourceBytes });
  });
  assert(mutants.length === 19 && new Set(mutants.map(({ id }) => id)).size === 19
    && mutants.filter(({ expectedOutcome }) => expectedOutcome === 'kill').length === 15
    && mutants.filter(({ expectedOutcome }) => expectedOutcome === 'guard-equivalent').length === 4,
  'ACL_FINAL_WHERE_CATALOGUE_INVALID');
  return Object.freeze({
    schemaVersion: 1, authority: 'test-only-non-runtime',
    sourcePin: FINAL_WHERE_MUTATION_SOURCE_PIN_V1, mutants: Object.freeze(mutants),
  });
}

function validateSource(source) {
  assert(typeof source === 'string' && Buffer.byteLength(source, 'utf8')
    === FINAL_WHERE_MUTATION_SOURCE_PIN_V1.bytes
    && sha256(source) === FINAL_WHERE_MUTATION_SOURCE_PIN_V1.sha256
    && source.endsWith('\n') && !source.includes('\r') && !source.includes('\0'),
  'ACL_FINAL_WHERE_SOURCE_PIN_INVALID');
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

function assert(condition, code) {
  if (!condition) throw new Error(code);
}
