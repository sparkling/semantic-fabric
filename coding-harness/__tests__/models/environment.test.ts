// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  assertNativeSubscriptionEnvironment,
  buildNativeSubscriptionEnvironment,
} from '../../src/models/environment.js';

describe('native subscription environment', () => {
  const source = {
    HOME: '/home/tester',
    PATH: '/usr/bin',
    LANG: 'C.UTF-8',
    CODEX_HOME: '/home/tester/.codex',
    OPENAI_API_KEY: 'must-not-cross',
    ANTHROPIC_API_KEY: 'must-not-cross',
    OPENROUTER_API_KEY: 'must-not-cross',
    OPENAI_BASE_URL: 'https://gateway.invalid',
    ANTHROPIC_BASE_URL: 'https://gateway.invalid',
    HTTP_PROXY: 'http://proxy.invalid',
    HTTPS_PROXY: 'http://proxy.invalid',
    ALL_PROXY: 'socks://proxy.invalid',
    NO_PROXY: '*',
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '0',
  } as const;

  it('copies only the Codex allow-list and strips keys, base URLs, and proxies', () => {
    const environment = buildNativeSubscriptionEnvironment('codex', source);

    expect(environment).toEqual({
      HOME: '/home/tester',
      PATH: '/usr/bin',
      LANG: 'C.UTF-8',
      CODEX_HOME: '/home/tester/.codex',
    });
    expect(Object.isFrozen(environment)).toBe(true);
    expect(() => assertNativeSubscriptionEnvironment('codex', environment)).not.toThrow();
  });

  it('does not pass Codex-specific state to Claude Code', () => {
    const environment = buildNativeSubscriptionEnvironment('claude-code', source);

    expect(environment).toEqual({
      HOME: '/home/tester',
      PATH: '/usr/bin',
      LANG: 'C.UTF-8',
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    });
  });

  it('requires the harness-managed Claude essential-traffic control', () => {
    expect(() => assertNativeSubscriptionEnvironment('claude-code', {
      HOME: '/home/tester',
    })).toThrow('HARNESS_NATIVE_ESSENTIAL_TRAFFIC_REQUIRED:claude-code');
    expect(() => assertNativeSubscriptionEnvironment('claude-code', {
      HOME: '/home/tester',
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '0',
    })).toThrow('HARNESS_NATIVE_ESSENTIAL_TRAFFIC_REQUIRED:claude-code');
  });

  it('fails closed if a caller constructs a non-allow-listed environment', () => {
    expect(() =>
      assertNativeSubscriptionEnvironment('codex', {
        HOME: '/home/tester',
        OPENAI_API_KEY: 'unexpected',
      }),
    ).toThrow('HARNESS_NATIVE_ENVIRONMENT_FORBIDDEN:OPENAI_API_KEY');
  });
});
