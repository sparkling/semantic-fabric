// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
  normalizeWorkspacePath,
} from './contracts.js';

export type ConfigurationScope = 'repository' | 'coding-harness';
export type SurfaceKind =
  | 'mcp-json'
  | 'claude-settings'
  | 'hook-target'
  | 'skill'
  | 'executable-target';
export type SurfaceProvenance = 'tracked-clean' | 'tracked-dirty' | 'untracked' | 'ignored';
export type AuditSeverity = 'info' | 'low' | 'medium' | 'high';

export interface AuditedSurface {
  scope: ConfigurationScope;
  path: string;
  kind: SurfaceKind;
  provenance: SurfaceProvenance;
  digest: string;
}

export interface ConfiguredServer {
  scope: ConfigurationScope;
  name: string;
  sourcePath: string;
  command: string;
  packageSelector: string | null;
  packagePinned: boolean | null;
  localTarget: string | null;
}

export interface ConfigurationFinding {
  code: string;
  severity: AuditSeverity;
  scope: ConfigurationScope;
  paths: string[];
  message: string;
}

export interface EffectiveConfigurationAudit {
  status: 'PASS' | 'FAIL' | 'INCONCLUSIVE';
  complete: boolean;
  surfaces: AuditedSurface[];
  servers: ConfiguredServer[];
  findings: ConfigurationFinding[];
}

interface ParsedSurface extends AuditedSurface {
  content: string;
}

interface UpstreamDiagnostic {
  target: ConfigurationScope;
  mcpEnabled: boolean;
  worstSeverity: AuditSeverity;
  verdict: 'clean' | 'findings' | 'inconclusive';
}

interface ParsedScope {
  target: ConfigurationScope;
  inventoryComplete: boolean;
  surfaces: ParsedSurface[];
  upstreamDiagnostic: UpstreamDiagnostic;
}

const REQUIRED_SCOPES: ConfigurationScope[] = ['repository', 'coding-harness'];
const JSON_SURFACES = new Set<SurfaceKind>(['mcp-json', 'claude-settings']);
const MUTABLE_EXECUTION_SURFACES = new Set<SurfaceKind>([
  'mcp-json', 'claude-settings', 'hook-target', 'skill', 'executable-target',
]);
const EXACT_NPM_SELECTOR = /^(?:@[a-z0-9._-]+\/[a-z0-9._-]+|[a-z0-9._-]+)@\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;
const EVOLUTION_COMMAND = /(?:metaharness(?:-darwin)?\s+evolve|\/evolve\b|darwin\s+evolve)/i;
const FORBIDDEN_TRANSPORT = /(?:openrouter|requesty)/i;
const SENSITIVE_ENVIRONMENT = /(?:api[_-]?key|token|secret|password|openrouter|requesty)/i;
const SEVERITY_RANK: Record<AuditSeverity, number> = { info: 0, low: 1, medium: 2, high: 3 };

export function auditEffectiveConfiguration(value: unknown): EffectiveConfigurationAudit {
  const scopes = parseInput(value);
  const findings: ConfigurationFinding[] = [];
  const surfaces = scopes.flatMap(({ surfaces: entries }) => entries);
  const servers: ConfiguredServer[] = [];
  const inconclusiveScopes = new Set<ConfigurationScope>();

  for (const surface of surfaces) {
    if (surface.provenance !== 'tracked-clean' && MUTABLE_EXECUTION_SURFACES.has(surface.kind)) {
      addFinding(findings, {
        code: 'MUTABLE_EXECUTION_SURFACE', severity: 'high', scope: surface.scope,
        paths: [surface.path], message: `${surface.kind} is ${surface.provenance}`,
      });
    }
    if (surface.kind === 'skill' && EVOLUTION_COMMAND.test(surface.content)) {
      addFinding(findings, {
        code: 'EVOLUTION_COMMAND_SURFACE', severity: 'high', scope: surface.scope,
        paths: [surface.path], message: 'an executable evolution instruction is present',
      });
    }
    if (!JSON_SURFACES.has(surface.kind)) continue;
    try {
      const parsed = asRecord(JSON.parse(surface.content) as unknown, surface.path);
      servers.push(...extractServers(parsed, surface, findings, surfaces));
      if (surface.kind === 'claude-settings') {
        inspectHookCommands(parsed.hooks, surface, surfaces, findings);
      }
    } catch {
      inconclusiveScopes.add(surface.scope);
      addFinding(findings, {
        code: 'CONFIG_PARSE_FAILURE', severity: 'high', scope: surface.scope,
        paths: [surface.path], message: 'declared configuration could not be parsed completely',
      });
    }
  }

  findServerCollisions(servers, findings);
  reconcileDiagnostics(scopes, servers, findings, inconclusiveScopes);

  const complete = inconclusiveScopes.size === 0;
  const hasBlockingFinding = findings.some(({ severity }) => SEVERITY_RANK[severity] >= SEVERITY_RANK.medium);
  return deepFreeze({
    status: !complete ? 'INCONCLUSIVE' : hasBlockingFinding ? 'FAIL' : 'PASS',
    complete,
    surfaces: surfaces.map(({ content: _content, ...surface }) => surface),
    servers,
    findings,
  });
}

