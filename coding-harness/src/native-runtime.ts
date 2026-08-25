// SPDX-License-Identifier: MIT

import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import type { HarnessConfig } from './contracts.js';
import { NativeAdapterStructuredClient } from './native-client.js';
import { UnixSocketOriginPinningBoundary } from './native-egress.js';
import { BoundedNativeProcessRunner } from './native-process.js';
import { NativeRuntimeLedger } from './native-runtime-ledger.js';
import {
  SystemNativeFilesystemBoundary,
  type NativeRuntimeMount,
} from './native-system-filesystem.js';
import { SystemdResourceBoundary, type NativeResourceLimits } from './resource-boundary.js';
import {
  ClaudeCodeSubscriptionAdapter,
  CodexSubscriptionAdapter,
  preflightNativeSubscriptions,
} from './models/native-adapters.js';
import type { NativeHost } from './models/types.js';
import type { HostEvidence } from './receipts.js';

const MAX_CREDENTIAL_BYTES = 10 * 1024 * 1024;

export interface NativeRuntimeExecutablePaths {
  readonly codex: string;
  readonly claude: string;
  readonly node: string;
  readonly bwrap: string;
  readonly systemdRun: string;
  readonly proxyLauncher: string;
}

export interface NativeRuntimeFactoryOptions {
  readonly config: HarnessConfig;
  readonly runtimeParent: string;
  readonly allowedWorkspaceRoots: readonly string[];
  readonly workspaceRoot: string | (() => string);
  readonly executables: NativeRuntimeExecutablePaths;
  readonly credentials: Readonly<Record<NativeHost, string>>;
  readonly commonRuntimeMounts: readonly NativeRuntimeMount[];
  readonly resourceLimits: NativeResourceLimits;
  readonly maskedWorkspacePaths?: readonly string[];
  readonly forbiddenRoots?: readonly string[];
  readonly controllerEnvironment?: Readonly<Record<string, string | undefined>>;
  readonly timeoutMs?: number;
}

export interface NativeRuntimePreflight {
  readonly hosts: readonly [HostEvidence, HostEvidence];
  readonly codexVersion: string;
  readonly claudeVersion: string;
}

export interface TrustedNativeRuntime {
  readonly runtimeRoot: string;
  readonly evidenceRoot: string;
  readonly runner: BoundedNativeProcessRunner;
  readonly ledger: NativeRuntimeLedger;
  readonly adapters: Readonly<{
    codex: CodexSubscriptionAdapter;
    claude: ClaudeCodeSubscriptionAdapter;
  }>;
  readonly clients: Readonly<Record<NativeHost, NativeAdapterStructuredClient>>;
  preflight(models: Readonly<{ codex: string; claude: string }>, cwd: string): Promise<NativeRuntimePreflight>;
  cleanup(): void;
}

