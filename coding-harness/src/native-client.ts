// SPDX-License-Identifier: MIT

import { createHash, randomUUID } from 'node:crypto';
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
  readSync,
  realpathSync,
  statSync,
  truncateSync,
  writeFileSync,
} from 'node:fs';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import { deepFreeze } from './contracts.js';
import type {
  ModelOperation,
  NativeStructuredClient,
  NativeStructuredInvocation,
} from './model-controller.js';
import {
  NATIVE_PATCH_MAX_CHARS,
  NATIVE_REVIEW_MAX_REASONS,
  NATIVE_REVIEW_REASON_MAX_CHARS,
} from './model-controller.js';
import {
  ClaudeCodeSubscriptionAdapter,
  CodexSubscriptionAdapter,
} from './models/native-adapters.js';
import type { NativeRuntimeLedger } from './native-runtime-ledger.js';

type NativeAdapter = CodexSubscriptionAdapter | ClaudeCodeSubscriptionAdapter;

export interface NativeAdapterStructuredClientOptions {
  readonly adapter: NativeAdapter;
  readonly evidenceRoot: string;
  readonly workspaceRoot: string | (() => string);
  readonly timeoutMs: number;
  readonly maxOutputBytes?: number;
  readonly runtimeLedger?: NativeRuntimeLedger;
}

const DEFAULT_MAX_OUTPUT_BYTES = 10_000_000;

export class NativeAdapterStructuredClient implements NativeStructuredClient {
  readonly #adapter: NativeAdapter;
  readonly #evidenceRoot: string;
  readonly #workspaceRoot: () => string;
  readonly #timeoutMs: number;
  readonly #maxOutputBytes: number;
  readonly #runtimeLedger?: NativeRuntimeLedger;

  constructor(options: NativeAdapterStructuredClientOptions) {
    this.#adapter = options.adapter;
    this.#evidenceRoot = validatePrivateDirectory(options.evidenceRoot, 'EVIDENCE_ROOT', true);
    this.#workspaceRoot = typeof options.workspaceRoot === 'function'
      ? options.workspaceRoot
      : () => options.workspaceRoot as string;
    this.#timeoutMs = validateLimit(options.timeoutMs, 1_800_000, 'timeoutMs');
    this.#maxOutputBytes = validateLimit(
      options.maxOutputBytes ?? DEFAULT_MAX_OUTPUT_BYTES,
      DEFAULT_MAX_OUTPUT_BYTES,
      'maxOutputBytes',
    );
    this.#runtimeLedger = options.runtimeLedger;
  }

  async invoke(input: Readonly<{
    candidate: { readonly host: 'codex' | 'claude-code'; readonly model: string };
    operation: ModelOperation;
    prompt: string;
    signal?: AbortSignal;
  }>): Promise<NativeStructuredInvocation> {
    if (input.candidate.host !== this.#adapter.host) {
      throw new Error('HARNESS_NATIVE_CLIENT_HOST_MISMATCH');
    }
    const workspaceRoot = validatePrivateDirectory(
      this.#workspaceRoot(),
      'WORKSPACE_ROOT',
      false,
    );
    if (contains(workspaceRoot, this.#evidenceRoot)
      || contains(this.#evidenceRoot, workspaceRoot)) {
      throw new Error('HARNESS_NATIVE_EVIDENCE_WORKSPACE_OVERLAP');
    }
    const invocationRoot = join(this.#evidenceRoot, randomUUID());
    mkdirSync(invocationRoot, { mode: 0o700 });
    const schema = responseSchema(input.operation);
    const schemaPath = join(invocationRoot, 'response.schema.json');
    const outputPath = join(invocationRoot, 'response.json');
    createExclusiveFile(schemaPath, `${JSON.stringify(schema)}\n`);
    createExclusiveFile(outputPath, '');

    let raw: string;
    let executionId: string;
    if (this.#adapter.host === 'codex') {
      const result = await this.#adapter.invoke({
        cwd: workspaceRoot,
        model: input.candidate.model,
        prompt: input.prompt,
        schema,
        schemaPath,
        outputPath,
        workspaceAccess: 'read',
        timeoutMs: this.#timeoutMs,
        signal: input.signal,
        operation: input.operation,
      });
      executionId = result.executionId;
      raw = readEvidenceFile(outputPath, this.#maxOutputBytes);
    } else {
      const result = await this.#adapter.invoke({
        cwd: workspaceRoot,
        model: input.candidate.model,
        prompt: input.prompt,
        schema,
        workspaceAccess: 'read',
        timeoutMs: this.#timeoutMs,
        signal: input.signal,
        operation: input.operation,
      });
      executionId = result.executionId;
      raw = boundedOutput(result.stdout, this.#maxOutputBytes);
      writeEvidenceFile(outputPath, raw);
    }
    const output = this.#adapter.host === 'claude-code'
      ? parseClaudeEnvelope(raw)
      : parseJson(raw, 'Codex structured output');
    const outputDigest = createHash('sha256').update(raw, 'utf8').digest('hex');
    this.#runtimeLedger?.recordInvocation({
      invocationId: executionId,
      host: input.candidate.host,
      model: input.candidate.model,
      operation: input.operation,
      outputDigest,
    });
    return deepFreeze({
      invocationId: executionId,
      output,
      outputDigest,
    });
  }
}

