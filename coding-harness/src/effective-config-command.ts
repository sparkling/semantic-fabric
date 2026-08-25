// SPDX-License-Identifier: MIT

import { APPROVED_NPM_REGISTRY, asNonEmptyString, asRecord } from './contracts.js';
import type {
  ConfigurationFinding,
  ConfigurationScope,
  ConfiguredServer,
  ParsedConfigurationSurface,
} from './effective-config.js';

type AddFinding = (finding: ConfigurationFinding) => void;

const EXACT_NPM_SELECTOR = /^(?:@[a-z0-9._-]+\/[a-z0-9._-]+|[a-z0-9._-]+)@\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;
const FORBIDDEN_TRANSPORT = /(?:openrouter|requesty)/i;
const SENSITIVE_ENVIRONMENT = /(?:api[_-]?key|token|secret|password|openrouter|requesty)/i;
const TRANSPORT_FIELD = /(?:base[_-]?url|api[_-]?base|proxy|gateway|provider|registry)/i;
const SHELL_EXECUTABLE = /^(?:ba|c|da|fi|k|tc|z)?sh(?:\.exe)?$/i;
const SHELL_META = /[;&|`$<>\r\n\0]/;

export function inspectSurfaceText(surface: ParsedConfigurationSurface, add: AddFinding): void {
  if (FORBIDDEN_TRANSPORT.test(surface.content)) {
    finding(add, 'FORBIDDEN_MODEL_TRANSPORT', surface, 'surface references a forbidden model transport');
  }
}

export function inspectStructuredConfiguration(
  parsed: Record<string, unknown>,
  surface: ParsedConfigurationSurface,
  surfaces: readonly ParsedConfigurationSurface[],
  add: AddFinding,
): ConfiguredServer[] {
  inspectEnvironmentTree(parsed, surface, add);
  const servers = parsed.mcpServers === undefined
    ? [] : extractServers(asRecord(parsed.mcpServers, `${surface.path}.mcpServers`), surface, surfaces, add);
  if (surface.kind === 'claude-settings') inspectHookCommands(parsed.hooks, surface, surfaces, add);
  return servers;
}

function extractServers(
  declared: Record<string, unknown>,
  surface: ParsedConfigurationSurface,
  surfaces: readonly ParsedConfigurationSurface[],
  add: AddFinding,
): ConfiguredServer[] {
  return Object.entries(declared).map(([name, raw]) => {
    if (name.trim() === '') throw new TypeError(`${surface.path} has an empty MCP server name`);
    const server = asRecord(raw, `${surface.path}.mcpServers.${name}`);
    const command = asNonEmptyString(server.command, `${surface.path}.mcpServers.${name}.command`);
    const args = strings(server.args ?? [], `${surface.path}.mcpServers.${name}.args`);
    const classification = classify(command, args, surface, surfaces, add);
    return {
      scope: surface.scope,
      name,
      sourcePath: surface.path,
      command: classification.command,
      packageSelector: classification.packageSelector,
      packagePinned: classification.packagePinned,
      localTarget: classification.localTarget,
    };
  });
}

function classify(
  rawCommand: string,
  args: string[],
  surface: ParsedConfigurationSurface,
  surfaces: readonly ParsedConfigurationSurface[],
  add: AddFinding,
): Pick<ConfiguredServer, 'command' | 'packageSelector' | 'packagePinned' | 'localTarget'> {
  const command = executableName(rawCommand);
  if (rawCommand !== command && rawCommand.toLowerCase() !== `${command}.exe`) {
    finding(add, 'UNTRUSTED_EXECUTABLE_PATH', surface, 'launcher must use an admitted bare executable name');
  }
  if (command === 'npx') {
    const packageSelector = npxSelector(args, surface, add);
    const packagePinned = packageSelector !== null && EXACT_NPM_SELECTOR.test(packageSelector);
    if (!packagePinned) finding(add, 'FLOATING_NPX_SELECTOR', surface, 'npx launcher is not pinned exactly');
    inspectRegistryArguments(args, surface, add);
    return { command, packageSelector, packagePinned, localTarget: null };
  }
  if (command === 'node') {
    const localTarget = normalizeTarget(args[0], surface.scope);
    inspectLocalTarget(localTarget, surface, surfaces, add);
    return { command, packageSelector: null, packagePinned: null, localTarget };
  }
  finding(
    add,
    SHELL_EXECUTABLE.test(command) ? 'SHELL_COMMAND_SURFACE' : 'UNSUPPORTED_EXECUTABLE',
    surface,
    'launcher executable is not admitted',
  );
  return { command, packageSelector: null, packagePinned: null, localTarget: null };
}

function inspectHookCommands(
  hooks: unknown,
  source: ParsedConfigurationSurface,
  surfaces: readonly ParsedConfigurationSurface[],
  add: AddFinding,
): void {
  for (const raw of commandStrings(hooks)) {
    if (SHELL_META.test(raw)) {
      finding(add, 'SHELL_COMMAND_SURFACE', source, 'hook uses shell syntax or indirection');
    }
    const direct = raw.trim().match(/^([^\s"']+)(?:\s+(.*))?$/s);
    const executable = executableName(direct?.[1] ?? '');
    if (SHELL_EXECUTABLE.test(executable)) {
      finding(add, 'SHELL_COMMAND_SURFACE', source, 'hook delegates through a shell');
    } else if (executable === 'node') {
      const target = normalizeTarget(firstToken(direct?.[2]), source.scope);
      inspectLocalTarget(target, source, surfaces, add);
    } else if (executable === 'npx') {
      npxSelector(tokens(direct?.[2] ?? ''), source, add);
    } else {
      finding(add, 'UNSUPPORTED_EXECUTABLE', source, 'hook executable is not admitted');
    }
  }
}

function inspectEnvironmentTree(
  value: unknown,
  surface: ParsedConfigurationSurface,
  add: AddFinding,
): void {
  if (value === null || typeof value !== 'object') return;
  if (Array.isArray(value)) {
    value.forEach((entry) => inspectEnvironmentTree(entry, surface, add));
    return;
  }
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    if (TRANSPORT_FIELD.test(key) && typeof child === 'string'
      && !admittedTransportValue(key, child)) {
      finding(add, 'UNTRUSTED_TRANSPORT_CONFIGURATION', surface, 'configuration declares an untrusted transport field');
    }
    if (key === 'env' || key === 'environment') {
      const environment = asRecord(child, `${surface.path}.${key}`);
      for (const [name, raw] of Object.entries(environment)) {
        if (SENSITIVE_ENVIRONMENT.test(name)) {
          finding(add, 'SENSITIVE_SERVER_ENVIRONMENT', surface, 'configuration declares a sensitive environment field');
        }
        if (typeof raw !== 'string' || FORBIDDEN_TRANSPORT.test(raw)
          || (TRANSPORT_FIELD.test(name) && !admittedTransportValue(name, raw))) {
          finding(add, 'UNTRUSTED_SERVER_ENVIRONMENT', surface, 'configuration declares an untrusted transport environment');
        }
      }
    }
    inspectEnvironmentTree(child, surface, add);
  }
}

function admittedTransportValue(name: string, value: string): boolean {
  if (!TRANSPORT_FIELD.test(name)) return true;
  if (/update_notifier/i.test(name)) return value === 'false';
  if (/registry/i.test(name)) return value === APPROVED_NPM_REGISTRY || value === new URL(APPROVED_NPM_REGISTRY).origin;
  return value === '';
}

function npxSelector(args: string[], surface: ParsedConfigurationSurface, add: AddFinding): string | null {
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === '-y' || argument === '--yes') continue;
    if (argument === '--package' || argument === '-p') return args[index + 1] ?? null;
    if (argument.startsWith('--package=')) return argument.slice('--package='.length) || null;
    if (argument.startsWith('-')) {
      finding(add, 'UNSUPPORTED_NPX_ARGUMENT', surface, 'npx uses an unadmitted pre-selector argument');
      continue;
    }
    return argument;
  }
  return null;
}

function inspectRegistryArguments(args: string[], surface: ParsedConfigurationSurface, add: AddFinding): void {
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index]!;
    if (!value.startsWith('--registry')) continue;
    const registry = value.includes('=') ? value.slice(value.indexOf('=') + 1) : args[index + 1];
    if (registry !== APPROVED_NPM_REGISTRY && registry !== new URL(APPROVED_NPM_REGISTRY).origin) {
      finding(add, 'UNTRUSTED_NPM_REGISTRY', surface, 'npx overrides the approved registry');
    }
  }
}

function inspectLocalTarget(
  target: string | null,
  source: ParsedConfigurationSurface,
  surfaces: readonly ParsedConfigurationSurface[],
  add: AddFinding,
): void {
  const attestation = target === null ? undefined
    : surfaces.find(({ path }) => path === target && path !== source.path);
  if (attestation?.provenance !== 'tracked-clean') {
    add({
      code: 'LOCAL_TARGET_NOT_TRACKED_CLEAN', severity: 'high', scope: source.scope,
      paths: target === null ? [source.path] : [source.path, target],
      message: 'local target is missing, ambiguous, or not tracked-clean',
    });
  }
}

function normalizeTarget(value: string | undefined, scope: ConfigurationScope): string | null {
  if (!value || value.includes('\0') || value.includes('\\') || value.startsWith('/')
    || value === '..' || value.startsWith('../') || value.includes('/../')
    || value.includes('$') || value.includes('{')) return null;
  const normalized = value.replace(/^\.\//, '');
  if (normalized === '' || normalized.split('/').some((part) => part === '' || part === '.')) return null;
  if (scope === 'coding-harness' && !normalized.startsWith('coding-harness/')) {
    return `coding-harness/${normalized}`;
  }
  return normalized;
}

function commandStrings(value: unknown): string[] {
  if (value === null || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(commandStrings);
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) =>
    key === 'command' && typeof child === 'string' ? [child] : commandStrings(child));
}

function tokens(value: string): string[] {
  if (SHELL_META.test(value)) return [];
  return value.trim().split(/\s+/).filter(Boolean).map((entry) => entry.replace(/^["']|["']$/g, ''));
}

function firstToken(value: string | undefined): string | undefined {
  return value === undefined ? undefined : tokens(value)[0];
}

function strings(value: unknown, label: string): string[] {
  if (!Array.isArray(value)) throw new TypeError(`${label} must be an array`);
  return value.map((entry, index) => asNonEmptyString(entry, `${label}[${index}]`));
}

function executableName(command: string): string {
  return command.replaceAll('\\', '/').split('/').at(-1)?.replace(/\.(?:cmd|exe)$/i, '').toLowerCase() ?? '';
}

function finding(add: AddFinding, code: string, surface: ParsedConfigurationSurface, message: string): void {
  add({ code, severity: 'high', scope: surface.scope, paths: [surface.path], message });
}
