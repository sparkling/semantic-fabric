// SPDX-License-Identifier: MIT

import { PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY } from './programme-v5-system.js';
import {
  createTaskQeCollectorForPackage,
  type TaskQeCollector,
  type TaskQeCollectorOptions,
  type TaskQeContext,
} from './task-qe.js';

export { PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY } from './programme-v5-system.js';

export type ProgrammeV5QeContext = TaskQeContext;

export function createProgrammeV5TaskQeCollector(
  options: Readonly<TaskQeCollectorOptions>,
): TaskQeCollector {
  return createTaskQeCollectorForPackage(
    options,
    PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY,
  );
}
