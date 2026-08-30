// SPDX-License-Identifier: MIT
import { configDefaults, defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    clearMocks: true,
    exclude: [...configDefaults.exclude, 'supervisor-service/**'],
    restoreMocks: true,
  },
});