function parseInput(value: unknown): ParsedScope[] {
  const input = asRecord(value, 'effective configuration audit');
  assertExactKeys(input, ['schemaVersion', 'scopes'], 'effective configuration audit');
  if (input.schemaVersion !== 1) throw new TypeError('effective configuration audit.schemaVersion must be 1');
  if (!Array.isArray(input.scopes)) throw new TypeError('effective configuration audit.scopes must be an array');
  const scopes = input.scopes.map((entry, index) => parseScope(entry, index));
  const targets = scopes.map(({ target }) => target);
  if (targets.length !== REQUIRED_SCOPES.length
    || new Set(targets).size !== targets.length
    || REQUIRED_SCOPES.some((target) => !targets.includes(target))) {
    throw new TypeError('effective configuration audit scopes must include every target exactly once');
  }
  const byTarget = new Map(scopes.map((scope) => [scope.target, scope]));
  return REQUIRED_SCOPES.map((target) => byTarget.get(target) as ParsedScope);
}

function parseScope(value: unknown, index: number): ParsedScope {
  const label = `effective configuration audit.scopes[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, ['target', 'inventoryComplete', 'surfaces', 'upstreamDiagnostic'], label);
  const target = parseScopeName(input.target, `${label}.target`);
  if (typeof input.inventoryComplete !== 'boolean') {
    throw new TypeError(`${label}.inventoryComplete must be a boolean`);
  }
  if (!Array.isArray(input.surfaces)) throw new TypeError(`${label}.surfaces must be an array`);
  const surfaces = input.surfaces.map((entry, surfaceIndex) => parseSurface(entry, target, surfaceIndex));
  const paths = surfaces.map(({ path }) => path);
  if (new Set(paths).size !== paths.length) throw new TypeError(`${label}.surfaces contains duplicate paths`);
  const upstreamDiagnostic = parseDiagnostic(input.upstreamDiagnostic, `${label}.upstreamDiagnostic`);
  if (upstreamDiagnostic.target !== target) throw new TypeError(`${label} diagnostic target does not match scope`);
  return { target, inventoryComplete: input.inventoryComplete, surfaces, upstreamDiagnostic };
}

function parseSurface(value: unknown, scope: ConfigurationScope, index: number): ParsedSurface {
  const label = `effective configuration ${scope}.surfaces[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, ['path', 'kind', 'provenance', 'content'], label);
  const path = normalizeWorkspacePath(input.path, `${label}.path`);
  const kinds: SurfaceKind[] = ['mcp-json', 'claude-settings', 'hook-target', 'skill', 'executable-target'];
  if (typeof input.kind !== 'string' || !kinds.includes(input.kind as SurfaceKind)) {
    throw new TypeError(`${label}.kind is invalid`);
  }
  const provenances: SurfaceProvenance[] = ['tracked-clean', 'tracked-dirty', 'untracked', 'ignored'];
  if (typeof input.provenance !== 'string' || !provenances.includes(input.provenance as SurfaceProvenance)) {
    throw new TypeError(`${label}.provenance is invalid`);
  }
  const content = asNonEmptyString(input.content, `${label}.content`);
  return {
    scope,
    path,
    kind: input.kind as SurfaceKind,
    provenance: input.provenance as SurfaceProvenance,
    content,
    digest: createHash('sha256').update(content).digest('hex'),
  };
}

function parseDiagnostic(value: unknown, label: string): UpstreamDiagnostic {
  const input = asRecord(value, label);
  assertExactKeys(input, ['target', 'mcpEnabled', 'worstSeverity', 'verdict'], label);
  const target = parseScopeName(input.target, `${label}.target`);
  if (typeof input.mcpEnabled !== 'boolean') throw new TypeError(`${label}.mcpEnabled must be a boolean`);
  const severities: AuditSeverity[] = ['info', 'low', 'medium', 'high'];
  if (typeof input.worstSeverity !== 'string'
    || !severities.includes(input.worstSeverity as AuditSeverity)) {
    throw new TypeError(`${label}.worstSeverity is invalid`);
  }
  const verdicts = ['clean', 'findings', 'inconclusive'] as const;
  if (typeof input.verdict !== 'string'
    || !verdicts.includes(input.verdict as typeof verdicts[number])) {
    throw new TypeError(`${label}.verdict is invalid`);
  }
  return {
    target,
    mcpEnabled: input.mcpEnabled,
    worstSeverity: input.worstSeverity as AuditSeverity,
    verdict: input.verdict as UpstreamDiagnostic['verdict'],
  };
}