export function createTrustedNativeRuntime(
  options: NativeRuntimeFactoryOptions,
): TrustedNativeRuntime {
  const parent = privateDirectory(options.runtimeParent, 'RUNTIME_PARENT');
  if (options.allowedWorkspaceRoots.length === 0) {
    throw new Error('HARNESS_NATIVE_WORKSPACE_ROOTS_REQUIRED');
  }
  const runtimeRoot = mkdtempSync(join(parent, 'native-runtime-'));
  let cleaned = false;
  try {
    const brokerRoot = createPrivateDirectory(runtimeRoot, 'broker');
    const evidenceRoot = createPrivateDirectory(runtimeRoot, 'evidence');
    const authRoot = createPrivateDirectory(runtimeRoot, 'auth');
    const codexAuthRoot = createPrivateDirectory(authRoot, 'codex');
    const claudeAuthRoot = createPrivateDirectory(authRoot, 'claude');
    const codexAuth = join(codexAuthRoot, 'auth.json');
    const claudeAuth = join(claudeAuthRoot, '.credentials.json');
    copyCredential(options.credentials.codex, codexAuth, 'CODEX');
    copyCredential(options.credentials['claude-code'], claudeAuth, 'CLAUDE');

    const egress = new UnixSocketOriginPinningBoundary({
      brokerRoot,
      nodeExecutable: canonicalFile(options.executables.node, 'NODE'),
      launcherPath: canonicalFile(options.executables.proxyLauncher, 'PROXY_LAUNCHER'),
    });
    const filesystem = new SystemNativeFilesystemBoundary({
      bwrapExecutable: canonicalFile(options.executables.bwrap, 'BWRAP'),
      brokerRoot,
      commonRuntimeMounts: [
        ...options.commonRuntimeMounts,
        samePathMount(options.executables.node),
        samePathMount(options.executables.proxyLauncher),
      ],
      hosts: {
        codex: {
          authenticationMounts: [{ source: codexAuth, destination: '/home/harness/.codex/auth.json' }],
          runtimeMounts: [samePathMount(options.executables.codex)],
          privateEnvironment: {
            HOME: '/home/harness',
            CODEX_HOME: '/home/harness/.codex',
          },
        },
        'claude-code': {
          authenticationMounts: [{
            source: claudeAuth,
            destination: '/home/harness/.claude/.credentials.json',
          }],
          runtimeMounts: [samePathMount(options.executables.claude)],
          privateEnvironment: {
            HOME: '/home/harness',
            CLAUDE_CONFIG_DIR: '/home/harness/.claude',
          },
        },
      },
    });
    const resources = new SystemdResourceBoundary({
      executablePath: canonicalFile(options.executables.systemdRun, 'SYSTEMD_RUN'),
      sourceEnvironment: options.controllerEnvironment ?? process.env,
    });
    const forbidden = options.forbiddenRoots ?? unique([
      dirname(options.credentials.codex),
      dirname(options.credentials['claude-code']),
    ]);
    const runner = new BoundedNativeProcessRunner({
      config: options.config,
      executables: {
        codex: canonicalFile(options.executables.codex, 'CODEX'),
        'claude-code': canonicalFile(options.executables.claude, 'CLAUDE'),
      },
      allowedRoots: options.allowedWorkspaceRoots,
      allowedReadRoots: [...options.allowedWorkspaceRoots, evidenceRoot],
      allowedWriteRoots: [evidenceRoot],
      forbiddenRoots: forbidden,
      egressBoundary: egress,
      filesystemBoundary: filesystem,
      resourceBoundary: resources,
      resourceLimits: options.resourceLimits,
      maskedWorkspacePaths: options.maskedWorkspacePaths,
    });
    const ledger = new NativeRuntimeLedger(runner);
    const sourceEnvironment = Object.freeze({});
    const adapters = Object.freeze({
      codex: new CodexSubscriptionAdapter({
        executable: options.executables.codex,
        runner,
        sourceEnvironment,
        evidenceRoot,
      }),
      claude: new ClaudeCodeSubscriptionAdapter({
        executable: options.executables.claude,
        runner,
        sourceEnvironment,
      }),
    });
    const workspaceRoot = typeof options.workspaceRoot === 'function'
      ? options.workspaceRoot
      : () => options.workspaceRoot as string;
    const clients = Object.freeze({
      codex: new NativeAdapterStructuredClient({
        adapter: adapters.codex,
        evidenceRoot,
        workspaceRoot,
        timeoutMs: options.timeoutMs ?? options.config.limits.maxTimeoutMs,
        runtimeLedger: ledger,
      }),
      'claude-code': new NativeAdapterStructuredClient({
        adapter: adapters.claude,
        evidenceRoot,
        workspaceRoot,
        timeoutMs: options.timeoutMs ?? options.config.limits.maxTimeoutMs,
        runtimeLedger: ledger,
      }),
    });
    return Object.freeze({
      runtimeRoot,
      evidenceRoot,
      runner,
      ledger,
      adapters,
      clients,
      async preflight(
        models: Readonly<{ codex: string; claude: string }>,
        cwd: string,
      ) {
        const evidence = await preflightNativeSubscriptions({
          codex: adapters.codex,
          claude: adapters.claude,
          cwd,
          requestedModels: models,
        });
        for (const item of evidence) ledger.recordPreflight(item);
        const hosts = Object.freeze([
          hostEvidence(evidence[0], 'implementation-review'),
          hostEvidence(evidence[1], 'architecture-review'),
        ]) as readonly [HostEvidence, HostEvidence];
        return Object.freeze({
          hosts,
          codexVersion: evidence[0].clientVersion,
          claudeVersion: evidence[1].clientVersion,
        });
      },
      cleanup() {
        if (cleaned) return;
        cleaned = true;
        rmSync(runtimeRoot, { recursive: true, force: true });
      },
    });
  } catch (error) {
    rmSync(runtimeRoot, { recursive: true, force: true });
    throw error;
  }
}

