// SPDX-License-Identifier: MIT

import type { HarnessConfig } from './contracts.js';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  asInteger,
  asNonEmptyString,
  asUniqueStrings,
  assertExactKeys,
  assertStructuredText,
  deepFreeze,
  normalizeWorkspacePath,
  pathsOverlap,
} from './contracts.js';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_PROFILE_PATH =
  'crates/sf-bench/config/performance-runner-profile-v1.tsv' as const;
export const PROGRAMME_CAPTURE_SCENARIOS_PATH =
  'crates/sf-bench/config/performance-scenarios-v1.tsv' as const;
export const PROGRAMME_CAPTURE_OUTPUT_PATH =
  'crates/sf-bench/config/performance-baseline-v1.tsv' as const;
export const PROGRAMME_CAPTURE_MAXIMUM_BYTES = 1_048_576 as const;
export const PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS = Object.freeze([
  'Cargo.toml',
  'crates/sf-bench/Cargo.toml',
  'crates/sf-bench/src/bin/sf-performance-receipt.rs',
  'crates/sf-bench/src/driver.rs',
  'crates/sf-bench/src/lib.rs',
  'crates/sf-bench/src/mem.rs',
  'crates/sf-bench/src/performance/bounded_io.rs',
  'crates/sf-bench/src/performance/capture.rs',
  'crates/sf-bench/src/performance/compare.rs',
  'crates/sf-bench/src/performance/config.rs',
  'crates/sf-bench/src/performance/digest.rs',
  'crates/sf-bench/src/performance/format.rs',
  'crates/sf-bench/src/performance/mod.rs',
  'crates/sf-bench/src/performance/model.rs',
  'crates/sf-bench/src/performance/paths.rs',
  'crates/sf-bench/src/performance/proc_status.rs',
  'crates/sf-bench/src/performance/producer.rs',
  'crates/sf-bench/src/performance/profile.rs',
  'crates/sf-bench/src/performance/source.rs',
  'crates/sf-bench/src/performance/stats.rs',
  'crates/sf-bench/src/performance/subprocess.rs',
  'crates/sf-bench/src/performance/worker.rs',
  'crates/sf-bench/src/performance/workload_runner.rs',
  'crates/sf-bench/src/workload.rs',
  'crates/sf-core/Cargo.toml',
  'crates/sf-core/src/datatype.rs',
  'crates/sf-core/src/graph_map.rs',
  'crates/sf-core/src/ir.rs',
  'crates/sf-core/src/lib.rs',
  'crates/sf-core/src/term.rs',
  'crates/sf-mapping/Cargo.toml',
  'crates/sf-mapping/src/direct_mapping.rs',
  'crates/sf-mapping/src/lib.rs',
  'crates/sf-mapping/src/r2rml.rs',
  'crates/sf-mapping/src/r2rml/sql.rs',
  'crates/sf-mapping/src/r2rml/star.rs',
  'crates/sf-mapping/src/r2rml/star/ids.rs',
  'crates/sf-mapping/src/r2rml/tests.rs',
  'crates/sf-sparql/Cargo.toml',
  'crates/sf-sparql/src/build.rs',
  'crates/sf-sparql/src/cache.rs',
  'crates/sf-sparql/src/cascade/fd.rs',
  'crates/sf-sparql/src/cascade/joinelim.rs',
  'crates/sf-sparql/src/cascade/mod.rs',
  'crates/sf-sparql/src/cascade/sameterm.rs',
  'crates/sf-sparql/src/cascade/tests.rs',
  'crates/sf-sparql/src/cascade/ws_fk.rs',
  'crates/sf-sparql/src/cascade/ws_g.rs',
  'crates/sf-sparql/src/cascade/ws_st.rs',
  'crates/sf-sparql/src/dump.rs',
  'crates/sf-sparql/src/emit.rs',
  'crates/sf-sparql/src/exec.rs',
  'crates/sf-sparql/src/exec_core.rs',
  'crates/sf-sparql/src/exec_mysql.rs',
  'crates/sf-sparql/src/exec_pg.rs',
  'crates/sf-sparql/src/graph_map.rs',
  'crates/sf-sparql/src/iq.rs',
  'crates/sf-sparql/src/iq/lower.rs',
  'crates/sf-sparql/src/iq/node.rs',
  'crates/sf-sparql/src/iq/normalize.rs',
  'crates/sf-sparql/src/iq/resolve.rs',
  'crates/sf-sparql/src/leftjoin.rs',
  'crates/sf-sparql/src/lib.rs',
  'crates/sf-sparql/src/path.rs',
  'crates/sf-sparql/src/saturate.rs',
  'crates/sf-sparql/src/star.rs',
  'crates/sf-sparql/src/star/collect_vars.rs',
  'crates/sf-sparql/src/star/decompose.rs',
  'crates/sf-sparql/src/star/env.rs',
  'crates/sf-sparql/src/star/expr.rs',
  'crates/sf-sparql/src/star/tests.rs',
  'crates/sf-sparql/src/star/top_level.rs',
  'crates/sf-sparql/src/star/util.rs',
  'crates/sf-sparql/src/star/walk.rs',
  'crates/sf-sparql/src/unfold.rs',
  'crates/sf-sparql/src/unify.rs',
  'crates/sf-sql/Cargo.toml',
  'crates/sf-sql/src/backend.rs',
  'crates/sf-sql/src/backend/duckdb.rs',
  'crates/sf-sql/src/backend/hana.rs',
  'crates/sf-sql/src/backend/monetdb.rs',
  'crates/sf-sql/src/backend/mysql.rs',
  'crates/sf-sql/src/backend/odbc.rs',
  'crates/sf-sql/src/backend/oracle.rs',
  'crates/sf-sql/src/backend/pg.rs',
  'crates/sf-sql/src/backend/redshift.rs',
  'crates/sf-sql/src/backend/rest.rs',
  'crates/sf-sql/src/backend/sqlite.rs',
  'crates/sf-sql/src/backend/sqlserver.rs',
  'crates/sf-sql/src/cost.rs',
  'crates/sf-sql/src/dialect.rs',
  'crates/sf-sql/src/error.rs',
  'crates/sf-sql/src/introspect.rs',
  'crates/sf-sql/src/lib.rs',
  'crates/sf-sql/src/schema.rs',
  'crates/sf-sql/src/stream.rs',
  'rust-toolchain.toml',
] as const);

