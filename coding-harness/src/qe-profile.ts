// SPDX-License-Identifier: MIT

export const AGENTIC_QE_PROFILES = Object.freeze([
  'lcov-gap', 'rust-testgen-no-ai', 'quality-contract', 'sast',
] as const);

export type AgenticQeProfile = typeof AGENTIC_QE_PROFILES[number];
