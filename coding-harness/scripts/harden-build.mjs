// SPDX-License-Identifier: MIT

import { chmodSync, lstatSync, realpathSync } from 'node:fs';
import { resolve } from 'node:path';

const launcher = resolve('dist/native-proxy-launcher.js');
const stat = lstatSync(launcher);
if (!stat.isFile() || stat.isSymbolicLink() || realpathSync(launcher) !== launcher || stat.nlink !== 1) {
  throw new Error('HARNESS_BUILD_LAUNCHER_INVALID');
}
chmodSync(launcher, 0o644);
