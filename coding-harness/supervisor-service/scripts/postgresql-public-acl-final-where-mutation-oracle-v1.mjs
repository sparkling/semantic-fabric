// SPDX-License-Identifier: MIT

import { TextDecoder } from 'node:util';
import { FINAL_WHERE_MUTATION_SPECS_V1 }
  from './postgresql-public-acl-projection-final-where-mutations-v1.mjs';
import {
  canonicalFixture, compareRecordBags, deriveOracleRecords, parseProjectionRecords, sha256,
} from './postgresql-public-acl-oracle-v1.mjs';
import { assert, hex, parseOracleSession } from './postgresql-public-acl-oracle-wire-v1.mjs';

const MAX_TRANSCRIPT_BYTES = 16 * 1024 * 1024;
const MAX_LOOSE_RECORDS = 65_536;
const RAW_END = '@@ADR0047-RAW-V1/CONTROL/END@@\n';
const PROJECTION_BEGIN = '@@ADR0047-PROJECTION/BEGIN@@\n';
const PROJECTION_END = '@@ADR0047-PROJECTION/END@@\n';
const RECORD_KEYS = Object.freeze([
  'objectClass', 'schemaName', 'objectName', 'subobjectName', 'objectKind',
  'routineIdentityArguments', 'privilege', 'grantable',
]);
export const FINAL_WHERE_SEED_WITNESS_KEYS_V1 = Object.freeze([
  'schemaPrivate', 'schemaExcluded', 'relationPrivate', 'relationExcluded',
  'columnSystem', 'columnDropped', 'columnPrivate', 'columnExcluded',
  'routinePrivate', 'routineExcluded', 'typePrivate', 'typeExcluded',
  'languagePrivate', 'fdwPrivate', 'serverPrivate',
]);
export const FINAL_WHERE_GUARD_WITNESS_KEYS_V1 = Object.freeze([
  'relationUnsupported', 'columnUnsupportedParent', 'routineUnsupported', 'typeUnsupported',
]);

export const FINAL_WHERE_MUTATION_BATCHES_V1 = Object.freeze([
  Object.freeze({ sequence: 1, mutantIds: Object.freeze(FINAL_WHERE_MUTATION_SPECS_V1
    .slice(0, 10).map(({ id }) => id)) }),
  Object.freeze({ sequence: 2, mutantIds: Object.freeze(FINAL_WHERE_MUTATION_SPECS_V1
    .slice(10).map(({ id }) => id)) }),
]);

export function parseFinalWhereLooseRecordsV1(text) {
  assert(typeof text === 'string' && Buffer.byteLength(text, 'utf8') <= MAX_TRANSCRIPT_BYTES
    && !text.includes('\r') && !text.includes('\0') && !text.startsWith('\uFEFF')
    && (text === '' || text.endsWith('\n')),
  'ACL_FINAL_WHERE_LOOSE_FRAMING_INVALID');
  const lines = text === '' ? [] : text.slice(0, -1).split('\n');
  assert(lines.length <= MAX_LOOSE_RECORDS && lines.every((line) => line.length > 0),
    'ACL_FINAL_WHERE_LOOSE_RECORD_COUNT_INVALID');
  return lines.map((line) => {
    let value;
    try { value = JSON.parse(line); }
    catch { throw new Error('ACL_FINAL_WHERE_LOOSE_JSON_INVALID'); }
    assert(value !== null && typeof value === 'object' && !Array.isArray(value)
      && Object.getPrototypeOf(value) === Object.prototype
      && JSON.stringify(Object.keys(value)) === JSON.stringify(RECORD_KEYS),
    'ACL_FINAL_WHERE_LOOSE_RECORD_INVALID');
    assert(nonempty(value.objectClass) && nonempty(value.objectName) && nonempty(value.privilege)
      && nullableString(value.schemaName) && nullableString(value.subobjectName)
      && nullableString(value.objectKind) && nullableString(value.routineIdentityArguments)
      && typeof value.grantable === 'boolean',
    'ACL_FINAL_WHERE_LOOSE_RECORD_INVALID');
    return value;
  });
}

