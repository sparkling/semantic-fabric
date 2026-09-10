// SPDX-License-Identifier: MIT
import { configDefaults, defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    clearMocks: true,
    // Native/packed-controller fixtures contend for the same host resources.
    // Limit test-file concurrency, not model/subscription requests or acceptance scope.
    maxWorkers: 2,
    exclude: [...configDefaults.exclude, 'supervisor-service/**'],
    restoreMocks: true,
  },
});
