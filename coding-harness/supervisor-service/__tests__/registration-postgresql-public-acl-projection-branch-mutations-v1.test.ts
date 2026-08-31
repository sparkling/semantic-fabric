// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { linkSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
// @ts-expect-error The test exercises the private test-only JavaScript mutator directly.
import * as publicAclProjectionMutationsV1 from '../scripts/postgresql-public-acl-projection-branch-mutations-v1.mjs';
// @ts-expect-error The test exercises the private test-only replay support directly.
import * as publicAclMutationReplaySupportV1 from '../scripts/postgresql-public-acl-mutation-replay-support-v1.mjs';
// @ts-expect-error The test constructs private replay-support snapshots directly.
import * as publicAclReplaySupportV2 from '../scripts/postgresql-public-acl-replay-support-v2.mjs';

const {
  PUBLIC_ACL_OBJECT_CLASSES_V1, buildPublicAclProjectionBranchDeletionMutantsV1,
  buildPublicAclProjectionRecordSetMutantsV1, splitTopLevelRawRecordBranchesV1,
  withExplicitRawRecordColumnsV1,
} = publicAclProjectionMutationsV1;
const {
  MUTATION_REPLAY_TIMEOUTS_V1, buildMutationPsqlInvocationV1,
  classifyMutationCommandResultV1, readMutationSource, runMutationPsql,
  validateMutationContainerSnapshotV1,
} = publicAclMutationReplaySupportV1;
const { IMAGE_CONFIGURATION, IMAGE_REFERENCE, OWNER_LABEL } = publicAclReplaySupportV2;

const ROOT = resolve(import.meta.dirname, '..');
const PROJECTION_PATH = resolve(import.meta.dirname,
  'fixtures/postgresql-16.15-public-acl-projection-v1.sql');
const MUTATOR_PATH = 'scripts/postgresql-public-acl-projection-branch-mutations-v1.mjs';
const RUNNER_PATH = 'scripts/verify-postgresql-public-acl-projection-branch-mutations-v1.mjs';
const SUPPORT_PATH = 'scripts/postgresql-public-acl-mutation-replay-support-v1.mjs';
const TEST_PATH = '__tests__/registration-postgresql-public-acl-projection-branch-mutations-v1.test.ts';
const CLASSES = Object.freeze(['schema', 'relation', 'column', 'routine', 'type', 'language',
  'foreign-data-wrapper', 'foreign-server']);
const COLUMNS = Object.freeze([
  'object_class', 'schema_name', 'object_name', 'subobject_name', 'object_kind',
  'routine_identity_arguments', 'privilege', 'grantable',
]);
const RECORD_SET_MUTATION_IDS = Object.freeze([
  'return-zero', 'omit-first-atom', 'add-sentinel-atom', 'substitute-first-atom',
]);
const SENTINEL = Object.freeze({
  objectClass: 'type',
  schemaName: 'zzzz_sf_public_acl_mutation_v1',
  objectName: 'zzzz_added_atom_v1',
  subobjectName: null,
  objectKind: 'base',
  routineIdentityArguments: null,
  privilege: 'USAGE',
  grantable: false,
});