export function classifyFinalWhereMutationV1(spec, expected, actual, guardWitness) {
  const frozen = FINAL_WHERE_MUTATION_SPECS_V1[spec?.sequence - 1];
  assert(frozen === spec && Array.isArray(expected) && Array.isArray(actual),
    'ACL_FINAL_WHERE_CLASSIFIER_ARGUMENTS_INVALID');
  const equivalent = sameBag(expected, actual);
  if (spec.expectedOutcome === 'kill') {
    assert(!equivalent, 'ACL_FINAL_WHERE_KILLABLE_SURVIVED');
    return Object.freeze({ outcome: 'killed', guardWitness: null, guardCount: null,
      recordCount: actual.length, recordsSha256: sha256(canonicalFixture(actual)) });
  }
  assert(spec.expectedOutcome === 'guard-equivalent',
    'ACL_FINAL_WHERE_CLASSIFIER_ARGUMENTS_INVALID');
  assert(equivalent, 'ACL_FINAL_WHERE_EQUIVALENT_CHANGED');
  const guardCount = guardWitness?.[spec.guardWitness];
  assert(Number.isSafeInteger(guardCount) && guardCount === 0,
    'ACL_FINAL_WHERE_GUARD_PROOF_INVALID');
  return Object.freeze({ outcome: 'guard-equivalent', guardWitness: spec.guardWitness,
    guardCount, recordCount: actual.length, recordsSha256: sha256(canonicalFixture(actual)) });
}

export function splitFinalWhereMutationTranscriptV1(source, mutantIds) {
  assert(Buffer.isBuffer(source) && source.byteLength > 0
    && source.byteLength <= MAX_TRANSCRIPT_BYTES && validIds(mutantIds),
  'ACL_FINAL_WHERE_TRANSCRIPT_FRAMING_INVALID');
  let text;
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(source); }
  catch { throw new Error('ACL_FINAL_WHERE_TRANSCRIPT_UTF8_INVALID'); }
  assert(text.endsWith('\n') && !text.includes('\r') && !text.includes('\0')
    && !text.startsWith('\uFEFF'), 'ACL_FINAL_WHERE_TRANSCRIPT_FRAMING_INVALID');
  const rawEnd = text.indexOf(RAW_END);
  assert(rawEnd >= 0 && text.indexOf(RAW_END, rawEnd + 1) === -1,
    'ACL_FINAL_WHERE_RAW_BOUNDARY_INVALID');
  const raw = text.slice(0, rawEnd + RAW_END.length);
  const sections = Object.create(null);
  let cursor = raw.length;
  for (const id of ['seed-witness', 'guard-witness', 'original', ...mutantIds]) {
    const begin = marker(id, 'BEGIN');
    const end = marker(id, 'END');
    assert(text.startsWith(begin, cursor), 'ACL_FINAL_WHERE_SECTION_BEGIN_INVALID');
    cursor += begin.length;
    const endOffset = text.indexOf(end, cursor);
    assert(endOffset >= cursor, 'ACL_FINAL_WHERE_SECTION_END_INVALID');
    const value = text.slice(cursor, endOffset);
    assert(!value.includes('@@ADR0047-FINAL-WHERE-V1/'),
      'ACL_FINAL_WHERE_SECTION_NESTING_INVALID');
    sections[id] = value;
    cursor = endOffset + end.length;
  }
  assert(cursor === text.length, 'ACL_FINAL_WHERE_TRANSCRIPT_TRAILING_INVALID');
  return Object.freeze({ raw, seedWitness: sections['seed-witness'],
    guardWitness: sections['guard-witness'], original: sections.original,
    mutations: Object.freeze(Object.fromEntries(mutantIds.map((id) => [id, sections[id]]))) });
}