const CAPTURE_TIMEOUT_MS = 1_800_000 as const;
const VERIFY_TIMEOUT_MS = 60_000 as const;

interface DigestBinding {
  readonly path: string;
  readonly sha256: string;
}

interface CaptureCommand {
  readonly tool: 'sf-performance-receipt';
  readonly executable: 'target/release/sf-performance-receipt';
  readonly argv: readonly [string];
  readonly cwd: '.';
  readonly env: Readonly<Record<string, never>>;
  readonly timeoutMs: typeof CAPTURE_TIMEOUT_MS | typeof VERIFY_TIMEOUT_MS;
  readonly maxOutputBytes: typeof PROGRAMME_CAPTURE_MAXIMUM_BYTES;
}

interface NamedCaptureCommand {
  readonly commandId: string;
  readonly command: CaptureCommand;
}

export interface ProgrammeCaptureTaskV1 {
  readonly schemaVersion: 1;
  readonly taskKind: 'controlled-performance-baseline';
  readonly taskId: string;
  readonly workItem: string;
  readonly objective: string;
  readonly invariants: readonly string[];
  readonly exclusions: readonly string[];
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly inputs: Readonly<{
    runnerProfile: DigestBinding;
    scenarios: DigestBinding;
    cargoLock: DigestBinding;
    workloadSha256: string;
    sources: readonly DigestBinding[];
  }>;
  readonly commands: Readonly<{
    capture: NamedCaptureCommand;
    verify: NamedCaptureCommand;
  }>;
  readonly output: Readonly<{
    path: typeof PROGRAMME_CAPTURE_OUTPUT_PATH;
    mode: 'create-new';
    mediaType: 'text/tab-separated-values; charset=utf-8';
    maximumBytes: typeof PROGRAMME_CAPTURE_MAXIMUM_BYTES;
  }>;
  readonly policy: Readonly<{
    measurementNetwork: 'offline';
    modelTransport: 'native-first-party-only';
    nativeHosts: readonly ['codex', 'claude-code'];
    dualReview: Readonly<{ preCapture: true; postCapture: true }>;
    maximumMeasurementAttempts: 1;
    automaticMeasurementRetries: 0;
    automaticRepairs: 0;
    modelMeasurementOverlap: 'forbidden';
    coreEvidence: 'fail-closed';
  }>;
  readonly routing: Readonly<{
    tags: readonly string[];
    difficulty: number;
    evolutionEligible: false;
  }>;
}

