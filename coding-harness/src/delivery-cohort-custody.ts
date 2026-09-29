// SPDX-License-Identifier: MIT
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { atomicJson, processIdentity, readJson } from './delivery-workspace.js';
import type { DeliveryHarness } from './delivery-runtime.js';
import { liveDeliveryRuntimeInputs, requiredDeliveryInputs } from './delivery-lineage.js';

/** privateSnapshot: callback executes only in a private candidate snapshot; absent means live canonical reads. */
export interface OutcomeReservation { id: string; mutationPaths: string[]; readPaths?: string[]; resources: string[]; privateSnapshot?: true; retainedDirectory?: string }
interface ReservationFile { outcomes: OutcomeReservation[] }
const overlaps = (a: string, b: string): boolean => a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
const intersects = (left: string[], right: string[]) => left.some(a => right.some(b => overlaps(a, b)));
// Snapshot evaluator inputs are re-pinned at the reader's own integration; only live runtime stays implicit here.
const reads = (reservation: OutcomeReservation, paths: string[]) => reservation.readPaths === undefined || intersects(reservation.readPaths, paths)
  || (reservation.privateSnapshot === true ? liveDeliveryRuntimeInputs(paths)
    : requiredDeliveryInputs(Object.fromEntries(paths.map(path => [path, ''])), [])).length > 0;

/** Caller holds canonical operation lock for admission and integration. Stale reservations fail closed. */
export function activeReservations(harness: DeliveryHarness): OutcomeReservation[] {
  return readdirSync(harness.directory).filter(name => name.startsWith('pool-')).flatMap(name => {
    const path = join(harness.directory, name, 'reservations.json');
    return existsSync(path) ? (readJson(path) as ReservationFile).outcomes.filter(row =>
      !row.retainedDirectory || candidateHasOperation(row.retainedDirectory)) : [];
  });
}
export function assertReservationAdmission(harness: DeliveryHarness, incoming: readonly OutcomeReservation[]): void {
  // Reads are private snapshot inputs; guard their canonical changes at integration.
  for (const existing of activeReservations(harness)) for (const next of incoming) {
    if (existing.id === next.id || intersects(existing.mutationPaths, next.mutationPaths)
      || existing.resources.some(resource => next.resources.includes(resource))) throw new Error('DELIVERY_POOL_RESOURCE_CONFLICT');
  }
}
export function assertIntegrationReservations(harness: DeliveryHarness, scope: string[], resources: string[] = []): void {
  for (const existing of activeReservations(harness)) {
    if (intersects(existing.mutationPaths, scope) || reads(existing, scope) || existing.resources.some(value => resources.includes(value))) {
      throw new Error('DELIVERY_INTEGRATION_ACTIVE_DEPENDENCY');
    }
  }
}
export function candidateHasOperation(directory: string): boolean {
  if (!existsSync(directory)) return true; // Missing custody evidence cannot prove child cleanup.
  return readdirSync(directory, { withFileTypes: true }).some(entry => entry.name === 'operation.lock'
    || (entry.isDirectory() && candidateHasOperation(join(directory, entry.name))));
}
export function saveReservations(directory: string, outcomes: OutcomeReservation[]): void {
  atomicJson(join(directory, 'reservations.json'), { pid: process.pid, start: processIdentity(process.pid), outcomes });
}