export function analyzeFinalWhereBatchTranscriptV1(source, batch, catalogue) {
  const expectedBatch = FINAL_WHERE_MUTATION_BATCHES_V1[batch?.sequence - 1];
  assert(expectedBatch === batch && catalogue?.mutants?.length === 19,
    'ACL_FINAL_WHERE_ANALYSIS_ARGUMENTS_INVALID');
  const parsed = splitFinalWhereMutationTranscriptV1(source, batch.mutantIds);
  const oracleSession = parseOracleSession(Buffer.from(
    `${parsed.raw}${PROJECTION_BEGIN}${parsed.original}${PROJECTION_END}`, 'utf8'));
  const expected = deriveOracleRecords(oracleSession.raw, { enforceCleanProfile: false });
  validateVisibleSeed(oracleSession.raw, expected);
  const seedWitness = parseWitness(parsed.seedWitness, FINAL_WHERE_SEED_WITNESS_KEYS_V1,
    'ACL_FINAL_WHERE_SEED_WITNESS_INVALID', true);
  const guardWitness = parseWitness(parsed.guardWitness, FINAL_WHERE_GUARD_WITNESS_KEYS_V1,
    'ACL_FINAL_WHERE_GUARD_WITNESS_INVALID', false);
  const original = parseProjectionRecords(projectionLines(parsed.original));
  assert(sameBag(expected, original) && canonicalFixture(expected).equals(canonicalFixture(original)),
    'ACL_FINAL_WHERE_ORIGINAL_CONTROL_INVALID');
  const byId = new Map(catalogue.mutants.map((mutant) => [mutant.id, mutant]));
  const mutations = batch.mutantIds.map((id) => {
    const mutant = byId.get(id);
    assert(mutant !== undefined, 'ACL_FINAL_WHERE_CATALOGUE_ID_INVALID');
    const actual = parseFinalWhereLooseRecordsV1(parsed.mutations[id]);
    const result = classifyFinalWhereMutationV1(
      FINAL_WHERE_MUTATION_SPECS_V1[mutant.sequence - 1], expected, actual, guardWitness);
    return Object.freeze({ sequence: mutant.sequence, id, expectedOutcome: mutant.expectedOutcome,
      outcome: result.outcome, executed: true, parsed: true,
      guardWitness: result.guardWitness, guardCount: result.guardCount,
      recordCount: result.recordCount, recordsSha256: result.recordsSha256,
      sourceSha256: mutant.sourceSha256 });
  });
  assert(Object.values(seedWitness).every((value) => value > 0),
    'ACL_FINAL_WHERE_SEED_WITNESS_INVALID');
  return Object.freeze({ sequence: batch.sequence, transcriptBytes: source.byteLength,
    transcriptSha256: sha256(source), originalEquivalent: true, seedWitnessComplete: true,
    mutations: Object.freeze(mutations) });
}

export function combineFinalWhereMutationBatchReceiptsV1(batches) {
  const value = {
    schemaVersion: 1, authority: 'test-only-non-runtime', executed: 19, killed: 15,
    guardEquivalent: 4, unresolved: 0, batches: Object.freeze([...batches]),
  };
  validateFinalWhereMutationReceiptV1(value);
  return Object.freeze(value);
}

export function validateFinalWhereMutationReceiptV1(value) {
  exactObject(value, ['schemaVersion', 'authority', 'executed', 'killed', 'guardEquivalent',
    'unresolved', 'batches'], 'ACL_FINAL_WHERE_RECEIPT_INVALID');
  assert(value.schemaVersion === 1 && value.authority === 'test-only-non-runtime'
    && value.executed === 19 && value.killed === 15 && value.guardEquivalent === 4
    && value.unresolved === 0 && Array.isArray(value.batches) && value.batches.length === 2,
  'ACL_FINAL_WHERE_RECEIPT_INVALID');
  const observed = [];
  value.batches.forEach((batch, index) => {
    exactObject(batch, ['sequence', 'transcriptBytes', 'transcriptSha256',
      'originalEquivalent', 'seedWitnessComplete', 'mutations'],
    'ACL_FINAL_WHERE_RECEIPT_BATCH_INVALID');
    const expectedBatch = FINAL_WHERE_MUTATION_BATCHES_V1[index];
    assert(batch.sequence === index + 1 && positive(batch.transcriptBytes)
      && digest(batch.transcriptSha256) && batch.originalEquivalent === true
      && batch.seedWitnessComplete === true && Array.isArray(batch.mutations)
      && batch.mutations.length === expectedBatch.mutantIds.length,
    'ACL_FINAL_WHERE_RECEIPT_BATCH_INVALID');
    batch.mutations.forEach((mutation) => observed.push(mutation));
  });
  assert(observed.length === FINAL_WHERE_MUTATION_SPECS_V1.length,
    'ACL_FINAL_WHERE_RECEIPT_MUTATION_INVALID');
  observed.forEach((mutation, index) => validateReceiptMutation(
    mutation, FINAL_WHERE_MUTATION_SPECS_V1[index]));
  return value;
}

