// SPDX-License-Identifier: MIT

import {
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import type { ProviderFreeAgenticQeMcpRequest } from './agentic-qe-lcov.js';

export const AGENTIC_QE_MCP_PROTOCOL_VERSION = '2025-11-25' as const;
export const AGENTIC_QE_MCP_INITIALIZE_ID = 1 as const;
export const AGENTIC_QE_MCP_TOOL_CALL_ID = 2 as const;
export const AGENTIC_QE_MCP_SHUTDOWN_ID = 3 as const;

const CLIENT_NAME = 'semantic-fabric-coding-harness';
const CLIENT_VERSION = '1';

export function initializeMessage(): string {
  return jsonLine({
    jsonrpc: '2.0',
    id: AGENTIC_QE_MCP_INITIALIZE_ID,
    method: 'initialize',
    params: {
      protocolVersion: AGENTIC_QE_MCP_PROTOCOL_VERSION,
      capabilities: {},
      clientInfo: { name: CLIENT_NAME, version: CLIENT_VERSION },
    },
  });
}

export function initializedAndToolMessages(
  request: ProviderFreeAgenticQeMcpRequest,
): string {
  return [
    {
      jsonrpc: '2.0',
      method: 'initialized',
      params: {},
    },
    {
      jsonrpc: '2.0',
      id: AGENTIC_QE_MCP_TOOL_CALL_ID,
      method: 'tools/call',
      params: {
        name: request.toolName,
        arguments: request.arguments,
      },
    },
  ].map(jsonLine).join('');
}

export function shutdownMessage(): string {
  return jsonLine({
    jsonrpc: '2.0',
    id: AGENTIC_QE_MCP_SHUTDOWN_ID,
    method: 'shutdown',
    params: {},
  });
}

export function parseRpcResponse(line: string, expectedId: 1 | 2 | 3): unknown {
  if (Buffer.byteLength(line, 'utf8') > 5_000_000 || line.includes('\0')) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_RESPONSE_LINE_INVALID');
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(line);
  } catch {
    throw new Error('HARNESS_AGENTIC_QE_MCP_RESPONSE_JSON_INVALID');
  }
  const response = asRecord(parsed, 'Agentic-QE JSON-RPC response');
  if (response.jsonrpc !== '2.0' || response.id !== expectedId) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_RESPONSE_ID_INVALID');
  }
  const hasResult = Object.prototype.hasOwnProperty.call(response, 'result');
  const hasError = Object.prototype.hasOwnProperty.call(response, 'error');
  if (hasResult === hasError) throw new Error('HARNESS_AGENTIC_QE_MCP_RESPONSE_SHAPE_INVALID');
  if (hasError) {
    assertExactKeys(response, ['jsonrpc', 'id', 'error'], 'Agentic-QE JSON-RPC error');
    throw rpcError(response.error, expectedId);
  }
  assertExactKeys(response, ['jsonrpc', 'id', 'result'], 'Agentic-QE JSON-RPC result');
  return deepFreeze(response.result);
}

export function validateInitializeResult(value: unknown, packageVersion: string): void {
  const result = asRecord(value, 'Agentic-QE initialize result');
  assertExactKeys(
    result,
    ['protocolVersion', 'capabilities', 'serverInfo'],
    'Agentic-QE initialize result',
  );
  if (result.protocolVersion !== AGENTIC_QE_MCP_PROTOCOL_VERSION) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_PROTOCOL_NEGOTIATION_INVALID');
  }
  const capabilities = asRecord(result.capabilities, 'Agentic-QE capabilities');
  assertExactKeys(capabilities, ['tools', 'logging'], 'Agentic-QE capabilities');
  const tools = asRecord(capabilities.tools, 'Agentic-QE tool capabilities');
  assertExactKeys(tools, ['listChanged'], 'Agentic-QE tool capabilities');
  if (tools.listChanged !== true) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_TOOL_CAPABILITY_INVALID');
  }
  const logging = asRecord(capabilities.logging, 'Agentic-QE logging capabilities');
  assertExactKeys(logging, [], 'Agentic-QE logging capabilities');
  const serverInfo = asRecord(result.serverInfo, 'Agentic-QE serverInfo');
  assertExactKeys(
    serverInfo,
    ['name', 'version', 'protocolVersion'],
    'Agentic-QE serverInfo',
  );
  if (serverInfo.name !== 'agentic-qe-v3'
    || serverInfo.version !== packageVersion
    || serverInfo.protocolVersion !== AGENTIC_QE_MCP_PROTOCOL_VERSION) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_SERVER_IDENTITY_INVALID');
  }
}

export function validateShutdownResult(value: unknown): void {
  const result = asRecord(value, 'Agentic-QE shutdown result');
  assertExactKeys(result, [], 'Agentic-QE shutdown result');
}

function rpcError(value: unknown, id: number): Error {
  const error = asRecord(value, 'Agentic-QE JSON-RPC error payload');
  const keys = Object.keys(error);
  if (!keys.includes('code') || !keys.includes('message')
    || keys.some((key) => !['code', 'message', 'data'].includes(key))
    || !Number.isSafeInteger(error.code)) {
    return new Error('HARNESS_AGENTIC_QE_MCP_RPC_ERROR_INVALID');
  }
  const message = asNonEmptyString(error.message, 'Agentic-QE JSON-RPC error message');
  if (Buffer.byteLength(message, 'utf8') > 4096 || message.includes('\0')) {
    return new Error('HARNESS_AGENTIC_QE_MCP_RPC_ERROR_INVALID');
  }
  return new Error(`HARNESS_AGENTIC_QE_MCP_RPC_ERROR:${id}:${String(error.code)}:${message}`);
}

function jsonLine(value: unknown): string {
  return `${JSON.stringify(value)}\n`;
}
