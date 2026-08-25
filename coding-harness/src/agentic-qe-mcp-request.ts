// SPDX-License-Identifier: MIT

import { lstatSync, realpathSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';
import {
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  AGENTIC_QE_LCOV_GAPS_TOOL,
  type ProviderFreeAgenticQeMcpRequest,
} from './agentic-qe-lcov.js';
import { AGENTIC_QE_MAX_MCP_OUTPUT_BYTES } from './agentic-qe-lcov-response.js';

const GIT_OBJECT = /^[a-f0-9]{40}(?:[a-f0-9]{24})?$/;
const OPAQUE_ID = /^[A-Za-z0-9_-]{8,128}$/;

export function validateProviderFreeMcpRequest(
  value: unknown,
): ProviderFreeAgenticQeMcpRequest {
  const request = asRecord(value, 'Agentic-QE MCP runner request');
  assertExactKeys(request, [
    'executable', 'transport', 'method', 'toolName', 'arguments', 'bindings', 'runtime',
  ], 'Agentic-QE MCP runner request');
  if (request.executable !== 'aqe-mcp'
    || request.transport !== 'stdio-mcp'
    || request.method !== 'tools/call'
    || request.toolName !== AGENTIC_QE_LCOV_GAPS_TOOL) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_REQUEST_ROUTE_INVALID');
  }
  const argumentsValue = parseArguments(request.arguments);
  const bindings = parseBindings(request.bindings);
  const runtime = parseRuntime(request.runtime, argumentsValue.target, argumentsValue.coverageFile);
  return deepFreeze({
    executable: 'aqe-mcp',
    transport: 'stdio-mcp',
    method: 'tools/call',
    toolName: AGENTIC_QE_LCOV_GAPS_TOOL,
    arguments: argumentsValue,
    bindings,
    runtime,
  });
}

function parseArguments(value: unknown): ProviderFreeAgenticQeMcpRequest['arguments'] {
  const input = asRecord(value, 'Agentic-QE MCP arguments');
  assertExactKeys(input, [
    'target', 'coverageFile', 'coverageFormat', 'language', 'minRisk', 'limit',
    'prioritization', 'includeGhost',
  ], 'Agentic-QE MCP arguments');
  const target = canonicalDirectory(input.target, 'HARNESS_AGENTIC_QE_MCP_TARGET_INVALID');
  const coverageFile = canonicalRegularFile(
    input.coverageFile,
    'HARNESS_AGENTIC_QE_MCP_COVERAGE_FILE_INVALID',
  );
  if (input.coverageFormat !== 'lcov' || input.language !== 'rust'
    || input.minRisk !== 0 || input.limit !== 20
    || input.prioritization !== 'complexity' || input.includeGhost !== false) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_ARGUMENT_PROFILE_INVALID');
  }
  return {
    target,
    coverageFile,
    coverageFormat: 'lcov',
    language: 'rust',
    minRisk: 0,
    limit: 20,
    prioritization: 'complexity',
    includeGhost: false,
  };
}

function parseBindings(value: unknown): ProviderFreeAgenticQeMcpRequest['bindings'] {
  const input = asRecord(value, 'Agentic-QE MCP bindings');
  assertExactKeys(input, [
    'taskId', 'runId', 'candidateTree', 'lcovSha256', 'coverageCommandDigest',
    'generatorVersion',
  ], 'Agentic-QE MCP bindings');
  const taskId = opaqueId(input.taskId, 'taskId');
  const runId = opaqueId(input.runId, 'runId');
  if (typeof input.candidateTree !== 'string' || !GIT_OBJECT.test(input.candidateTree)) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_CANDIDATE_TREE_INVALID');
  }
  if (typeof input.lcovSha256 !== 'string' || !SHA256_PATTERN.test(input.lcovSha256)
    || typeof input.coverageCommandDigest !== 'string'
    || !SHA256_PATTERN.test(input.coverageCommandDigest)
    || input.generatorVersion !== 'cargo-llvm-cov 0.8.7') {
    throw new Error('HARNESS_AGENTIC_QE_MCP_COVERAGE_BINDING_INVALID');
  }
  return {
    taskId,
    runId,
    candidateTree: input.candidateTree,
    lcovSha256: input.lcovSha256,
    coverageCommandDigest: input.coverageCommandDigest,
    generatorVersion: 'cargo-llvm-cov 0.8.7',
  };
}