function validateReceiptMutation(value, spec) {
  exactObject(value, ['sequence', 'id', 'expectedOutcome', 'outcome', 'executed', 'parsed',
    'guardWitness', 'guardCount', 'recordCount', 'recordsSha256', 'sourceSha256'],
  'ACL_FINAL_WHERE_RECEIPT_MUTATION_INVALID');
  const equivalent = spec.expectedOutcome === 'guard-equivalent';
  assert(value.sequence === spec.sequence && value.id === spec.id
    && value.expectedOutcome === spec.expectedOutcome
    && value.outcome === (equivalent ? 'guard-equivalent' : 'killed')
    && value.executed === true && value.parsed === true
    && value.guardWitness === spec.guardWitness
    && value.guardCount === (equivalent ? 0 : null)
    && positive(value.recordCount) && digest(value.recordsSha256)
    && value.sourceSha256 === spec.sourceSha256,
  'ACL_FINAL_WHERE_RECEIPT_MUTATION_INVALID');
}

function parseWitness(text, keys, code, positiveOnly) {
  assert(typeof text === 'string' && text.endsWith('\n') && !text.slice(0, -1).includes('\n'), code);
  let value;
  try { value = JSON.parse(text); } catch { throw new Error(code); }
  exactObject(value, keys, code);
  assert(Object.values(value).every((entry) => Number.isSafeInteger(entry)
    && (positiveOnly ? entry > 0 : entry >= 0)), code);
  return value;
}

function validateVisibleSeed(raw, records) {
  const fdw = raw.FDW.filter((row) => hex(row[1]) === 'sf_public_acl_mutation_fdw_v1'
    && row[8] === '0');
  const server = raw.SERVER.filter((row) => hex(row[1]) === 'sf_public_acl_mutation_server_v1'
    && row[9] === '0');
  const seeded = records.filter(({ objectName }) => [
    'sf_public_acl_mutation_fdw_v1', 'sf_public_acl_mutation_server_v1',
  ].includes(objectName));
  assert(fdw.length === 1 && server.length === 1 && seeded.length === 2
    && seeded.every(({ privilege, grantable }) => privilege === 'USAGE' && grantable === false),
  'ACL_FINAL_WHERE_VISIBLE_SEED_INVALID');
}

function projectionLines(text) {
  assert(text.length > 1 && text.endsWith('\n'), 'ACL_FINAL_WHERE_PROJECTION_FRAMING_INVALID');
  return text.slice(0, -1).split('\n');
}

function sameBag(left, right) {
  try { compareRecordBags(left, right); return true; } catch { return false; }
}

function marker(id, edge) {
  assert(/^[a-z0-9-]+$/.test(id) && (edge === 'BEGIN' || edge === 'END'),
    'ACL_FINAL_WHERE_SECTION_ID_INVALID');
  return `@@ADR0047-FINAL-WHERE-V1/${id}/${edge}@@\n`;
}

function validIds(value) {
  return Array.isArray(value) && value.length > 0 && value.length <= 10
    && new Set(value).size === value.length
    && value.every((id) => typeof id === 'string' && /^[a-z0-9-]+$/.test(id));
}

function exactObject(value, keys, code) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype
    && JSON.stringify(Object.keys(value)) === JSON.stringify(keys), code);
}

function nullableString(value) { return value === null || typeof value === 'string'; }
function nonempty(value) { return typeof value === 'string' && value.length > 0; }
function positive(value) { return Number.isSafeInteger(value) && value > 0; }
function digest(value) { return typeof value === 'string' && /^[0-9a-f]{64}$/.test(value); }
