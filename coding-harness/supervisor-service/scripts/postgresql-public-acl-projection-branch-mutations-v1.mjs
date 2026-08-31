// SPDX-License-Identifier: MIT

export const PUBLIC_ACL_OBJECT_CLASSES_V1 = Object.freeze([
  'schema', 'relation', 'column', 'routine', 'type', 'language',
  'foreign-data-wrapper', 'foreign-server',
]);

const RAW_RECORD_COLUMNS = Object.freeze([
  'object_class', 'schema_name', 'object_name', 'subobject_name', 'object_kind',
  'routine_identity_arguments', 'privilege', 'grantable',
]);
const EXPLICIT_COLUMNS = ` (\n${RAW_RECORD_COLUMNS
  .map((column) => `  ${column}`).join(',\n')}\n) `;
const MAX_SOURCE_BYTES = 64 * 1024;
const RECORD_SET_SENTINEL_V1 = Object.freeze({
  objectClass: 'type',
  schemaName: 'zzzz_sf_public_acl_mutation_v1',
  objectName: 'zzzz_added_atom_v1',
  subobjectName: null,
  objectKind: 'base',
  routineIdentityArguments: null,
  privilege: 'USAGE',
  grantable: false,
});
const SENTINEL_BRANCH = `  UNION ALL
  SELECT 'type'::text, 'zzzz_sf_public_acl_mutation_v1'::text,
    'zzzz_added_atom_v1'::text, NULL::text, 'base'::text,
    NULL::text, 'USAGE'::text, false
`;

export function withExplicitRawRecordColumnsV1(source) {
  const parsed = parseProjection(source);
  if (parsed.columns !== null) return source;
  return source.slice(0, parsed.rawRecordsEnd) + EXPLICIT_COLUMNS
    + source.slice(parsed.asStart);
}

export function splitTopLevelRawRecordBranchesV1(source) {
  const parsed = parseProjection(source);
  const branches = splitBranches(source, parsed);
  validateClassSubsequence(branches.map((branch) => branch.objectClass));
  return Object.freeze(branches.map(({ objectClass, source: branchSource }) => Object.freeze({
    objectClass, source: branchSource,
  })));
}

export function buildPublicAclProjectionBranchDeletionMutantsV1(source) {
  const normalizedSource = withExplicitRawRecordColumnsV1(source);
  const parsed = parseProjection(normalizedSource);
  const branches = splitBranches(normalizedSource, parsed);
  const classes = branches.map((branch) => branch.objectClass);
  assert(branches.length === PUBLIC_ACL_OBJECT_CLASSES_V1.length,
    'ACL_MUTATION_BRANCH_COUNT_INVALID');
  assert(sameArray(classes, PUBLIC_ACL_OBJECT_CLASSES_V1),
    'ACL_MUTATION_BRANCH_CLASSES_INVALID');
  const mutants = branches.map((branch, index) => {
    const mutantSource = deleteBranch(normalizedSource, parsed, branches, index);
    const surviving = splitTopLevelRawRecordBranchesV1(mutantSource)
      .map((value) => value.objectClass);
    assert(sameArray(surviving, classes.filter((_, candidate) => candidate !== index)),
      'ACL_MUTATION_DELETE_INVALID');
    return Object.freeze({ objectClass: branch.objectClass, source: mutantSource });
  });
  assert(new Set(mutants.map((mutant) => mutant.source)).size === mutants.length,
    'ACL_MUTATION_DELETE_INVALID');
  return Object.freeze({
    schemaVersion: 1,
    authority: 'test-only-non-runtime',
    objectClasses: PUBLIC_ACL_OBJECT_CLASSES_V1,
    normalizedSource,
    mutants: Object.freeze(mutants),
  });
}