function hostEvidence(
  evidence: Awaited<ReturnType<CodexSubscriptionAdapter['preflight']>>,
  role: string,
): HostEvidence {
  return Object.freeze({
    host: evidence.host,
    model: evidence.requestedModel,
    role,
    clientVersion: evidence.clientVersion,
    authClass: evidence.host === 'codex'
      ? 'native-openai-subscription'
      : 'native-anthropic-subscription',
    subscriptionCostUsd: 0,
  });
}

function copyCredential(source: string, destination: string, label: string): void {
  const path = canonicalFile(source, `${label}_CREDENTIAL`);
  const stat = lstatSync(path);
  const uid = process.getuid?.() ?? stat.uid;
  if (stat.uid !== uid || stat.nlink !== 1 || (stat.mode & 0o077) !== 0
    || stat.size < 1 || stat.size > MAX_CREDENTIAL_BYTES) {
    throw new Error(`HARNESS_NATIVE_${label}_CREDENTIAL_INVALID`);
  }
  const descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const opened = fstatSync(descriptor);
    if (opened.dev !== stat.dev || opened.ino !== stat.ino || opened.size !== stat.size) {
      throw new Error(`HARNESS_NATIVE_${label}_CREDENTIAL_CHANGED`);
    }
    const buffer = Buffer.allocUnsafe(opened.size);
    let offset = 0;
    while (offset < buffer.length) {
      const count = readSync(descriptor, buffer, offset, buffer.length - offset, offset);
      if (count === 0) break;
      offset += count;
    }
    const finished = fstatSync(descriptor);
    if (offset !== buffer.length || finished.dev !== opened.dev
      || finished.ino !== opened.ino || finished.size !== opened.size) {
      throw new Error(`HARNESS_NATIVE_${label}_CREDENTIAL_CHANGED`);
    }
    writeFileSync(destination, buffer, { flag: 'wx', mode: 0o600 });
  } finally {
    closeSync(descriptor);
  }
}

function createPrivateDirectory(parent: string, name: string): string {
  const path = join(parent, name);
  mkdirSync(path, { mode: 0o700 });
  return path;
}

function privateDirectory(path: string, label: string): string {
  const canonical = canonicalPath(path, label);
  const stat = lstatSync(canonical);
  const uid = process.getuid?.() ?? stat.uid;
  if (!stat.isDirectory() || stat.uid !== uid || (stat.mode & 0o077) !== 0) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  return canonical;
}

function canonicalFile(path: string, label: string): string {
  const canonical = canonicalPath(path, label);
  if (!lstatSync(canonical).isFile()) throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  return canonical;
}

function canonicalPath(path: string, label: string): string {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  const stat = lstatSync(path);
  if (stat.isSymbolicLink() || realpathSync(path) !== path) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  return path;
}

function samePathMount(path: string): NativeRuntimeMount {
  return Object.freeze({ source: path, destination: path });
}

function unique(values: readonly string[]): string[] {
  return [...new Set(values)];
}