describe('PostgreSQL PUBLIC ACL projection branch mutation catalogue V1', () => {
  const source = readFileSync(PROJECTION_PATH, 'utf8');

  it('normalizes the real projection without mutating its committed source', () => {
    const before = `${source}`;
    const normalized = withExplicitRawRecordColumnsV1(source);
    expect(source).toBe(before);
    expect(normalized).not.toBe(source);
    expect(normalized).toContain(`WITH raw_records (\n${COLUMNS
      .map((column) => `  ${column}`)
      .join(',\n')}\n) AS (`);
    expect(withExplicitRawRecordColumnsV1(normalized)).toBe(normalized);
    expect(normalized.slice(normalized.indexOf('  SELECT')))
      .toBe(source.slice(source.indexOf('  SELECT')));
  });

  it('finds only the eight top-level UNION ALL branches in their closed class order', () => {
    const branches = splitTopLevelRawRecordBranchesV1(source);
    expect(PUBLIC_ACL_OBJECT_CLASSES_V1).toEqual(CLASSES);
    expect(Object.isFrozen(PUBLIC_ACL_OBJECT_CLASSES_V1)).toBe(true);
    expect(Object.isFrozen(branches)).toBe(true);
    expect(branches.map((branch: { objectClass: string }) => branch.objectClass))
      .toEqual(CLASSES);
    expect(branches).toHaveLength(8);
    for (const branch of branches as readonly { objectClass: string; source: string }[]) {
      expect(Object.isFrozen(branch)).toBe(true);
      expect(branch.source.trimStart()).toMatch(
        new RegExp(`^SELECT '${escapeRegex(branch.objectClass)}'(?:::text)?[,\\s]`, 'u'),
      );
    }
  });

  it('builds eight valid single-branch deletions without changing the baseline', () => {
    const catalogue = buildPublicAclProjectionBranchDeletionMutantsV1(source);
    expect(Object.isFrozen(catalogue)).toBe(true);
    expect(Object.isFrozen(catalogue.mutants)).toBe(true);
    expect(catalogue.objectClasses).toEqual(CLASSES);
    expect(catalogue.mutants).toHaveLength(8);
    expect(catalogue.normalizedSource).toBe(withExplicitRawRecordColumnsV1(source));
    expect(splitTopLevelRawRecordBranchesV1(catalogue.normalizedSource)).toHaveLength(8);
    expect(new Set(catalogue.mutants.map((mutant: { source: string }) => mutant.source)).size)
      .toBe(8);
    for (const [index, mutant] of catalogue.mutants.entries()) {
      expect(Object.isFrozen(mutant)).toBe(true);
      expect(mutant.objectClass).toBe(CLASSES[index]);
      expect(mutant.source).not.toBe(catalogue.normalizedSource);
      const surviving = splitTopLevelRawRecordBranchesV1(mutant.source)
        .map((branch: { objectClass: string }) => branch.objectClass);
      expect(surviving).toEqual(CLASSES.filter((_, branchIndex) => branchIndex !== index));
      expect((mutant.source.match(/\bUNION\s+ALL\b/gu) ?? [])).toHaveLength(6);
      expect(mutant.source).not.toMatch(/\bUNION\b(?!\s+ALL\b)/gu);
    }
  });

  it('builds the four frozen record-set sensitivity mutants without changing the baseline', () => {
    const before = `${source}`;
    const catalogue = buildPublicAclProjectionRecordSetMutantsV1(source);
    expect(source).toBe(before);
    expect(Object.isFrozen(catalogue)).toBe(true);
    expect(Object.isFrozen(catalogue.mutants)).toBe(true);
    expect(Object.isFrozen(catalogue.sentinel)).toBe(true);
    expect(catalogue.authority).toBe('test-only-non-runtime');
    expect(catalogue.sentinel).toEqual(SENTINEL);
    expect(catalogue.mutants.map((mutant: { id: string }) => mutant.id))
      .toEqual(RECORD_SET_MUTATION_IDS);
    expect(new Set(catalogue.mutants
      .map((mutant: { source: string }) => mutant.source)).size).toBe(4);
    for (const mutant of catalogue.mutants as readonly { id: string; source: string }[]) {
      expect(Object.isFrozen(mutant)).toBe(true);
      expect(mutant.source.endsWith(';\n')).toBe(true);
      expect(mutant.source.split(';').length).toBe(catalogue.normalizedSource.split(';').length);
    }
  });

  it('places every record-set mutation at scanner-proven statement boundaries', () => {
    const catalogue = buildPublicAclProjectionRecordSetMutantsV1(source);
    const byId = Object.fromEntries(catalogue.mutants
      .map((mutant: { id: string; source: string }) => [mutant.id, mutant.source]));
    expect(byId['return-zero']).toBe(catalogue.normalizedSource
      .replace('grantable ASC;\n', 'grantable ASC\nLIMIT 0;\n'));
    expect(byId['omit-first-atom']).toBe(catalogue.normalizedSource
      .replace('grantable ASC;\n', 'grantable ASC\nOFFSET 1;\n'));
    expect(byId['add-sentinel-atom']).toContain(
      "  UNION ALL\n  SELECT 'type'::text, 'zzzz_sf_public_acl_mutation_v1'::text,\n",
    );
    expect(byId['add-sentinel-atom']).not.toContain('\nOFFSET 1;\n');
    expect(byId['substitute-first-atom']).toContain(
      "  UNION ALL\n  SELECT 'type'::text, 'zzzz_sf_public_acl_mutation_v1'::text,\n",
    );
    expect(byId['substitute-first-atom']).toContain('\nOFFSET 1;\n');
  });

  it('ignores hostile semicolons while requiring one terminal top-level statement', () => {
    const hostile = syntheticProjection((objectClass, index) => index === 0
      ? `SELECT '${objectClass}' AS object_class, ';'::text, $$;$$, `
        + '/* ; */ (SELECT 1) -- ;\n'
      : `SELECT '${objectClass}' AS object_class, 1`);
    expect(buildPublicAclProjectionRecordSetMutantsV1(hostile).mutants).toHaveLength(4);
    const invalid = [
      hostile.slice(0, -2) + '\n',
      hostile.slice(0, -1) + ';\n',
      hostile + 'SELECT 1;\n',
      hostile.slice(0, -1) + ' -- trailing\n',
    ];
    invalid.forEach((candidate) => expect(
      () => buildPublicAclProjectionRecordSetMutantsV1(candidate),
    ).toThrow('ACL_MUTATION_TERMINAL_STATEMENT_INVALID'));
  });

  it('ignores UNION ALL inside literals, identifiers, comments, dollar quotes and nesting', () => {
    const noisy = syntheticProjection((objectClass, index) => {
      const noise = index === 0
        ? "'UNION ALL'::text, \"UNION ALL\", $$UNION ALL$$, $tag$UNION ALL$tag$, "
          + '(SELECT 1 UNION ALL SELECT 2), /* outer /* UNION ALL */ end */ 1 -- UNION ALL\n'
        : '1';
      return `SELECT '${objectClass}' AS object_class, ${noise}`;
    });
    expect(splitTopLevelRawRecordBranchesV1(noisy)
      .map((branch: { objectClass: string }) => branch.objectClass)).toEqual(CLASSES);
    expect(buildPublicAclProjectionBranchDeletionMutantsV1(noisy).mutants).toHaveLength(8);
  });

  it('does not cut at a fake separator inside a PostgreSQL escape string', () => {
    const escapeString = String.raw`E'foo\' UNION ALL -- hidden close '`;
    const hostile = syntheticProjection((objectClass, index) =>
      `SELECT '${objectClass}' AS object_class, ${index === 0 ? escapeString : '1'}`)
      .replace('\n\n  UNION ALL\n', ' UNION ALL\n');
    expect(splitTopLevelRawRecordBranchesV1(hostile)
      .map((branch: { objectClass: string }) => branch.objectClass)).toEqual(CLASSES);
    const catalogue = buildPublicAclProjectionBranchDeletionMutantsV1(hostile);
    expect(catalogue.mutants).toHaveLength(8);
    expect(catalogue.mutants[1].source).toContain(escapeString);
  });

  it('fails closed on malformed framing, lexical state, structure and class topology', () => {
    const valid = syntheticProjection((objectClass) => `SELECT '${objectClass}' AS object_class`);
    const mutants: ReadonlyArray<readonly [string, string]> = [
      [valid.replace('\n', '\r\n'), 'ACL_MUTATION_SOURCE_FRAMING_INVALID'],
      [`\uFEFF${valid}`, 'ACL_MUTATION_SOURCE_FRAMING_INVALID'],
      [valid.replace("'schema'", "'schema"), 'ACL_MUTATION_SQL_LEXICAL_INVALID'],
      [valid.replace("'schema'", '$tag$schema'), 'ACL_MUTATION_SQL_LEXICAL_INVALID'],
      [valid.replace("'schema'", '/* schema'), 'ACL_MUTATION_SQL_LEXICAL_INVALID'],
      [valid.replace('SELECT \'schema\'', '(SELECT \'schema\''),
        'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID'],
      [valid.replace('\n  UNION ALL\n', '\n  UNION\n'),
        'ACL_MUTATION_BRANCH_COUNT_INVALID'],
      [valid.replace("SELECT 'schema'", "SELECT 'wrong'"),
        'ACL_MUTATION_BRANCH_CLASSES_INVALID'],
      [valid.replace("SELECT 'relation'", "SELECT 'schema'"),
        'ACL_MUTATION_BRANCH_CLASSES_INVALID'],
      [valid.replace("SELECT 'schema'", "SELECT 'temporary'")
        .replace("SELECT 'relation'", "SELECT 'schema'")
        .replace("SELECT 'temporary'", "SELECT 'relation'"),
        'ACL_MUTATION_BRANCH_CLASSES_INVALID'],
      [valid.replace('WITH raw_records AS (', 'WITH raw_records (wrong) AS ('),
        'ACL_MUTATION_RAW_RECORD_COLUMNS_INVALID'],
      [`${valid}\n${valid}`, 'ACL_MUTATION_RAW_RECORDS_STRUCTURE_INVALID'],
    ];
    for (const [mutant, code] of mutants) {
      expect(() => buildPublicAclProjectionBranchDeletionMutantsV1(mutant), code).toThrow(code);
    }
  });

  it('keeps the pure mutator independent of fixture pins, processes and file writes', () => {
    const implementation = readFileSync(
      resolve(ROOT, MUTATOR_PATH), 'utf8',
    );
    expect(implementation).not.toMatch(/a108e05f|4_059|860_988/iu);
    expect(implementation).not.toMatch(/node:(?:child_process|fs)|\bspawn|\bexec|\bwrite/iu);
    expect(implementation.endsWith('\n')).toBe(true);
    expect(implementation.split('\n').length - 1).toBeLessThan(500);
  });

  it('confines replay-support source reads to regular files beneath the service root', () => {
    expect(readMutationSource(ROOT, MUTATOR_PATH, 64 * 1024)).toBeInstanceOf(Buffer);
    const invalid = [
      ['/etc/passwd', 64 * 1024],
      ['../package.json', 64 * 1024],
      [MUTATOR_PATH, 0],
    ] as const;
    invalid.forEach(([path, limit]) => expect(
      () => readMutationSource(ROOT, path, limit),
    ).toThrow('ACL_MUTATION_SOURCE_ARGUMENTS_INVALID'));
    expect(() => readMutationSource(ROOT, MUTATOR_PATH, 1024 * 1024 + 1))
      .toThrow('ACL_MUTATION_SOURCE_ARGUMENTS_INVALID');

    const temporaryRoot = mkdtempSync(resolve(tmpdir(), 'sf-acl-mutation-source-'));
    const outsideRoot = mkdtempSync(resolve(tmpdir(), 'sf-acl-mutation-outside-'));
    try {
      const sourcePath = resolve(temporaryRoot, 'source.sql');
      writeFileSync(sourcePath, 'SELECT 1;\n', { encoding: 'utf8', mode: 0o600 });
      expect(readMutationSource(temporaryRoot, 'source.sql', 10))
        .toEqual(Buffer.from('SELECT 1;\n'));
      expect(() => readMutationSource(temporaryRoot, 'source.sql', 9))
        .toThrow('ACL_MUTATION_SOURCE_FILE_INVALID');
      symlinkSync(sourcePath, resolve(temporaryRoot, 'source-link.sql'));
      mkdirSync(resolve(temporaryRoot, 'directory'));
      writeFileSync(resolve(temporaryRoot, 'hard-source.sql'), 'SELECT 2;\n');
      linkSync(resolve(temporaryRoot, 'hard-source.sql'), resolve(temporaryRoot, 'hard-link.sql'));
      ['source-link.sql', 'directory', 'hard-source.sql', 'hard-link.sql'].forEach((path) =>
        expect(() => readMutationSource(temporaryRoot, path, 64 * 1024))
          .toThrow('ACL_MUTATION_SOURCE_FILE_INVALID'));
      writeFileSync(resolve(outsideRoot, 'outside.sql'), 'SELECT 3;\n');
      symlinkSync(outsideRoot, resolve(temporaryRoot, 'outside-directory'));
      expect(() => readMutationSource(temporaryRoot, 'outside-directory/outside.sql', 64 * 1024))
        .toThrow('ACL_MUTATION_SOURCE_FILE_INVALID');
      const fifoPath = resolve(temporaryRoot, 'source.fifo');
      expect(spawnSync('/usr/bin/mkfifo', ['--mode=600', fifoPath], {
        shell: false, timeout: 5_000,
      }).status).toBe(0);
      const probeSource = `import{readMutationSource as r}from${JSON.stringify(
        new URL('../scripts/postgresql-public-acl-mutation-replay-support-v1.mjs', import.meta.url).href,
      )};try{r(process.argv[1],'source.fifo',65536);process.exit(2)}catch(e){if(e?.message!=='ACL_MUTATION_SOURCE_FILE_INVALID')process.exit(3)}`;
      const probe = spawnSync(process.execPath,
        ['--input-type=module', '--eval', probeSource, '--', temporaryRoot],
        { shell: false, timeout: 3_000, killSignal: 'SIGKILL', maxBuffer: 64 * 1024 });
      expect({ error: (probe.error as NodeJS.ErrnoException | undefined)?.code,
        status: probe.status, signal: probe.signal })
        .toEqual({ error: undefined, status: 0, signal: null });
    } finally {
      rmSync(temporaryRoot, { recursive: true, force: true });
      rmSync(outsideRoot, { recursive: true, force: true });
    }
  });

  it('freezes exact psql arguments and reserves parent time for postflight', () => {
    const id = 'a'.repeat(64);
    expect(buildMutationPsqlInvocationV1(id)).toEqual([
      'exec', '-i', id, 'psql', '-U', 'postgres', '-d', 'sf_public_baseline',
      '-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1',
    ]);
    expect(Object.isFrozen(buildMutationPsqlInvocationV1(id))).toBe(true);
    expect(() => buildMutationPsqlInvocationV1('--help'))
      .toThrow('ACL_MUTATION_PSQL_ARGUMENTS_INVALID');
    expect(runMutationPsql).toHaveLength(5);
    expect(() => runMutationPsql(
      ROOT, id, Buffer.from('SELECT 1;\n', 'utf8'), 64 * 1024, 'invalid',
    )).toThrow('ACL_MUTATION_PSQL_ARGUMENTS_INVALID');
    expect(MUTATION_REPLAY_TIMEOUTS_V1).toEqual({
      containerInspectMs: 15_000,
      probePsqlMs: 15_000,
      sessionPsqlMs: 60_000,
      boundedExternalMs: 120_000,
      parentChildMs: 300_000,
      parentHeadroomMs: 180_000,
    });
    expect(Object.isFrozen(MUTATION_REPLAY_TIMEOUTS_V1)).toBe(true);
    const parentSupport = readFileSync(
      resolve(ROOT, 'scripts/postgresql-public-acl-replay-support-v2.mjs'), 'utf8',
    );
    const parentRunChild = parentSupport.match(
      /function runChild\(root, path, name, prefix\) \{[\s\S]*?\n\}/u,
    )?.[0];
    expect(parentRunChild).toContain('process.execPath, [path, name], undefined, 300_000');
    expect(parentRunChild?.match(/300_000/gu)).toHaveLength(1);
    expect(MUTATION_REPLAY_TIMEOUTS_V1.boundedExternalMs
      + MUTATION_REPLAY_TIMEOUTS_V1.parentHeadroomMs)
      .toBe(MUTATION_REPLAY_TIMEOUTS_V1.parentChildMs);
  });

  it('classifies bounded command failures without weakening fail-closed output checks', () => {
    const ok = { error: undefined, signal: null, status: 0,
      stdout: Buffer.from('ok'), stderr: Buffer.alloc(0) };
    expect(classifyMutationCommandResultV1(ok, 'ACL_MUTATION_TEST_FAILED'))
      .toEqual(Buffer.from('ok'));
    const systemError = (code: string) => Object.assign(new Error(code), { code });
    const failures = [
      [{ ...ok, error: systemError('ETIMEDOUT') }, 'ACL_MUTATION_TEST_FAILED_TIMEOUT'],
      [{ ...ok, error: systemError('ENOBUFS') }, 'ACL_MUTATION_TEST_FAILED_OUTPUT_LIMIT'],
      [{ ...ok, error: systemError('ENOENT') }, 'ACL_MUTATION_TEST_FAILED_SPAWN'],
      [{ ...ok, signal: 'SIGKILL' }, 'ACL_MUTATION_TEST_FAILED_SIGNAL'],
      [{ ...ok, status: 1 }, 'ACL_MUTATION_TEST_FAILED_STATUS'],
      [{ ...ok, stderr: Buffer.from('bad') }, 'ACL_MUTATION_TEST_FAILED_STDERR'],
      [{ ...ok, stdout: 'bad' }, 'ACL_MUTATION_TEST_FAILED_RESULT'],
    ] as const;
    failures.forEach(([result, code]) =>
      expect(() => classifyMutationCommandResultV1(result, 'ACL_MUTATION_TEST_FAILED'))
        .toThrow(code));
  });

  it('makes every container identity and isolation predicate family load-bearing', () => {
    const name = 'sf-pgacl-v2-test';
    const snapshot = validMutationContainerSnapshot(name);
    const accepted = validateMutationContainerSnapshotV1(name, snapshot.container, snapshot.image);
    expect(accepted).toEqual({ id: 'a'.repeat(64), volumeName: 'b'.repeat(64) });
    expect(Object.isFrozen(accepted)).toBe(true);
    const mutations: ReadonlyArray<readonly ['container' | 'image', string, unknown]> = [
      ['image', 'Id', 'wrong'], ['image', 'Os', 'wrong'], ['image', 'Architecture', 'wrong'],
      ['image', 'RepoDigests', []], ['image', 'Config', null],
      ['image', 'Config.Env', ['WRONG=1']], ['image', 'Config.Env', 'wrong'], ['image', 'Config.Env', {}],
      ['container', 'Id', 'wrong'], ['container', 'Name', '/wrong'],
      ['container', 'Config', null], ['container', 'Config.Image', 'wrong'],
      ['container', 'Image', 'wrong'], ['container', 'HostConfig', null],
      ['container', 'State.Running', false], ['container', 'HostConfig.NetworkMode', 'bridge'],
      ['container', 'HostConfig.PublishAllPorts', true],
      ['container', 'HostConfig.PortBindings', { 5432: [{}] }],
      ['container', 'NetworkSettings.Ports', { 5432: [{}] }],
      ['container', 'Config.Labels', {}], ['container', 'Config.Entrypoint', ['wrong']],
      ['container', 'Config.Cmd', ['wrong']], ['container', 'Config.Env', ['WRONG=1']],
      ['container', 'Mounts', [...snapshot.container.Mounts, snapshot.container.Mounts[0]]],
      ['container', 'Mounts.0.Type', 'bind'],
      ['container', 'Mounts.0.Destination', '/wrong'], ['container', 'Mounts.0.RW', false],
      ['container', 'Mounts.0.Name', 'wrong'],
      ['container', 'HostConfig.Mounts', [
        ...snapshot.container.HostConfig.Mounts, snapshot.container.HostConfig.Mounts[0],
      ]],
      ['container', 'HostConfig.Mounts.0.Type', 'bind'], ['container', 'HostConfig.Mounts.0.Target', '/wrong'],
      ['container', 'HostConfig.Mounts.0.Source', '/host'], ['container', 'HostConfig.Mounts.0.ReadOnly', true],
      ['container', 'HostConfig.Binds', ['/host:/data']], ['container', 'HostConfig.Tmpfs', { '/tmp': 'rw' }],
    ];
    mutations.forEach(([target, path, replacement]) => {
      const value = structuredClone(snapshot) as Record<string, any>;
      const parts = path.split('.'); const key = parts.pop() as string;
      const owner = parts.reduce((entry, part) => entry[part], value[target]);
      owner[key] = replacement;
      expect(() => validateMutationContainerSnapshotV1(name, value.container, value.image))
        .toThrow('ACL_MUTATION_CONTAINER_IDENTITY_INVALID');
    });
    const invalidName = '--invalid'; const invalidNamed = structuredClone(snapshot.container);
    invalidNamed.Name = `/${invalidName}`;
    expect(() => validateMutationContainerSnapshotV1(invalidName, invalidNamed, snapshot.image))
      .toThrow('ACL_MUTATION_CONTAINER_IDENTITY_INVALID');
    [null, [], {}].forEach((value) => {
      expect(() => validateMutationContainerSnapshotV1(name, value, snapshot.image))
        .toThrow('ACL_MUTATION_CONTAINER_IDENTITY_INVALID');
      expect(() => validateMutationContainerSnapshotV1(name, snapshot.container, value))
        .toThrow('ACL_MUTATION_CONTAINER_IDENTITY_INVALID');
    });
  });

  it('owns two networkless runs while keeping mutation detection independent', () => {
    const runner = readFileSync(resolve(ROOT, RUNNER_PATH), 'utf8');
    const support = readFileSync(resolve(ROOT, SUPPORT_PATH), 'utf8');
    expect(runner).toContain('runOwnedReplayPair');
    expect(runner).toContain("authority: 'test-only-non-runtime'");
    expect(runner).toContain('enforceCleanProfile: false');
    expect(runner).toContain('parseOracleSession');
    expect(runner).toContain('parseProjectionRecords');
    expect(runner).toContain('compareRecordBags');
    expect(runner).toContain('compareRecords');
    expect(runner).toContain('canonicalFixture');
    expect(runner).toContain('ORACLE_RECORD_BAG_KEYS_MISMATCH');
    expect(runner).toContain('ORACLE_RECORD_BAG_MULTIPLICITY_MISMATCH');
    RECORD_SET_MUTATION_IDS.forEach((id) => expect(runner).toContain(`'${id}'`));
    expect(runner).toContain('expected.slice(1)');
    expect(runner).toContain('catalogue.recordSet.sentinel');
    expect(runner).toContain('CREATE FOREIGN DATA WRAPPER sf_public_acl_mutation_fdw_v1');
    expect(runner).toContain('CREATE SERVER sf_public_acl_mutation_server_v1');
    expect(runner).toContain('GRANT USAGE ON FOREIGN DATA WRAPPER');
    expect(runner).toContain('GRANT USAGE ON FOREIGN SERVER');
    expect(support).toContain("MUTATION_PSQL_TIMEOUTS_V1[operation]");
    expect(support).toContain('constants.O_NOFOLLOW');
    expect(support).toContain('constants.O_NONBLOCK');
    expect(support).toContain('components.slice(0, -1)');
    expect(support).toContain("`/proc/self/fd/${fileDescriptor}`");
    expect(runner).toContain("runPsql(container.id, session, MAX_TRANSCRIPT_BYTES, 'session')");
    expect(runner.split("64 * 1024, 'probe')")).toHaveLength(2);
    expect(runner).toContain('raw.FDW.length === 2 && raw.SERVER.length === 2');
    expect(runner).toContain("slice(19, 25).join(',') === '1,2,2,1,2,2'");
    expect(runner).toContain('ROLLBACK;');
    expect(runner).not.toMatch(/a108e05f|4_059|860_988/iu);
    expect(runner).not.toMatch(/\bwriteFile(?:Sync)?\b|\bappendFile(?:Sync)?\b/u);
    expect(runner).not.toContain("['run', '--detach'");
  });

  it('gates the live mutation proof after V1 and V2 on exact Node 20 and 24', () => {
    const workflow = readFileSync(resolve(ROOT, '../../.github/workflows/ci.yml'), 'utf8');
    const start = workflow.indexOf('  postgresql-public-acl-replay:');
    const end = workflow.indexOf('\n  build:', start);
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const job = workflow.slice(start, end);
    expect(job.match(/- node: '[^']+'/gu)).toEqual(["- node: '20.0.0'", "- node: '24.14.1'"]);
    const commands = [
      'node coding-harness/supervisor-service/scripts/replay-postgresql-public-acl-baseline-v1.mjs',
      'node coding-harness/supervisor-service/scripts/replay-postgresql-public-acl-baseline-v2.mjs',
      `node coding-harness/supervisor-service/${RUNNER_PATH}`,
    ];
    expect(job.match(/timeout-minutes: 120/gu)).toHaveLength(1);
    const step = job.slice(job.indexOf('      - name: replay V1, V2, and projection mutations'))
      .trimEnd();
    expect(step).toBe([
      '      - name: replay V1, V2, and projection mutations from owned isolated containers',
      '        run: |',
      ...commands.map((command) => `          ${command}`),
    ].join('\n'));
  });

  it('keeps all mutation machinery protected, test-only and below 500 lines', () => {
    const paths = [MUTATOR_PATH, RUNNER_PATH, SUPPORT_PATH, TEST_PATH];
    const artifact = JSON.parse(
      readFileSync(resolve(ROOT, '.service/artifact.json'), 'utf8'),
    ) as { buildInputs: Record<string, string>; sourceInputs: Record<string, string> };
    const inputs = [...Object.keys(artifact.buildInputs), ...Object.keys(artifact.sourceInputs)];
    const manifest = JSON.parse(
      readFileSync(resolve(ROOT, '../.harness/manifest.json'), 'utf8'),
    ) as { protectedPaths: string[] };
    const registry = readFileSync(
      resolve(ROOT, '../src/programme-capture-protected-paths-v1.ts'), 'utf8',
    );
    paths.forEach((path) => {
      expect(inputs).not.toContain(path);
      const repositoryPath = `coding-harness/supervisor-service/${path}`;
      expect(manifest.protectedPaths.filter((value) => value === repositoryPath)).toHaveLength(1);
      expect(registry.split(`'${repositoryPath}'`)).toHaveLength(2);
      const file = readFileSync(resolve(ROOT, path), 'utf8');
      expect(file.endsWith('\n')).toBe(true);
      expect(file.split('\n').length - 1, path).toBeLessThan(500);
    });
  });
});

function syntheticProjection(branch: (objectClass: string, index: number) => string): string {
  return `WITH raw_records AS (\n${CLASSES.map((objectClass, index) =>
    `  ${branch(objectClass, index)}`).join('\n\n  UNION ALL\n')}\n)\nSELECT * FROM raw_records;\n`;
}

function escapeRegex(value: string): string { return value.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&'); }

function validMutationContainerSnapshot(name: string): {
  container: Record<string, any>; image: Record<string, any>;
} {
  const image = {
    Id: IMAGE_CONFIGURATION, Os: 'linux', Architecture: 'amd64', RepoDigests: [IMAGE_REFERENCE],
    Config: { Env: ['LANG=C'], Entrypoint: ['/entrypoint'], Cmd: ['postgres'] },
  };
  return {
    image,
    container: {
      Id: 'a'.repeat(64), Name: `/${name}`, Image: IMAGE_CONFIGURATION, State: { Running: true },
      Config: {
        Image: IMAGE_REFERENCE, Labels: { [OWNER_LABEL]: 'c'.repeat(32) },
        Entrypoint: image.Config.Entrypoint, Cmd: image.Config.Cmd,
        Env: [...image.Config.Env, 'POSTGRES_HOST_AUTH_METHOD=trust',
          'POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8'],
      },
      HostConfig: {
        NetworkMode: 'none', PublishAllPorts: false, PortBindings: null,
        Mounts: [{ Type: 'volume', Target: '/var/lib/postgresql/data', Source: '' }],
      },
      NetworkSettings: { Ports: null },
      Mounts: [{ Type: 'volume', Destination: '/var/lib/postgresql/data', RW: true,
        Name: 'b'.repeat(64) }],
    },
  };
}