export function buildPublicAclProjectionRecordSetMutantsV1(source) {
  const normalizedSource = withExplicitRawRecordColumnsV1(source);
  const parsed = parseProjection(normalizedSource);
  terminalSemicolon(normalizedSource, parsed);
  const withSentinel = insertSentinelBranch(normalizedSource, parsed);
  const mutants = Object.freeze([
    frozenMutant('return-zero', insertFinalClause(normalizedSource, 'LIMIT 0')),
    frozenMutant('omit-first-atom', insertFinalClause(normalizedSource, 'OFFSET 1')),
    frozenMutant('add-sentinel-atom', withSentinel),
    frozenMutant('substitute-first-atom', insertFinalClause(withSentinel, 'OFFSET 1')),
  ]);
  assert(new Set(mutants.map((mutant) => mutant.source)).size === mutants.length,
    'ACL_MUTATION_RECORD_SET_SOURCES_INVALID');
  mutants.forEach((mutant) => terminalSemicolon(mutant.source, parseProjection(mutant.source)));
  return Object.freeze({
    schemaVersion: 1,
    authority: 'test-only-non-runtime',
    sentinel: RECORD_SET_SENTINEL_V1,
    normalizedSource,
    mutants,
  });
}

function frozenMutant(id, source) {
  return Object.freeze({ id, source });
}

function insertSentinelBranch(source, parsed) {
  assert(source.slice(parsed.bodyEnd - 1, parsed.bodyEnd) === '\n',
    'ACL_MUTATION_RECORD_SET_STRUCTURE_INVALID');
  return source.slice(0, parsed.bodyEnd) + SENTINEL_BRANCH + source.slice(parsed.bodyEnd);
}

function insertFinalClause(source, clause) {
  assert(clause === 'LIMIT 0' || clause === 'OFFSET 1',
    'ACL_MUTATION_RECORD_SET_CLAUSE_INVALID');
  const parsed = parseProjection(source);
  const offset = terminalSemicolon(source, parsed).start;
  return source.slice(0, offset) + `\n${clause}` + source.slice(offset);
}

function terminalSemicolon(source, parsed) {
  const semicolons = parsed.tokens.filter((token) => token.depth === 0 && punct(token, ';'));
  const terminal = semicolons[0];
  assert(semicolons.length === 1 && terminal === parsed.tokens.at(-1)
    && terminal.start > parsed.bodyEnd && source.slice(terminal.end) === '\n',
  'ACL_MUTATION_TERMINAL_STATEMENT_INVALID');
  return terminal;
}

function parseProjection(source) {
  validateSource(source);
  const tokens = scan(source);
  const headers = [];
  for (let index = 0; index + 1 < tokens.length; index += 1) {
    if (word(tokens[index], 'WITH') && tokens[index].depth === 0
      && word(tokens[index + 1], 'raw_records') && tokens[index + 1].depth === 0) {
      headers.push(index);
    }
  }
  assert(headers.length === 1 && headers[0] === 0,
    'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID');
  const rawRecords = tokens[1];
  let cursor = 2;
  let columns = null;
  if (punct(tokens[cursor], '(') && tokens[cursor].depth === 0) {
    const close = matchingClose(tokens, cursor);
    columns = parseColumns(tokens.slice(cursor + 1, close));
    assert(sameArray(columns, RAW_RECORD_COLUMNS), 'ACL_MUTATION_RAW_RECORD_COLUMNS_INVALID');
    cursor = close + 1;
  }
  assert(word(tokens[cursor], 'AS') && tokens[cursor].depth === 0,
    'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID');
  const asToken = tokens[cursor];
  cursor += 1;
  assert(punct(tokens[cursor], '(') && tokens[cursor].depth === 0,
    'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID');
  const bodyOpenIndex = cursor;
  const bodyCloseIndex = matchingClose(tokens, bodyOpenIndex);
  const bodyOpen = tokens[bodyOpenIndex];
  const bodyClose = tokens[bodyCloseIndex];
  assert(bodyOpen.end < bodyClose.start, 'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID');
  return {
    tokens, columns, rawRecordsEnd: rawRecords.end, asStart: asToken.start,
    bodyOpenIndex, bodyCloseIndex, bodyStart: bodyOpen.end, bodyEnd: bodyClose.start,
    bodyDepth: bodyOpen.depth + 1,
  };
}