function extractServers(
  config: Record<string, unknown>,
  surface: ParsedSurface,
  findings: ConfigurationFinding[],
  surfaces: ParsedSurface[],
): ConfiguredServer[] {
  if (config.mcpServers === undefined) return [];
  const declared = asRecord(config.mcpServers, `${surface.path}.mcpServers`);
  return Object.entries(declared).map(([name, raw]) => {
    if (name.trim() === '') throw new TypeError(`${surface.path} has an empty MCP server name`);
    const server = asRecord(raw, `${surface.path}.mcpServers.${name}`);
    const command = asNonEmptyString(server.command, `${surface.path}.mcpServers.${name}.command`);
    const args = parseStringArray(server.args ?? [], `${surface.path}.mcpServers.${name}.args`);
    inspectForbiddenTransport(command, args, surface, findings);
    inspectSensitiveEnvironment(server.env, surface, findings);
    const executable = executableName(command);
    const packageSelector = executable === 'npx' ? findNpxSelector(args) : null;
    const packagePinned = executable === 'npx' ? Boolean(packageSelector && EXACT_NPM_SELECTOR.test(packageSelector)) : null;
    if (executable === 'npx' && !packagePinned) {
      addFinding(findings, {
        code: 'FLOATING_NPX_SELECTOR', severity: 'high', scope: surface.scope,
        paths: [surface.path], message: `MCP server ${name} does not use an exact npm version`,
      });
    }
    const localTarget = executable === 'node' ? normalizeTarget(args[0]) : null;
    if (executable === 'node') inspectLocalTarget(localTarget, surface, surfaces, findings);
    return {
      scope: surface.scope,
      name,
      sourcePath: surface.path,
      command: executable,
      packageSelector,
      packagePinned,
      localTarget,
    };
  });
}

