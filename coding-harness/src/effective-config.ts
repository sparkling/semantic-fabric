// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
  normalizeWorkspacePath,
  SHA256_PATTERN,
} from './contracts.js';
import {
  inspectStructuredConfiguration,
  inspectSurfaceText,
} from './effective-config-command.js';

export type ConfigurationScope = 'repository' | 'coding-harness';
export type SurfaceKind =
  | 'mcp-json'
  | 'claude-settings'
  | 'codex-config'
  | 'agent-config'
  | 'hook-target'
  | 'skill'
  | 'instruction'
  | 'executable-target'
  | 'harness-policy';
export type SurfaceProvenance = 'tracked-clean' | 'tracked-dirty' | 'untracked' | 'ignored';
export type AuditSeverity = 'info' | 'low' | 'medium' | 'high';
export type UpstreamDiagnosticTool = 'mcp-scan' | 'threat-model';

export interface AuditedSurface {
  scope: ConfigurationScope;
  path: string;
  kind: SurfaceKind;
  provenance: SurfaceProvenance;
  digest: string;
}

export interface ParsedConfigurationSurface extends AuditedSurface {
  content: string;
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

export interface ConfigurationRepositorySnapshot {
  repositoryRoot: string;
  headCommit: string;
  indexTree: string;
  gitExecutableDigest: string;
  digest: string;
}

export interface UpstreamDiagnostic {
  target: ConfigurationScope;
  tool: UpstreamDiagnosticTool;
  mcpEnabled: boolean;
  worstSeverity: AuditSeverity;
  verdict: 'clean' | 'findings' | 'inconclusive';
  toolVersion: string;
  rawDigest: string;
  invocationId: string;
  exitCode: number;
  degraded: false;
}

export interface EffectiveConfigurationAudit {
  status: 'PASS' | 'FAIL' | 'INCONCLUSIVE';
  complete: boolean;
  repositorySnapshot: ConfigurationRepositorySnapshot | null;
  surfaces: AuditedSurface[];
  servers: ConfiguredServer[];
  upstreamDiagnostics: UpstreamDiagnostic[];
  findings: ConfigurationFinding[];
}

interface ParsedScope {
  target: ConfigurationScope;
  inventoryComplete: boolean;
  surfaces: ParsedConfigurationSurface[];
  upstreamDiagnostics: UpstreamDiagnostic[];
}

const REQUIRED_SCOPES: ConfigurationScope[] = ['repository', 'coding-harness'];
const REQUIRED_DIAGNOSTICS: UpstreamDiagnosticTool[] = ['mcp-scan', 'threat-model'];
const JSON_SURFACES = new Set<SurfaceKind>([
  'mcp-json', 'claude-settings', 'harness-policy',
]);
const MUTABLE_EXECUTION_SURFACES = new Set<SurfaceKind>([
  'mcp-json', 'claude-settings', 'codex-config', 'agent-config', 'hook-target',
  'skill', 'instruction', 'executable-target', 'harness-policy',
]);
const EVOLUTION_COMMAND = /(?:metaharness(?:-darwin)?\s+evolve|\/evolve\b|darwin\s+evolve)/i;
const SEVERITY_RANK: Record<AuditSeverity, number> = { info: 0, low: 1, medium: 2, high: 3 };

export function auditEffectiveConfiguration(value: unknown): EffectiveConfigurationAudit {
  const scopes = parseInput(value);
  const findings: ConfigurationFinding[] = [];
  const surfaces = scopes.flatMap(({ surfaces: entries }) => entries);
  const servers: ConfiguredServer[] = [];
  const inconclusiveScopes = new Set<ConfigurationScope>();
  const add = (finding: ConfigurationFinding) => addFinding(findings, finding);

  for (const surface of surfaces) {
    if (surface.provenance !== 'tracked-clean' && MUTABLE_EXECUTION_SURFACES.has(surface.kind)) {
      add({
        code: 'MUTABLE_EXECUTION_SURFACE', severity: 'high', scope: surface.scope,
        paths: [surface.path], message: `${surface.kind} is ${surface.provenance}`,
      });
    }
    if (surface.kind === 'skill' && EVOLUTION_COMMAND.test(surface.content)) {
      add({
        code: 'EVOLUTION_COMMAND_SURFACE', severity: 'high', scope: surface.scope,
        paths: [surface.path], message: 'an executable evolution instruction is present',
      });
    }
    inspectSurfaceText(surface, add);
    if (!JSON_SURFACES.has(surface.kind)) continue;
    try {
      const parsed = asRecord(JSON.parse(surface.content) as unknown, surface.path);
      servers.push(...inspectStructuredConfiguration(parsed, surface, surfaces, add));
    } catch {
      inconclusiveScopes.add(surface.scope);
      add({
        code: 'CONFIG_PARSE_FAILURE', severity: 'high', scope: surface.scope,
        paths: [surface.path], message: 'declared configuration could not be parsed completely',
      });
    }
  }

  findServerCollisions(servers, findings);
  reconcileDiagnostics(scopes, servers, findings, inconclusiveScopes);
  const complete = inconclusiveScopes.size === 0;
  const blocking = findings.some(({ severity }) => SEVERITY_RANK[severity] >= SEVERITY_RANK.medium);
  return deepFreeze({
    status: !complete ? 'INCONCLUSIVE' : blocking ? 'FAIL' : 'PASS',
    complete,
    repositorySnapshot: null,
    surfaces: surfaces.map(({ content: _content, ...surface }) => surface),
    servers,
    upstreamDiagnostics: scopes.flatMap(({ upstreamDiagnostics }) => upstreamDiagnostics),
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
  if (targets.length !== REQUIRED_SCOPES.length || new Set(targets).size !== targets.length
    || REQUIRED_SCOPES.some((target) => !targets.includes(target))) {
    throw new TypeError('effective configuration audit scopes must include every target exactly once');
  }
  const byTarget = new Map(scopes.map((scope) => [scope.target, scope]));
  return REQUIRED_SCOPES.map((target) => byTarget.get(target) as ParsedScope);
}

function parseScope(value: unknown, index: number): ParsedScope {
  const label = `effective configuration audit.scopes[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, ['target', 'inventoryComplete', 'surfaces', 'upstreamDiagnostics'], label);
  const target = parseScopeName(input.target, `${label}.target`);
  if (typeof input.inventoryComplete !== 'boolean') throw new TypeError(`${label}.inventoryComplete must be a boolean`);
  if (!Array.isArray(input.surfaces)) throw new TypeError(`${label}.surfaces must be an array`);
  const surfaces = input.surfaces.map((entry, surfaceIndex) => parseSurface(entry, target, surfaceIndex));
  const keys = surfaces.map(({ path, kind }) => `${path}\0${kind}`);
  if (new Set(keys).size !== keys.length) throw new TypeError(`${label}.surfaces contains duplicate path roles`);
  if (!Array.isArray(input.upstreamDiagnostics)) throw new TypeError(`${label}.upstreamDiagnostics must be an array`);
  const upstreamDiagnostics = input.upstreamDiagnostics.map((entry, diagnosticIndex) =>
    parseDiagnostic(entry, `${label}.upstreamDiagnostics[${diagnosticIndex}]`));
  const tools = upstreamDiagnostics.map(({ tool }) => tool);
  if (tools.length !== REQUIRED_DIAGNOSTICS.length || new Set(tools).size !== tools.length
    || REQUIRED_DIAGNOSTICS.some((tool) => !tools.includes(tool))
    || upstreamDiagnostics.some((diagnostic) => diagnostic.target !== target)) {
    throw new TypeError(`${label} must contain both matching upstream diagnostics exactly once`);
  }
  return { target, inventoryComplete: input.inventoryComplete, surfaces, upstreamDiagnostics };
}

function parseSurface(value: unknown, scope: ConfigurationScope, index: number): ParsedConfigurationSurface {
  const label = `effective configuration ${scope}.surfaces[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, ['path', 'kind', 'provenance', 'content'], label);
  const path = normalizeWorkspacePath(input.path, `${label}.path`);
  const kinds: SurfaceKind[] = [
    'mcp-json', 'claude-settings', 'codex-config', 'agent-config', 'hook-target',
    'skill', 'instruction', 'executable-target', 'harness-policy',
  ];
  if (typeof input.kind !== 'string' || !kinds.includes(input.kind as SurfaceKind)) {
    throw new TypeError(`${label}.kind is invalid`);
  }
  const provenances: SurfaceProvenance[] = ['tracked-clean', 'tracked-dirty', 'untracked', 'ignored'];
  if (typeof input.provenance !== 'string' || !provenances.includes(input.provenance as SurfaceProvenance)) {
    throw new TypeError(`${label}.provenance is invalid`);
  }
  if (typeof input.content !== 'string' || input.content.includes('\0')) {
    throw new TypeError(`${label}.content must be text without NUL bytes`);
  }
  return {
    scope,
    path,
    kind: input.kind as SurfaceKind,
    provenance: input.provenance as SurfaceProvenance,
    content: input.content,
    digest: createHash('sha256').update(input.content).digest('hex'),
  };
}

function parseDiagnostic(value: unknown, label: string): UpstreamDiagnostic {
  const input = asRecord(value, label);
  assertExactKeys(input, [
    'target', 'tool', 'mcpEnabled', 'worstSeverity', 'verdict', 'toolVersion',
    'rawDigest', 'invocationId', 'exitCode', 'degraded',
  ], label);
  const target = parseScopeName(input.target, `${label}.target`);
  if (input.tool !== 'mcp-scan' && input.tool !== 'threat-model') throw new TypeError(`${label}.tool is invalid`);
  if (typeof input.mcpEnabled !== 'boolean') throw new TypeError(`${label}.mcpEnabled must be a boolean`);
  const severities: AuditSeverity[] = ['info', 'low', 'medium', 'high'];
  if (typeof input.worstSeverity !== 'string' || !severities.includes(input.worstSeverity as AuditSeverity)) {
    throw new TypeError(`${label}.worstSeverity is invalid`);
  }
  const verdicts = ['clean', 'findings', 'inconclusive'] as const;
  if (typeof input.verdict !== 'string' || !verdicts.includes(input.verdict as typeof verdicts[number])) {
    throw new TypeError(`${label}.verdict is invalid`);
  }
  const rawDigest = digest(input.rawDigest, `${label}.rawDigest`);
  const invocationId = digest(input.invocationId, `${label}.invocationId`);
  if (!Number.isSafeInteger(input.exitCode) || (input.exitCode as number) < 0 || (input.exitCode as number) > 255) {
    throw new TypeError(`${label}.exitCode is invalid`);
  }
  if (input.degraded !== false) throw new TypeError(`${label}.degraded must be false`);
  return {
    target,
    tool: input.tool,
    mcpEnabled: input.mcpEnabled,
    worstSeverity: input.worstSeverity as AuditSeverity,
    verdict: input.verdict as UpstreamDiagnostic['verdict'],
    toolVersion: asNonEmptyString(input.toolVersion, `${label}.toolVersion`),
    rawDigest,
    invocationId,
    exitCode: input.exitCode as number,
    degraded: false,
  };
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
    if (!scope.inventoryComplete) {
      inconclusiveScopes.add(scope.target);
      addFinding(findings, {
        code: 'INCOMPLETE_SURFACE_INVENTORY', severity: 'high', scope: scope.target,
        paths: [], message: 'effective configuration inventory is incomplete',
      });
    }
    const mcpScan = scope.upstreamDiagnostics.find(({ tool }) => tool === 'mcp-scan')!;
    if (servers.some(({ scope: target }) => target === scope.target) && !mcpScan.mcpEnabled) {
      inconclusiveScopes.add(scope.target);
      addFinding(findings, {
        code: 'UPSTREAM_BLIND_TO_MCP_SURFACE', severity: 'high', scope: scope.target,
        paths: scope.surfaces.filter(({ kind }) => JSON_SURFACES.has(kind)).map(({ path }) => path),
        message: 'upstream diagnostic disabled MCP despite declared servers',
      });
    }
    if (new Set(scope.upstreamDiagnostics.map(({ mcpEnabled }) => mcpEnabled)).size !== 1) {
      inconclusiveScopes.add(scope.target);
      addFinding(findings, {
        code: 'UPSTREAM_MCP_VISIBILITY_CONFLICT', severity: 'high', scope: scope.target,
        paths: [], message: 'upstream diagnostics disagree about MCP visibility',
      });
    }
    for (const diagnostic of scope.upstreamDiagnostics) {
      if (diagnostic.verdict === 'findings') {
        addFinding(findings, {
          code: 'UPSTREAM_DIAGNOSTIC_FINDINGS', severity: diagnostic.worstSeverity,
          scope: scope.target, paths: [], message: `${diagnostic.tool} reported findings`,
        });
      } else if (diagnostic.verdict === 'inconclusive') {
        inconclusiveScopes.add(scope.target);
        addFinding(findings, {
          code: 'UPSTREAM_DIAGNOSTIC_INCONCLUSIVE', severity: 'high',
          scope: scope.target, paths: [], message: `${diagnostic.tool} did not reach a conclusion`,
        });
      } else if (SEVERITY_RANK[diagnostic.worstSeverity] >= SEVERITY_RANK.medium) {
        inconclusiveScopes.add(scope.target);
        addFinding(findings, {
          code: 'UPSTREAM_VERDICT_CONFLICT', severity: 'high', scope: scope.target,
          paths: [], message: `${diagnostic.tool} clean verdict contradicts its worst severity`,
        });
      }
    }
  }
}

function parseScopeName(value: unknown, label: string): ConfigurationScope {
  if (value !== 'repository' && value !== 'coding-harness') throw new TypeError(`${label} is invalid`);
  return value;
}

function digest(value: unknown, label: string): string {
  const result = asNonEmptyString(value, label);
  if (!SHA256_PATTERN.test(result) || result === '0'.repeat(64)) throw new TypeError(`${label} is invalid`);
  return result;
}

function addFinding(findings: ConfigurationFinding[], finding: ConfigurationFinding): void {
  if (!findings.some(({ code, scope, paths }) => code === finding.code
    && scope === finding.scope && paths.join('\0') === finding.paths.join('\0'))) {
    findings.push(finding);
  }
}