const TOP_KEYS = [
  'schemaVersion', 'taskKind', 'taskId', 'workItem', 'objective', 'invariants', 'exclusions',
  'authority', 'inputs', 'commands', 'output', 'policy', 'routing',
] as const;

export function parseProgrammeCaptureTaskBlobV1(
  serialized: string,
  config: HarnessConfig,
): ProgrammeCaptureTaskV1 {
  if (typeof serialized !== 'string' || Buffer.byteLength(serialized, 'utf8') > 1_048_576) {
    throw new TypeError('programme capture task blob must be bounded UTF-8 JSON text');
  }
  return parseProgrammeCaptureTaskV1(
    parseJsonWithoutDuplicateKeys(serialized, 'programme capture task'),
    config,
  );
}

export function parseProgrammeCaptureTaskV1(
  value: unknown,
  config: HarnessConfig,
): ProgrammeCaptureTaskV1 {
  const input = asClosedRecord(value, 'programme capture task');
  assertExactKeys(input, TOP_KEYS, 'programme capture task');
  if (input.schemaVersion !== 1) {
    throw new TypeError('programme capture task.schemaVersion must be 1');
  }
  if (input.taskKind !== 'controlled-performance-baseline') {
    throw new TypeError('programme capture task.taskKind is invalid');
  }
  if (input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('programme capture task cannot grant promotion authority');
  }

  const inputs = parseInputs(input.inputs, config);
  const commands = parseCommands(input.commands, config);
  const output = parseOutput(input.output);
  assertInputOutputSeparation(inputs, output.path);
  const policy = parsePolicy(input.policy);
  const routing = parseRouting(input.routing);

  return deepFreeze({
    schemaVersion: 1,
    taskKind: 'controlled-performance-baseline',
    taskId: parseTaskOpaqueId(input.taskId, 'programme capture task.taskId'),
    workItem: parsePromptText(input.workItem, 'programme capture task.workItem', 256),
    objective: parsePromptText(input.objective, 'programme capture task.objective', 2_000),
    invariants: parsePromptList(input.invariants, 'programme capture task.invariants'),
    exclusions: parsePromptList(input.exclusions, 'programme capture task.exclusions'),
    authority: DEVELOPMENT_AUTHORITY,
    inputs,
    commands,
    output,
    policy,
    routing,
  });
}

function parseInputs(value: unknown, config: HarnessConfig): ProgrammeCaptureTaskV1['inputs'] {
  const input = asClosedRecord(value, 'programme capture task.inputs');
  assertExactKeys(
    input,
    ['runnerProfile', 'scenarios', 'cargoLock', 'workloadSha256', 'sources'],
    'programme capture task.inputs',
  );
  const sourceValues = asDenseArray(input.sources, 'programme capture task.inputs.sources');
  const sources = sourceValues.map((binding, index) => parseBinding(
    binding,
    `programme capture task.inputs.sources[${index}]`,
  ));
  const sourcePaths = sources.map(({ path }) => path);
  if (sourcePaths.length !== PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS.length
    || sourcePaths.some((path, index) => path !== PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS[index])) {
    throw new TypeError('programme capture task source paths must equal the exact local build-source closure');
  }
  const runnerProfile = parseFixedBinding(
    input.runnerProfile,
    'programme capture task.inputs.runnerProfile',
    PROGRAMME_CAPTURE_PROFILE_PATH,
  );
  const scenarios = parseFixedBinding(
    input.scenarios,
    'programme capture task.inputs.scenarios',
    PROGRAMME_CAPTURE_SCENARIOS_PATH,
  );
  const cargoLock = parseFixedBinding(
    input.cargoLock,
    'programme capture task.inputs.cargoLock',
    'Cargo.lock',
  );
  const protectedInputs = [runnerProfile.path, scenarios.path, cargoLock.path, ...sourcePaths];
  if (protectedInputs.some((path) => !config.requiredProtectedPaths.includes(path))) {
    throw new TypeError('programme capture task inputs must be protected capture-task inputs');
  }
  return {
    runnerProfile,
    scenarios,
    cargoLock,
    workloadSha256: parseDigest(
      input.workloadSha256,
      'programme capture task.inputs.workloadSha256',
    ),
    sources,
  };
}

function parseBinding(value: unknown, label: string): DigestBinding {
  const input = asClosedRecord(value, label);
  assertExactKeys(input, ['path', 'sha256'], label);
  const path = normalizeWorkspacePath(input.path, `${label}.path`);
  if (Buffer.byteLength(path, 'utf8') > 512) throw new TypeError(`${label}.path is too long`);
  return {
    path,
    sha256: parseDigest(input.sha256, `${label}.sha256`),
  };
}