function inspectHookCommands(
  hooks: unknown,
  source: ParsedSurface,
  surfaces: ParsedSurface[],
  findings: ConfigurationFinding[],
): void {
  for (const command of collectCommandStrings(hooks)) {
    inspectForbiddenTransport(command, [], source, findings);
    const npxMatch = command.match(/(?:^|\s)npx(?:\.cmd)?\s+(?:-y\s+|--yes\s+)?([^\s"']+)/i);
    if (npxMatch && !EXACT_NPM_SELECTOR.test(npxMatch[1] ?? '')) {
      addFinding(findings, {
        code: 'FLOATING_NPX_SELECTOR', severity: 'high', scope: source.scope,
        paths: [source.path], message: 'hook command does not use an exact npm version',
      });
    }
    const nodeMatch = command.match(/(?:^|\s)node(?:\.exe)?\s+["']?([^\s"']+)/i);
    if (nodeMatch) inspectLocalTarget(normalizeTarget(nodeMatch[1]), source, surfaces, findings);
  }
}

function collectCommandStrings(value: unknown): string[] {
  if (value === null || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(collectCommandStrings);
  const output: string[] = [];
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    if (key === 'command' && typeof child === 'string') output.push(child);
    else output.push(...collectCommandStrings(child));
  }
  return output;
}

function inspectLocalTarget(
  target: string | null,
  source: ParsedSurface,
  surfaces: ParsedSurface[],
  findings: ConfigurationFinding[],
): void {
  const attestation = target ? surfaces.find(({ path }) => path === target) : undefined;
  if (!attestation || attestation.provenance !== 'tracked-clean') {
    addFinding(findings, {
      code: 'LOCAL_TARGET_NOT_TRACKED_CLEAN', severity: 'high', scope: source.scope,
      paths: target ? [source.path, target] : [source.path],
      message: 'local Node target is missing, ambiguous, or not tracked-clean',
    });
  }
}

function inspectForbiddenTransport(
  command: string,
  args: string[],
  surface: ParsedSurface,
  findings: ConfigurationFinding[],
): void {
  if (!FORBIDDEN_TRANSPORT.test([command, ...args].join(' '))) return;
  addFinding(findings, {
    code: 'FORBIDDEN_MODEL_TRANSPORT', severity: 'high', scope: surface.scope,
    paths: [surface.path], message: 'configuration references a forbidden model transport',
  });
}

function inspectSensitiveEnvironment(
  value: unknown,
  surface: ParsedSurface,
  findings: ConfigurationFinding[],
): void {
  if (value === undefined) return;
  const environment = asRecord(value, `${surface.path}.server.env`);
  if (!Object.keys(environment).some((name) => SENSITIVE_ENVIRONMENT.test(name))) return;
  addFinding(findings, {
    code: 'SENSITIVE_SERVER_ENVIRONMENT', severity: 'high', scope: surface.scope,
    paths: [surface.path], message: 'MCP server declares a sensitive environment field',
  });
}

function findServerCollisions(servers: ConfiguredServer[], findings: ConfigurationFinding[]): void {
  const byName = new Map<string, ConfiguredServer[]>();
  for (const server of servers) byName.set(server.name, [...(byName.get(server.name) ?? []), server]);
  for (const [name, entries] of byName) {
    if (entries.length < 2) continue;
    addFinding(findings, {
      code: 'MCP_SERVER_NAME_COLLISION', severity: 'high', scope: entries[0]!.scope,
      paths: entries.map(({ sourcePath }) => sourcePath),
      message: `MCP server name ${name} is declared by multiple effective surfaces`,
    });
  }
}

function reconcileDiagnostics(
  scopes: ParsedScope[],
  servers: ConfiguredServer[],
  findings: ConfigurationFinding[],
  inconclusiveScopes: Set<ConfigurationScope>,
): void {
  for (const scope of scopes) {
    const diagnostic = scope.upstreamDiagnostic;
    if (!scope.inventoryComplete) {
      inconclusiveScopes.add(scope.target);
      addFinding(findings, {
        code: 'INCOMPLETE_SURFACE_INVENTORY', severity: 'high', scope: scope.target,
        paths: [], message: 'effective configuration inventory is incomplete',
      });
    }
    if (servers.some(({ scope: target }) => target === scope.target) && !diagnostic.mcpEnabled) {
      inconclusiveScopes.add(scope.target);
      addFinding(findings, {
        code: 'UPSTREAM_BLIND_TO_MCP_SURFACE', severity: 'high', scope: scope.target,
        paths: scope.surfaces.filter(({ kind }) => JSON_SURFACES.has(kind)).map(({ path }) => path),
        message: 'upstream diagnostic disabled MCP despite declared servers',
      });
    }
    if (diagnostic.verdict === 'inconclusive') {
      inconclusiveScopes.add(scope.target);
      addFinding(findings, {
        code: 'UPSTREAM_DIAGNOSTIC_INCONCLUSIVE', severity: 'high', scope: scope.target,
        paths: [], message: 'upstream diagnostic did not reach a conclusion',
      });
    }
    if (diagnostic.verdict === 'clean' && SEVERITY_RANK[diagnostic.worstSeverity] >= SEVERITY_RANK.medium) {
      inconclusiveScopes.add(scope.target);
      addFinding(findings, {
        code: 'UPSTREAM_VERDICT_CONFLICT', severity: 'high', scope: scope.target,
        paths: [], message: 'upstream clean verdict contradicts its worst severity',
      });
    }
  }
}

function parseScopeName(value: unknown, label: string): ConfigurationScope {
  if (value !== 'repository' && value !== 'coding-harness') throw new TypeError(`${label} is invalid`);
  return value;
}

function parseStringArray(value: unknown, label: string): string[] {
  if (!Array.isArray(value)) throw new TypeError(`${label} must be an array`);
  return value.map((entry, index) => asNonEmptyString(entry, `${label}[${index}]`));
}

function executableName(command: string): string {
  return command.replace(/\\/g, '/').split('/').at(-1)?.replace(/\.(?:cmd|exe)$/i, '').toLowerCase() ?? '';
}

function findNpxSelector(args: string[]): string | null {
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === '--package' || argument === '-p') return args[index + 1] ?? null;
    if (argument.startsWith('--package=')) return argument.slice('--package='.length) || null;
    if (!argument.startsWith('-')) return argument;
  }
  return null;
}

function normalizeTarget(value: string | undefined): string | null {
  if (!value) return null;
  try {
    return normalizeWorkspacePath(value, 'local target');
  } catch {
    return null;
  }
}

function addFinding(findings: ConfigurationFinding[], finding: ConfigurationFinding): void {
  if (!findings.some(({ code, scope, paths }) => code === finding.code
    && scope === finding.scope
    && paths.join('\0') === finding.paths.join('\0'))) {
    findings.push(finding);
  }
}
