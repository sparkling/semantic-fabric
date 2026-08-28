// SPDX-License-Identifier: MIT

import { SECURE_HARNESS_CONFIG } from './config.js';
import { parseHarnessConfig } from './contracts.js';
import {
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
} from './programme-capture-task-v1.js';

export const PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1 = Object.freeze([
  ...new Set([
    PROGRAMME_CAPTURE_PROFILE_PATH,
    PROGRAMME_CAPTURE_SCENARIOS_PATH,
    'Cargo.lock',
    ...PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  ]),
].sort(compareUtf8));

export const PROGRAMME_CAPTURE_HARNESS_CONFIG_V1 = parseHarnessConfig({
  ...structuredClone(SECURE_HARNESS_CONFIG),
  requiredProtectedPaths: [...PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1],
});

function compareUtf8(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}