function parseFixedBinding(value: unknown, label: string, expectedPath: string): DigestBinding {
  const binding = parseBinding(value, label);
  if (binding.path !== expectedPath) throw new TypeError(`${label}.path is not the fixed authority path`);
  return binding;
}

function parseCommands(
  value: unknown,
  config: HarnessConfig,
): ProgrammeCaptureTaskV1['commands'] {
  const input = asClosedRecord(value, 'programme capture task.commands');
  assertExactKeys(input, ['capture', 'verify'], 'programme capture task.commands');
  const capture = parseNamedCommand(
    input.capture, 'capture-baseline', CAPTURE_TIMEOUT_MS, config, 'capture',
  );
  const verify = parseNamedCommand(
    input.verify, 'check-baseline', VERIFY_TIMEOUT_MS, config, 'verify',
  );
  if (capture.commandId === verify.commandId) {
    throw new TypeError('programme capture task command IDs must be unique');
  }
  return { capture, verify };
}

function parseNamedCommand(
  value: unknown,
  expectedArgument: string,
  expectedTimeoutMs: typeof CAPTURE_TIMEOUT_MS | typeof VERIFY_TIMEOUT_MS,
  config: HarnessConfig,
  name: string,
): NamedCaptureCommand {
  const label = `programme capture task.commands.${name}`;
  const input = asClosedRecord(value, label);
  assertExactKeys(input, ['commandId', 'command'], label);
  const commandInput = asClosedRecord(input.command, `${label}.command`);
  assertExactKeys(
    commandInput,
    ['tool', 'executable', 'argv', 'cwd', 'env', 'timeoutMs', 'maxOutputBytes'],
    `${label}.command`,
  );
  if (commandInput.tool !== 'sf-performance-receipt'
    || commandInput.executable !== 'target/release/sf-performance-receipt') {
    throw new TypeError(`${label}.command must bind the exact trusted release producer`);
  }
  const argv = asDenseArray(commandInput.argv, `${label}.command.argv`);
  if (argv.length !== 1 || argv[0] !== expectedArgument) {
    throw new TypeError(`${label}.command.argv is invalid`);
  }
  if (commandInput.cwd !== '.') throw new TypeError(`${label}.command.cwd must be the repository root`);
  const env = asClosedRecord(commandInput.env, `${label}.command.env`);
  assertExactKeys(env, [], `${label}.command.env`);
  const timeoutMs = asInteger(commandInput.timeoutMs, `${label}.command.timeoutMs`, 1);
  const maxOutputBytes = asInteger(commandInput.maxOutputBytes, `${label}.command.maxOutputBytes`, 1);
  if (config.limits.maxTimeoutMs < expectedTimeoutMs
    || config.limits.maxOutputBytes < PROGRAMME_CAPTURE_MAXIMUM_BYTES) {
    throw new TypeError(`${label}.command cannot be authorized by the configured ceilings`);
  }
  if (timeoutMs !== expectedTimeoutMs
    || maxOutputBytes !== PROGRAMME_CAPTURE_MAXIMUM_BYTES) {
    throw new TypeError(`${label}.command must use the exact controlled resource bounds`);
  }
  return {
    commandId: parseTaskOpaqueId(input.commandId, `${label}.commandId`),
    command: {
      tool: 'sf-performance-receipt',
      executable: 'target/release/sf-performance-receipt',
      argv: [expectedArgument],
      cwd: '.',
      env: {},
      timeoutMs: expectedTimeoutMs,
      maxOutputBytes: PROGRAMME_CAPTURE_MAXIMUM_BYTES,
    },
  };
}

function parseOutput(value: unknown): ProgrammeCaptureTaskV1['output'] {
  const input = asClosedRecord(value, 'programme capture task.output');
  assertExactKeys(input, ['path', 'mode', 'mediaType', 'maximumBytes'], 'programme capture task.output');
  if (input.path !== PROGRAMME_CAPTURE_OUTPUT_PATH
    || input.mode !== 'create-new'
    || input.mediaType !== 'text/tab-separated-values; charset=utf-8'
    || input.maximumBytes !== PROGRAMME_CAPTURE_MAXIMUM_BYTES) {
    throw new TypeError('programme capture task.output must preserve the fixed create-new baseline contract');
  }
  return {
    path: PROGRAMME_CAPTURE_OUTPUT_PATH,
    mode: 'create-new',
    mediaType: 'text/tab-separated-values; charset=utf-8',
    maximumBytes: PROGRAMME_CAPTURE_MAXIMUM_BYTES,
  };
}

