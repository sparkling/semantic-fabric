// SPDX-License-Identifier: MIT

import type { NativeHost } from './types.js';

const COMMON_ALLOWLIST = Object.freeze([
  'PATH',
  'HOME',
  'USER',
  'LOGNAME',
  'SHELL',
  'TMPDIR',
  'TMP',
  'TEMP',
  'LANG',
  'LC_ALL',
  'LC_CTYPE',
  'TERM',
  'COLORTERM',
  'NO_COLOR',
  'XDG_CONFIG_HOME',
  'XDG_CACHE_HOME',
  'XDG_DATA_HOME',
] as const);

export const NATIVE_SUBSCRIPTION_ENV_ALLOWLIST = Object.freeze({
  codex: Object.freeze([...COMMON_ALLOWLIST, 'CODEX_HOME']),
  'claude-code': Object.freeze([
    ...COMMON_ALLOWLIST,
    'CLAUDE_CONFIG_DIR',
    'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC',
  ]),
} as const);

const NATIVE_MANAGED_ENVIRONMENT = Object.freeze({
  codex: Object.freeze({}),
  'claude-code': Object.freeze({
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
  }),
} as const);

export function buildNativeSubscriptionEnvironment(
  host: NativeHost,
  source: Readonly<Record<string, string | undefined>>,
): Readonly<Record<string, string>> {
  const environment: Record<string, string> = {};
  for (const name of NATIVE_SUBSCRIPTION_ENV_ALLOWLIST[host]) {
    const value = source[name];
    if (value !== undefined && value.length > 0) environment[name] = value;
  }
  Object.assign(environment, NATIVE_MANAGED_ENVIRONMENT[host]);
  assertNativeSubscriptionEnvironment(host, environment);
  return Object.freeze(environment);
}

export function assertNativeSubscriptionEnvironment(
  host: NativeHost,
  environment: Readonly<Record<string, string>>,
): void {
  const allowed = new Set<string>(NATIVE_SUBSCRIPTION_ENV_ALLOWLIST[host]);
  for (const [name, value] of Object.entries(environment)) {
    if (isSensitiveTransportName(name) || !allowed.has(name)) {
      throw new Error(`HARNESS_NATIVE_ENVIRONMENT_FORBIDDEN:${name}`);
    }
    if (typeof value !== 'string' || value.includes('\0')) {
      throw new Error(`HARNESS_NATIVE_ENVIRONMENT_INVALID:${name}`);
    }
  }
  if (host === 'claude-code'
    && environment.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC !== '1') {
    throw new Error('HARNESS_NATIVE_ESSENTIAL_TRAFFIC_REQUIRED:claude-code');
  }
}

function isSensitiveTransportName(name: string): boolean {
  const normalized = name.toUpperCase();
  return (
    normalized.includes('OPENROUTER') ||
    normalized.includes('REQUESTY') ||
    normalized.includes('PROXY') ||
    normalized.includes('BASE_URL') ||
    normalized.includes('ENDPOINT') ||
    normalized.endsWith('_API_KEY') ||
    normalized.endsWith('_TOKEN') ||
    normalized.endsWith('_SECRET') ||
    normalized.endsWith('_CREDENTIALS')
  );
}
