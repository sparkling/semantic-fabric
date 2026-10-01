// SPDX-License-Identifier: MIT
import type { NativeProcessRequest, NativeProcessRunner } from './types.js';
import { processSucceeded } from './native-adapter-contracts.js';
import { NativeCancellationError } from './recovery.js';

export const CODEX_MCP_CONFIGURATION_INVALID = 'HARNESS_CODEX_MCP_CONFIGURATION_INVALID';
export class CodexMcpInventory {
  readonly #entries = new Map<string, {
    names?: readonly string[];
    pending: Map<AbortSignal | undefined, Promise<readonly string[]>>;
  }>();

  async read(runner: NativeProcessRunner, request: NativeProcessRequest): Promise<readonly string[]> {
    if (request.signal?.aborted) throw new NativeCancellationError();
    const key = JSON.stringify([request.executable, request.cwd,
      request.env.HOME, request.env.CODEX_HOME, request.env.XDG_CONFIG_HOME]);
    let entry = this.#entries.get(key);
    if (!entry) { entry = { pending: new Map() }; this.#entries.set(key, entry); }
    if (entry.names) return entry.names;
    const existing = entry.pending.get(request.signal);
    if (existing) return await existing;
    const selected = entry;
    const pending = this.#discover(runner, request).then(names => {
      selected.names = names;
      return names;
    }).finally(() => {
      selected.pending.delete(request.signal);
      if (!selected.names && selected.pending.size === 0) this.#entries.delete(key);
    });
    selected.pending.set(request.signal, pending);
    return await pending;
  }

  async #discover(runner: NativeProcessRunner, request: NativeProcessRequest): Promise<readonly string[]> {
    try {
      const result = await runner.run(request);
      if (result.spawnError === 'check-process-group-unconfirmed') throw new Error('HARNESS_CODEX_MCP_CLEANUP_UNCONFIRMED');
      if (request.signal?.aborted || result.cancelled) throw new NativeCancellationError();
      if (!processSucceeded(result)) throw new Error();
      const rows: unknown = JSON.parse(result.stdout);
      if (!Array.isArray(rows)) throw new Error();
      const names = rows.map((row: unknown) => {
        if (!row || typeof row !== 'object' || Array.isArray(row)) throw new Error();
        const server = row as Record<string, unknown>;
        if (typeof server.name !== 'string' || !/^[A-Za-z0-9_-]+$/u.test(server.name)
          || typeof server.enabled !== 'boolean') throw new Error();
        return server.name;
      });
      return Object.freeze([...new Set(names)].sort());
    } catch (error) {
      // Inventory includes private transport values; never expose output or causes.
      if (error instanceof Error && ['HARNESS_CODEX_MCP_CLEANUP_UNCONFIRMED',
        'HARNESS_NATIVE_RESOURCE_TERMINATION_FAILED', 'HARNESS_NATIVE_EXECUTION_AND_CLEANUP_FAILED',
        'HARNESS_NATIVE_BOUNDARY_SETUP_AND_CLEANUP_FAILED'].includes(error.message)) throw new Error(error.message);
      if (request.signal?.aborted || error instanceof NativeCancellationError) throw new NativeCancellationError();
      throw new Error(CODEX_MCP_CONFIGURATION_INVALID);
    }
  }
}

const inventories = new WeakMap<NativeProcessRunner, CodexMcpInventory>();
export function codexMcpInventoryFor(runner: NativeProcessRunner): CodexMcpInventory {
  let inventory = inventories.get(runner);
  if (!inventory) { inventory = new CodexMcpInventory(); inventories.set(runner, inventory); }
  return inventory;
}