function parsePolicy(value: unknown): ProgrammeCaptureTaskV1['policy'] {
  const input = asClosedRecord(value, 'programme capture task.policy');
  assertExactKeys(input, [
    'measurementNetwork', 'modelTransport', 'nativeHosts', 'dualReview',
    'maximumMeasurementAttempts', 'automaticMeasurementRetries', 'automaticRepairs',
    'modelMeasurementOverlap', 'coreEvidence',
  ], 'programme capture task.policy');
  const review = asClosedRecord(input.dualReview, 'programme capture task.policy.dualReview');
  assertExactKeys(review, ['preCapture', 'postCapture'], 'programme capture task.policy.dualReview');
  const hosts = asDenseArray(input.nativeHosts, 'programme capture task.policy.nativeHosts');
  if (input.measurementNetwork !== 'offline'
    || input.modelTransport !== 'native-first-party-only'
    || hosts.length !== 2
    || hosts[0] !== 'codex'
    || hosts[1] !== 'claude-code'
    || review.preCapture !== true
    || review.postCapture !== true
    || input.maximumMeasurementAttempts !== 1
    || input.automaticMeasurementRetries !== 0
    || input.automaticRepairs !== 0
    || input.modelMeasurementOverlap !== 'forbidden'
    || input.coreEvidence !== 'fail-closed') {
    throw new TypeError('programme capture task.policy weakens the terminal controlled-capture contract');
  }
  return {
    measurementNetwork: 'offline',
    modelTransport: 'native-first-party-only',
    nativeHosts: ['codex', 'claude-code'],
    dualReview: { preCapture: true, postCapture: true },
    maximumMeasurementAttempts: 1,
    automaticMeasurementRetries: 0,
    automaticRepairs: 0,
    modelMeasurementOverlap: 'forbidden',
    coreEvidence: 'fail-closed',
  };
}

function parseRouting(value: unknown): ProgrammeCaptureTaskV1['routing'] {
  const input = asClosedRecord(value, 'programme capture task.routing');
  assertExactKeys(input, ['tags', 'difficulty', 'evolutionEligible'], 'programme capture task.routing');
  asDenseArray(input.tags, 'programme capture task.routing.tags');
  const tags = asUniqueStrings(input.tags, 'programme capture task.routing.tags');
  tags.forEach((tag) => assertStructuredText(tag, 'programme capture task.routing.tags entry'));
  if (tags.some((tag, index) => tag !== [...tags].sort(compareUtf8)[index])) {
    throw new TypeError('programme capture task.routing.tags must be UTF-8 byte sorted');
  }
  if (typeof input.difficulty !== 'number'
    || !Number.isFinite(input.difficulty)
    || input.difficulty < 0
    || input.difficulty > 1
    || input.evolutionEligible !== false) {
    throw new TypeError('programme capture task.routing is invalid or evolution eligible');
  }
  return { tags, difficulty: input.difficulty, evolutionEligible: false };
}

function assertInputOutputSeparation(
  inputs: ProgrammeCaptureTaskV1['inputs'],
  outputPath: string,
): void {
  const fixed = [inputs.runnerProfile.path, inputs.scenarios.path, inputs.cargoLock.path, outputPath];
  if (inputs.sources.some(({ path }) => fixed.some((authority) => pathsOverlap(path, authority)))) {
    throw new TypeError('programme capture task source paths overlap fixed authorities or output');
  }
}

function parseDigest(value: unknown, label: string): string {
  const digest = asNonEmptyString(value, label);
  if (!SHA256_PATTERN.test(digest) || digest === '0'.repeat(64)) {
    throw new TypeError(`${label} must be a nonzero lowercase SHA-256 digest`);
  }
  return digest;
}

function parsePromptList(value: unknown, label: string): string[] {
  asDenseArray(value, label);
  const values = asUniqueStrings(value, label);
  if (values.length > 32) throw new TypeError(`${label} must contain at most 32 entries`);
  return values.map((item, index) => parsePromptText(item, `${label}[${index}]`, 1_000));
}

function parsePromptText(value: unknown, label: string, maximumBytes: number): string {
  const text = asNonEmptyString(value, label);
  if (text.includes('\0') || text.includes('\r') || Buffer.byteLength(text, 'utf8') > maximumBytes) {
    throw new TypeError(`${label} is not bounded normalized text`);
  }
  return text;
}

function compareUtf8(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}