function responseSchema(operation: ModelOperation): Readonly<Record<string, unknown>> {
  const common = { type: 'object', additionalProperties: false } as const;
  if (operation === 'architecture') {
    return deepFreeze({
      ...common,
      properties: {
        proposal: {
          type: 'object',
          additionalProperties: false,
          properties: {
            summary: { type: 'string', minLength: 1, maxLength: 2_000 },
            invariants: {
              type: 'array',
              minItems: 1,
              maxItems: 8,
              items: { type: 'string', minLength: 1, maxLength: 400 },
            },
            steps: {
              type: 'array',
              minItems: 1,
              maxItems: 8,
              items: { type: 'string', minLength: 1, maxLength: 400 },
            },
          },
          required: ['summary', 'invariants', 'steps'],
        },
        confidence: { type: 'number', minimum: 0, maximum: 1 },
      },
      required: ['proposal', 'confidence'],
    });
  }
  if (operation === 'implementation' || operation === 'repair') {
    return deepFreeze({
      ...common,
      properties: {
        patch: { type: 'string', minLength: 1, maxLength: NATIVE_PATCH_MAX_CHARS },
      },
      required: ['patch'],
    });
  }
  return deepFreeze({
    ...common,
    properties: {
      accepted: { type: 'boolean' },
      reasons: {
        type: 'array',
        maxItems: NATIVE_REVIEW_MAX_REASONS,
        items: {
          type: 'string', minLength: 1, maxLength: NATIVE_REVIEW_REASON_MAX_CHARS,
        },
      },
    },
    required: ['accepted', 'reasons'],
  });
}

function createExclusiveFile(path: string, content: string): void {
  const descriptor = openSync(path, 'wx', 0o600);
  try {
    writeFileSync(descriptor, content, { encoding: 'utf8' });
  } finally {
    closeSync(descriptor);
  }
}

function readEvidenceFile(path: string, limit: number): string {
  validateEvidenceFile(path);
  const descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const size = fstatSync(descriptor).size;
    if (size < 1 || size > limit) throw new Error('HARNESS_NATIVE_STRUCTURED_OUTPUT_INVALID');
    const buffer = Buffer.allocUnsafe(size);
    let offset = 0;
    while (offset < size) {
      const count = readSync(descriptor, buffer, offset, size - offset, offset);
      if (count === 0) break;
      offset += count;
    }
    if (offset !== size || fstatSync(descriptor).size !== size) {
      throw new Error('HARNESS_NATIVE_EVIDENCE_FILE_CHANGED');
    }
    return boundedOutput(buffer.toString('utf8'), limit);
  } finally {
    closeSync(descriptor);
  }
}

function writeEvidenceFile(path: string, content: string): void {
  validateEvidenceFile(path);
  truncateSync(path, 0);
  writeFileSync(path, content, { encoding: 'utf8', flag: 'r+' });
  validateEvidenceFile(path);
}

function validateEvidenceFile(path: string): void {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || realpathSync(path) !== path) {
    throw new Error('HARNESS_NATIVE_EVIDENCE_FILE_INVALID');
  }
}

function boundedOutput(value: string, limit: number): string {
  if (value.trim().length === 0 || Buffer.byteLength(value, 'utf8') > limit) {
    throw new Error('HARNESS_NATIVE_STRUCTURED_OUTPUT_INVALID');
  }
  return value;
}

function parseClaudeEnvelope(raw: string): unknown {
  const envelope = parseJson(raw, 'Claude structured output');
  if (!isRecord(envelope)) return envelope;
  if ('structured_output' in envelope) return envelope.structured_output;
  if ('structuredOutput' in envelope) return envelope.structuredOutput;
  if ('result' in envelope) {
    return typeof envelope.result === 'string'
      ? parseJson(envelope.result, 'Claude result')
      : envelope.result;
  }
  return envelope;
}

function parseJson(raw: string, label: string): unknown {
  try {
    return JSON.parse(raw) as unknown;
  } catch (error) {
    throw new Error(`HARNESS_NATIVE_JSON_INVALID:${label}`, { cause: error });
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function validatePrivateDirectory(path: string, label: string, requirePrivate: boolean): string {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  const stat = statSync(path);
  if (!stat.isDirectory() || realpathSync(path) !== path || (requirePrivate && (stat.mode & 0o077) !== 0)) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  return path;
}

function contains(root: string, child: string): boolean {
  const delta = relative(root, child);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

function validateLimit(value: number, ceiling: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 1 || value > ceiling) {
    throw new TypeError(`${label} must be a safe integer within the supported ceiling`);
  }
  return value;
}