function splitBranches(source, parsed) {
  const separators = [];
  const { tokens } = parsed;
  for (let index = parsed.bodyOpenIndex + 1; index < parsed.bodyCloseIndex; index += 1) {
    const token = tokens[index];
    if (token.depth !== parsed.bodyDepth || !word(token, 'UNION')) continue;
    const all = tokens[index + 1];
    assert(word(all, 'ALL') && all.depth === parsed.bodyDepth,
      'ACL_MUTATION_BRANCH_COUNT_INVALID');
    separators.push({ start: token.start, end: all.end });
    index += 1;
  }
  const spans = [];
  let start = parsed.bodyStart;
  for (const separator of separators) {
    spans.push({ start, end: separator.start });
    start = separator.end;
  }
  spans.push({ start, end: parsed.bodyEnd });
  assert(spans.length >= 1 && spans.length <= PUBLIC_ACL_OBJECT_CLASSES_V1.length,
    'ACL_MUTATION_BRANCH_COUNT_INVALID');
  const branches = spans.map((span) => {
    const branchTokens = tokens.filter((token) => token.start >= span.start
      && token.end <= span.end && token.depth === parsed.bodyDepth);
    assert(word(branchTokens[0], 'SELECT') && branchTokens[1]?.kind === 'string',
      'ACL_MUTATION_BRANCH_CLASS_LITERAL_INVALID');
    return {
      ...span,
      objectClass: branchTokens[1].value,
      source: source.slice(span.start, span.end).trim(),
    };
  });
  return Object.freeze(branches.map((branch) => Object.freeze(branch)));
}

function deleteBranch(source, parsed, branches, index) {
  assert(Number.isInteger(index) && index >= 0 && index < branches.length,
    'ACL_MUTATION_DELETE_INVALID');
  const separators = [];
  for (let tokenIndex = parsed.bodyOpenIndex + 1;
    tokenIndex < parsed.bodyCloseIndex; tokenIndex += 1) {
    const token = parsed.tokens[tokenIndex];
    if (token.depth === parsed.bodyDepth && word(token, 'UNION')) {
      const all = parsed.tokens[tokenIndex + 1];
      assert(word(all, 'ALL'), 'ACL_MUTATION_BRANCH_COUNT_INVALID');
      separators.push({ start: token.start, end: all.end });
      tokenIndex += 1;
    }
  }
  let removeStart;
  let removeEnd;
  if (index === 0) {
    removeStart = parsed.bodyStart;
    removeEnd = separators[0].end;
  } else {
    removeStart = separators[index - 1].start;
    removeEnd = index === branches.length - 1 ? parsed.bodyEnd : separators[index].start;
  }
  return source.slice(0, removeStart) + source.slice(removeEnd);
}

function validateClassSubsequence(classes) {
  assert(classes.length >= 1 && classes.length <= PUBLIC_ACL_OBJECT_CLASSES_V1.length,
    'ACL_MUTATION_BRANCH_COUNT_INVALID');
  let cursor = 0;
  for (const objectClass of classes) {
    while (cursor < PUBLIC_ACL_OBJECT_CLASSES_V1.length
      && PUBLIC_ACL_OBJECT_CLASSES_V1[cursor] !== objectClass) cursor += 1;
    assert(cursor < PUBLIC_ACL_OBJECT_CLASSES_V1.length,
      'ACL_MUTATION_BRANCH_CLASSES_INVALID');
    cursor += 1;
  }
}

function parseColumns(tokens) {
  const result = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    assert(token.kind === 'word' && token.depth === 1,
      'ACL_MUTATION_RAW_RECORD_COLUMNS_INVALID');
    result.push(token.value);
    if (index + 1 < tokens.length) {
      index += 1;
      assert(punct(tokens[index], ',') && tokens[index].depth === 1,
        'ACL_MUTATION_RAW_RECORD_COLUMNS_INVALID');
    }
  }
  return result;
}

function matchingClose(tokens, openIndex) {
  const open = tokens[openIndex];
  assert(punct(open, '('), 'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID');
  for (let index = openIndex + 1; index < tokens.length; index += 1) {
    if (punct(tokens[index], ')') && tokens[index].depth === open.depth) return index;
  }
  throw new Error('ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID');
}