function parseRuntime(
  value: unknown,
  target: string,
  coverageFile: string,
): ProviderFreeAgenticQeMcpRequest['runtime'] {
  const input = asRecord(value, 'Agentic-QE MCP runtime');
  assertExactKeys(
    input,
    ['network', 'environment', 'filesystem', 'timeoutMs', 'maxOutputBytes'],
    'Agentic-QE MCP runtime',
  );
  if (input.network !== 'offline' || input.timeoutMs !== 120_000
    || input.maxOutputBytes !== AGENTIC_QE_MAX_MCP_OUTPUT_BYTES) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_RUNTIME_PROFILE_INVALID');
  }
  const environment = asRecord(input.environment, 'Agentic-QE MCP environment');
  assertExactKeys(environment, ['inheritance', 'variables'], 'Agentic-QE MCP environment');
  if (environment.inheritance !== 'none') {
    throw new Error('HARNESS_AGENTIC_QE_MCP_ENVIRONMENT_INHERITANCE_INVALID');
  }
  const variables = asRecord(environment.variables, 'Agentic-QE MCP variables');
  assertExactKeys(variables, [
    'AQE_MEMORY_BACKEND', 'AQE_LLM_ROUTER_DISABLED', 'AQE_SESSION_CACHE',
    'AQE_LOOP_DETECTION_ENABLED',
  ], 'Agentic-QE MCP variables');
  if (variables.AQE_MEMORY_BACKEND !== 'memory'
    || variables.AQE_LLM_ROUTER_DISABLED !== '1'
    || variables.AQE_SESSION_CACHE !== 'off'
    || variables.AQE_LOOP_DETECTION_ENABLED !== 'false') {
    throw new Error('HARNESS_AGENTIC_QE_MCP_VARIABLES_INVALID');
  }
  const filesystem = asRecord(input.filesystem, 'Agentic-QE MCP filesystem');
  assertExactKeys(filesystem, [
    'inputAccess', 'readOnlyPaths', 'privateHome', 'privateWritableTmp',
  ], 'Agentic-QE MCP filesystem');
  const expectedPaths = [...new Set([target, coverageFile])].sort();
  if (filesystem.inputAccess !== 'read-only'
    || filesystem.privateHome !== true || filesystem.privateWritableTmp !== true
    || !Array.isArray(filesystem.readOnlyPaths)
    || filesystem.readOnlyPaths.length !== expectedPaths.length
    || filesystem.readOnlyPaths.some((path, index) => path !== expectedPaths[index])) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_FILESYSTEM_PROFILE_INVALID');
  }
  return {
    network: 'offline',
    environment: {
      inheritance: 'none',
      variables: {
        AQE_MEMORY_BACKEND: 'memory',
        AQE_LLM_ROUTER_DISABLED: '1',
        AQE_SESSION_CACHE: 'off',
        AQE_LOOP_DETECTION_ENABLED: 'false',
      },
    },
    filesystem: {
      inputAccess: 'read-only',
      readOnlyPaths: expectedPaths,
      privateHome: true,
      privateWritableTmp: true,
    },
    timeoutMs: 120_000,
    maxOutputBytes: AGENTIC_QE_MAX_MCP_OUTPUT_BYTES,
  };
}

function canonicalDirectory(value: unknown, error: string): string {
  const path = canonical(value, error);
  const stat = lstatSync(path);
  if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error(error);
  return path;
}

function canonicalRegularFile(value: unknown, error: string): string {
  const path = canonical(value, error);
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) throw new Error(error);
  return path;
}

function canonical(value: unknown, error: string): string {
  if (typeof value !== 'string' || !isAbsolute(value) || resolve(value) !== value
    || value.includes('\0') || realpathSync(value) !== value) {
    throw new Error(error);
  }
  return value;
}

function opaqueId(value: unknown, label: string): string {
  const id = asNonEmptyString(value, `Agentic-QE MCP ${label}`);
  if (!OPAQUE_ID.test(id)) throw new Error(`HARNESS_AGENTIC_QE_MCP_${label.toUpperCase()}_INVALID`);
  return id;
}
