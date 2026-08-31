// SPDX-License-Identifier: MIT

import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type {
  ImmutablePrivateTreeOverrideManifestSpec,
} from './immutable-private-tree-overlay.js';

const MODULE_DIRECTORY = dirname(fileURLToPath(import.meta.url));

// This reconstructs the already-attested schema-v2 execution closure. It does
// not define a new package identity, provider authority, or fetch mechanism.
export const PROGRAMME_V5_RUFLO_SCHEMA_V2_OVERRIDE_MANIFEST:
Readonly<ImmutablePrivateTreeOverrideManifestSpec> = Object.freeze({
  sourcePath: join(
    MODULE_DIRECTORY,
    '..',
    'config',
    'programme-v5-ruflo-schema-v2-overlay.json',
  ),
  expectedDigest: '5f64f7205c58b1451f3fd3aefba902352da8fd23a638f1719222ef1851f3e558',
  expectedBytes: 924,
});