function scan(source) {
  const tokens = [];
  let cursor = 0;
  let depth = 0;
  while (cursor < source.length) {
    const start = cursor;
    const character = source[cursor];
    if (/\s/u.test(character)) { cursor += 1; continue; }
    if (source.startsWith('--', cursor)) {
      const newline = source.indexOf('\n', cursor + 2);
      cursor = newline === -1 ? source.length : newline;
      continue;
    }
    if (source.startsWith('/*', cursor)) {
      cursor = skipBlockComment(source, cursor);
      continue;
    }
    if ((character === 'E' || character === 'e') && source[cursor + 1] === "'") {
      const result = quoted(source, cursor + 1, "'", 'string', true);
      tokens.push({ ...result, start, depth });
      cursor = result.end;
      continue;
    }
    if (character === "'") {
      const result = quoted(source, cursor, "'", 'string');
      tokens.push({ ...result, depth });
      cursor = result.end;
      continue;
    }
    if (character === '"') {
      const result = quoted(source, cursor, '"', 'quoted-identifier');
      tokens.push({ ...result, depth });
      cursor = result.end;
      continue;
    }
    if (character === '$') {
      const delimiter = source.slice(cursor).match(/^\$(?:[A-Za-z_][A-Za-z0-9_]*)?\$/u)?.[0];
      if (delimiter !== undefined) {
        const close = source.indexOf(delimiter, cursor + delimiter.length);
        assert(close !== -1, 'ACL_MUTATION_SQL_LEXICAL_INVALID');
        cursor = close + delimiter.length;
        tokens.push({ kind: 'dollar-string', value: '', start, end: cursor, depth });
        continue;
      }
    }
    const wordMatch = source.slice(cursor).match(/^[A-Za-z_][A-Za-z0-9_$]*/u)?.[0];
    if (wordMatch !== undefined) {
      cursor += wordMatch.length;
      tokens.push({ kind: 'word', value: wordMatch, start, end: cursor, depth });
      continue;
    }
    if (character === '(') {
      cursor += 1;
      tokens.push({ kind: 'punct', value: character, start, end: cursor, depth });
      depth += 1;
      continue;
    }
    if (character === ')') {
      assert(depth > 0, 'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID');
      depth -= 1;
      cursor += 1;
      tokens.push({ kind: 'punct', value: character, start, end: cursor, depth });
      continue;
    }
    cursor += 1;
    tokens.push({ kind: 'punct', value: character, start, end: cursor, depth });
  }
  assert(depth === 0, 'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID');
  return tokens;
}

function quoted(source, start, delimiter, kind, backslashEscapes = false) {
  let cursor = start + 1;
  let value = '';
  while (cursor < source.length) {
    if (backslashEscapes && source[cursor] === '\\') {
      assert(cursor + 1 < source.length, 'ACL_MUTATION_SQL_LEXICAL_INVALID');
      value += source.slice(cursor, cursor + 2);
      cursor += 2;
      continue;
    }
    if (source[cursor] !== delimiter) {
      value += source[cursor];
      cursor += 1;
      continue;
    }
    if (source[cursor + 1] === delimiter) {
      value += delimiter;
      cursor += 2;
      continue;
    }
    return { kind, value, start, end: cursor + 1 };
  }
  throw new Error('ACL_MUTATION_SQL_LEXICAL_INVALID');
}

function skipBlockComment(source, start) {
  let cursor = start + 2;
  let depth = 1;
  while (cursor < source.length && depth > 0) {
    if (source.startsWith('/*', cursor)) { depth += 1; cursor += 2; }
    else if (source.startsWith('*/', cursor)) { depth -= 1; cursor += 2; }
    else cursor += 1;
  }
  assert(depth === 0, 'ACL_MUTATION_SQL_LEXICAL_INVALID');
  return cursor;
}

function validateSource(source) {
  assert(typeof source === 'string' && source.length > 0
    && new TextEncoder().encode(source).byteLength <= MAX_SOURCE_BYTES
    && source.endsWith('\n') && !source.includes('\r') && !source.includes('\0')
    && !source.startsWith('\uFEFF') && !/[\uD800-\uDFFF]/u.test(source),
  'ACL_MUTATION_SOURCE_FRAMING_INVALID');
}

function word(token, value) {
  return token?.kind === 'word' && token.value.toUpperCase() === value.toUpperCase();
}

function punct(token, value) {
  return token?.kind === 'punct' && token.value === value;
}

function sameArray(left, right) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function assert(condition, code) {
  if (!condition) throw new Error(code);
}
