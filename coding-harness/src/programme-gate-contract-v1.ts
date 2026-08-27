// SPDX-License-Identifier: MIT

import {
  DEVELOPMENT_AUTHORITY,
  asInteger,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';

const DIMENSIONS = Object.freeze([
  { id: 'policy-and-supply-chain-safety', maximumPoints: 20 },
  { id: 'evaluator-integrity', maximumPoints: 15 },
  { id: 'evolution-containment', maximumPoints: 5 },
  { id: 'patched-candidate-verification', maximumPoints: 20 },
  { id: 'dual-host-control-plane', maximumPoints: 15 },
  { id: 'reliability-and-receipts', maximumPoints: 15 },
  { id: 'ruflo-and-qe-integration', maximumPoints: 10 },
] as const);

const REQUIRED_TOOL_KEYS = Object.freeze([
  'bootstrapSource',
  'bootstrapControllerStoreDigest',
  'bootstrapBuildManifestDigest',
  'bootstrapRuntimeTreeDigest',
  'bootstrapNodeDigest',
  'bootstrapGitDigest',
  'controllerExecutionDigest',
  'controllerBuildManifestDigest',
  'controllerRuntimeTreeDigest',
  'controllerManifestDigest',
  'controllerTaskPath',
  'controllerTaskPathDigest',
  'controllerTaskDigest',
  'taskEvidencePlanDigest',
  'boundTaskDigest',
  'programmePolicyFingerprint',
  'frozenCargoLockDigest',
  'cargo',
  'cargoLlvmCov',
  'node',
  'codex',
  'claude',
  'codexExecutable',
  'claudeExecutable',
  'bwrap',
  'systemdRun',
  'systemctl',
  'caBundle',
  'agenticQeMcp',
  'agenticQe',
  'agenticQePackageTreeDigest',
  'rustToolchainClosure',
  'rustRegistryBootstrapSnapshot',
  'rustBootstrapClosureMetadata',
  'rustRegistryClosure',
  'rustRegistryLock',
  'rustRegistrySelection',
  'rustRegistryMetadata',
  'rufloHive',
  'rufloConsensus',
] as const);

export function createProgrammeGateContractV1(maxRepairs: number) {
  const repairs = asInteger(maxRepairs, 'programme policy v5 maxRepairs');
  if (repairs > 10) throw new TypeError('programme policy v5 maxRepairs cannot exceed 10');
  return deepFreeze({
    schemaVersion: 1 as const,
    authoritativeReplaySemantics: true as const,
    receiptSchemaVersion: 3 as const,
    taskSchemaVersion: 3 as const,
    acceptanceSchemaVersion: 2 as const,
    acceptance: {
      threshold: 98 as const,
      maximumScore: 100 as const,
      allDimensionsAreHardGates: true as const,
      diagnosticGateRequired: true as const,
      fitnessEligible: false as const,
      dimensions: DIMENSIONS.map((dimension) => ({ ...dimension })),
      resultBindings: {
        receiptDigest: 'receipt.digest' as const,
        dimensionOrder: 'exact-contract-dimension-order' as const,
        verifiedPoints: 'hard-gate-pass-then-maximum-else-zero' as const,
        hardGatePassed: 'recomputed-dimension-predicate' as const,
        evidenceDigests: 'single-digestValue-of-dimension-evidence' as const,
        score: 'sum-dimension-verifiedPoints' as const,
        failedDimensions: 'failed-contract-dimensions-in-order' as const,
        failedDiagnostics: 'failed-contract-diagnostics-in-order' as const,
        hardGatesPassed: 'failedDimensions-is-empty' as const,
        diagnosticGatePassed: 'failedDiagnostics-is-empty' as const,
        status: 'accepted-only-if-threshold-and-all-hard-and-diagnostic-gates-pass' as const,
      },
    },
    receipt: {
      chainSchemaVersion: 3 as const,
      chainCardinality: 1 as const,
      sequence: 0 as const,
      previousDigest: '0000000000000000000000000000000000000000000000000000000000000000',
      digestRule: 'digestValue(receipt-without-digest)' as const,
      step: 'candidate-transaction' as const,
      status: 'pass' as const,
      failureCode: null,
      authority: DEVELOPMENT_AUTHORITY,
      issuedAtRule: 'canonical-iso-timestamp' as const,
      repairCountRule: 'nonnegative-and-at-most-gate.maximumRepairs' as const,
      patchHistory: {
        cardinality: 'receipt.recovery.repairCount-plus-one' as const,
        finalDigest: 'equals-receipt.patchDigest' as const,
        everyDigest: 'non-genesis-lowercase-sha256' as const,
      },
      identityBindings: [
        ['receipt.identities.controller', 'policy.controller.identity'],
        ['receipt.identities.baseline', 'policy.task.baseline'],
        ['receipt.identities.evaluator', 'policy.execution.evaluator'],
      ] as const,
      candidateEvaluatorTreeRule: 'different' as const,
    },
    policyBindings: [
      ['receipt.taskId', 'policy.task.taskId'],
      ['receipt.identities.controller', 'policy.controller.identity'],
      ['receipt.toolVersions.bootstrapControllerStoreDigest', 'policy.bootstrap.controllerStoreDigest'],
      ['receipt.toolVersions.bootstrapBuildManifestDigest', 'policy.controller.buildManifestBlobDigest'],
      ['receipt.toolVersions.bootstrapRuntimeTreeDigest', 'policy.controller.runtimeTreeDigest'],
      ['receipt.toolVersions.bootstrapNodeDigest', 'policy.bootstrap.nodeDigest'],
      ['receipt.toolVersions.bootstrapGitDigest', 'policy.bootstrap.gitDigest'],
      ['receipt.toolVersions.controllerExecutionDigest', 'policy.controller.executionDigest'],
      ['receipt.toolVersions.controllerBuildManifestDigest', 'policy.controller.buildManifestBlobDigest'],
      ['receipt.toolVersions.controllerRuntimeTreeDigest', 'policy.controller.runtimeTreeDigest'],
      ['receipt.toolVersions.controllerManifestDigest', 'policy.controller.manifestBlobDigest'],
      ['receipt.toolVersions.controllerTaskPath', 'policy.controller.taskPath'],
      ['receipt.toolVersions.controllerTaskPathDigest', 'digestValue(policy.controller.taskPath)'],
      ['receipt.toolVersions.controllerTaskDigest', 'policy.controller.taskBlobDigest'],
      ['receipt.toolVersions.taskEvidencePlanDigest', 'policy.taskEvidencePlanDigest'],
      ['receipt.toolVersions.boundTaskDigest', 'policy.execution.boundTaskDigest'],
      ['receipt.toolVersions.programmePolicyFingerprint', 'policy.fingerprint'],
      ['receipt.toolVersions.frozenCargoLockDigest', 'policy.task.rust.frozenLockSha256'],
      ['receipt.protectedInputs', 'policy.execution.protectedInputs'],
      ['receipt.route.snapshotDigest', 'policy.execution.routeSnapshotDigest'],
    ] as const,
    protectedInputs: {
      setRule: 'exact-policy.execution.protectedInputs' as const,
      valueRule: 'non-genesis-lowercase-sha256' as const,
      runtimePaths: ['Cargo.lock'] as const,
      manifestDigestSource: 'policy.controller.manifestBlobDigest' as const,
      taskDigestSource: 'policy.controller.taskBlobDigest' as const,
      buildManifestDigestSource: 'policy.controller.buildManifestBlobDigest' as const,
      controllerLockfileDigestSource: 'policy.controller.lockfileDigest' as const,
      frozenLockDigestSource: 'policy.task.rust.frozenLockSha256' as const,
    },
    route: {
      routerVersion: '@metaharness/router@0.4.0' as const,
      snapshotDigestRule: 'lowercase-sha256' as const,
      snapshotBlobDigestRule: 'sha256(policy.execution.routeSnapshotBlob)' as const,
      snapshotBinding: 'digestValue(strictJson(policy.execution.routeSnapshotBlob))' as const,
      frozenAtRule: 'canonical-iso-timestamp' as const,
    },
    tools: {
      requiredKeys: [...REQUIRED_TOOL_KEYS],
      keySetRule: 'exactly-requiredKeys' as const,
      exactValues: {
        bootstrapSource: 'verified-packed-private-runtime',
        bootstrapNodeDigest: '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc',
        bootstrapGitDigest: '2a8c18fbf43da9f692d75474c72bea9dfd796c260b0f3dfe456376abc3bbd668',
        cargo: '/home/claude/.rustup/toolchains/1.96.0-x86_64-unknown-linux-gnu/bin/cargo#sha256:f30f9fd1b1d0b8fd10dc33219eb4cd4bec3543f40e434ac71f5a03fd0359063f',
        cargoLlvmCov: '/home/claude/.cargo/bin/cargo-llvm-cov#sha256:c59831d34b46a3e3a3dc5b357fa12f75eb0af3172f8e9e81a6fc1412cdbcaa1a',
        node: '/usr/bin/node#sha256:53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc',
        codex: 'codex-cli 0.149.1',
        claude: '2.1.234 (Claude Code)',
        codexExecutable: '/home/claude/.codex/packages/standalone/releases/0.149.1-x86_64-unknown-linux-musl/bin/codex#sha256:73dc5888888f411c1f0fa7b81d866e721dcc86b527ce8e3b2cf4708661e823ba',
        claudeExecutable: '/home/claude/.local/share/claude/versions/2.1.234#sha256:3473601ea695d5bf769c5b202844d4cb4fbf723ae995450fcb6973204775c84a',
        bwrap: '/usr/bin/bwrap#sha256:52231e1caf55bcbc667b269f49c63599a6f7db4767ae6a039580d0ff853db712',
        systemdRun: '/usr/bin/systemd-run#sha256:dbc8b988a849d5c9d7ef2de7068a6f107021bc6c11e0d7864c73f373eef726a7',
        systemctl: '/usr/bin/systemctl#sha256:e0d3d0e9444da1b2b58c792c3f5028b69f049b77d5ca17b3ec0d09f89117225b',
        caBundle: '/etc/ssl/certs/ca-certificates.crt#sha256:6602a85a36afc2e51c66a0df5ae3d383c5b7c2fed93339ccef7d37e01faf09e8',
        agenticQeMcp: '/home/claude/.npm-global/lib/node_modules/agentic-qe/dist/mcp/bundle.js#sha256:a07f22e29ff2dd074e05b30ccdaf76ce042418e6a879d83807e9fdd722dfa483',
        agenticQe: '3.13.12#sast-only-flat-v1+lcov-gap',
        agenticQePackageTreeDigest: '0e7497a02997c9c43c2dbe9c200ce016c8a6c345b8fdb5d5ee99d61ff8722884',
      },
      digestValueKeys: [
        'bootstrapControllerStoreDigest', 'bootstrapBuildManifestDigest',
        'bootstrapRuntimeTreeDigest', 'bootstrapNodeDigest', 'bootstrapGitDigest',
        'controllerExecutionDigest', 'controllerBuildManifestDigest',
        'controllerRuntimeTreeDigest', 'controllerManifestDigest',
        'controllerTaskPathDigest', 'controllerTaskDigest', 'taskEvidencePlanDigest',
        'boundTaskDigest',
        'programmePolicyFingerprint', 'frozenCargoLockDigest',
        'agenticQePackageTreeDigest', 'rustBootstrapClosureMetadata',
        'rustRegistryMetadata',
      ] as const,
      nonEmptyValueKeys: [
        'rufloHive', 'rufloConsensus',
      ] as const,
      structuredValueRules: {
        rustToolchainClosure: {
          format: 'literal:positive-safe-integer:positive-safe-integer',
          leadingValue: '81cc515ef94bae07d2451ff3701ce6e6eee7878327dc8088ebac773f1570f7c4',
        },
        rustRegistryBootstrapSnapshot: {
          format: 'lowercase-sha256:positive-safe-integer:positive-safe-integer',
        },
        rustRegistryClosure: {
          format: 'literal:positive-safe-integer:positive-safe-integer',
          leadingValue: '1bb717af28554b8cbb83ff1a219bbbd294ccee98691191bc9f65dc431106e908',
        },
        rustRegistryLock: {
          format: 'binding:positive-safe-integer:positive-safe-integer',
          leadingBinding: 'policy.task.rust.frozenLockSha256',
        },
        rustRegistrySelection: {
          format: 'literal:lowercase-sha256',
          leadingValue: 'x86_64-unknown-linux-gnu',
        },
      },
      runtimePackages: {
        harness: '@metaharness/harness@0.2.0',
        router: '@metaharness/router@0.4.0',
        codexHost: '@metaharness/host-codex@0.1.2',
        claudeHost: '@metaharness/host-claude-code@0.1.2',
      },
      agenticQePackage: {
        version: '3.13.12',
        treeSha256: '0e7497a02997c9c43c2dbe9c200ce016c8a6c345b8fdb5d5ee99d61ff8722884',
      },
    },
    attempts: {
      finalAttemptSource: 'receipt.recovery.repairCount' as const,
      maximumRepairs: repairs,
      nonFinalEvidenceRetention: 'allowed-but-never-satisfies-final-gates' as const,
      redBaseline: {
        key: 'red-baseline' as const,
        stage: 'red-baseline' as const,
        attempt: 0 as const,
        candidateIdentity: 'receipt.identities.evaluator' as const,
        exitCode: 101 as const,
        required: true as const,
      },
      commands: {
        allowedStages: ['red-baseline', 'build', 'mutation'] as const,
        stageSet: 'exactly-every-allowed-stage' as const,
        attemptRange: 'zero-through-final-attempt' as const,
        declarationBinding: 'exact-programme-command-receipt-projection-v1' as const,
        receiptProjectionFields: [
          'tool', 'executable', 'argv', 'cwd',
        ] as const,
        nonReceiptedPolicyFields: [
          'environmentDigest', 'timeoutMs', 'maxOutputBytes',
        ] as const,
        boundTaskDigestBinding: 'policy.execution.boundTaskDigest' as const,
        rustBinding: {
          sourceTool: 'cargo' as const,
          executable: '/toolchain/bin/cargo' as const,
          environment: {
            PATH: '/cargo-home/bin:/toolchain/bin:/usr/bin',
            HOME: '/home/harness',
            CARGO_HOME: '/cargo-home',
            CARGO_NET_OFFLINE: 'true',
            CARGO_INCREMENTAL: '0',
          },
          mergeRule: 'task-env-overlaid-by-frozen-profile' as const,
        },
        buildExitCode: 0 as const,
        mutationExitCode: 101 as const,
        completed: {
          timedOut: false as const,
          cancelled: false as const,
          outputLimitExceeded: false as const,
          signal: null,
          spawnErrorDigest: null,
        },
        finalBuildAndMutationBindCandidateTree: true as const,
        redSet: 'exact-task-redBaseline-command-multiset' as const,
        finalBuildSet: 'exact-task-build-command-multiset' as const,
        finalMutationSet: 'exact-task-mutation-command-multiset' as const,
        priorBuildSet:
          'per-attempt-nonempty-declared-prefix-ending-failure-or-exact-successful-set' as const,
        noOtherCommandEvidence: true as const,
      },
      verifiers: {
        stages: ['public', 'independent', 'regression'] as const,
        key: 'attempt-{attempt}:{stage}' as const,
        requireEveryStageOnFinalAttempt: true as const,
      },
      artifacts: {
        key: 'attempt-{attempt}:{path}' as const,
        finalAttemptPaths: 'exact-task-artifactPaths' as const,
        minimumFinalCount: 1 as const,
      },
      generatedOutputs: {
        key: 'attempt-{attempt}:{stage}:generated:{evidenceId}' as const,
        finalAttemptSet: 'exact-task-evidence-generatedOutputs' as const,
      },
      mutations: {
        key: 'attempt-{attempt}:mutation:{mutationId}' as const,
        finalAttemptSet: 'exact-task-command-mutationIds' as const,
        everyMutationKilled: true as const,
      },
      qe: {
        key: 'attempt-{attempt}:qe:{profile}' as const,
        finalAttemptProfiles: 'exact-evidence-plan-order' as const,
        coordinationDigests: 'exact-final-qe-digests-in-order' as const,
      },
    },
    candidate: {
      oracle: {
        requiredMode: 'verifier-only' as const,
        source: 'policy.task.candidateOracle.mode' as const,
      },
      receiptStatus: 'pass' as const,
      admittedPaths: 'exact-task-requiredAdmittedPaths' as const,
      patchDigest: 'non-null-lowercase-sha256' as const,
    },
    nativeControlPlane: {
      hosts: [
        {
          host: 'codex', authClass: 'native-openai-subscription',
          model: 'gpt-5.6-sol', role: 'implementation-review',
          clientVersion: 'codex-cli 0.149.1', subscriptionCostUsd: 0,
        },
        {
          host: 'claude-code', authClass: 'native-anthropic-subscription',
          model: 'claude-sonnet-4-6', role: 'architecture-review',
          clientVersion: '2.1.234 (Claude Code)', subscriptionCostUsd: 0,
        },
      ] as const,
      hostCardinality: 'exactly-once-each' as const,
      minimumCritiqueDigests: 1 as const,
      finalReviewDigests: 2 as const,
      finalReviewOrder: ['codex', 'claude-code'] as const,
      uniqueFinalReviewDigests: true as const,
      minimumNativeEvidenceDigests: 4 as const,
      uniqueNativeEvidenceDigests: true as const,
      nativeRuntimeEvidenceDigestRequired: true as const,
      clientVersionBindings: [
        ['receipt.hosts[codex].clientVersion', 'receipt.toolVersions.codex'],
        ['receipt.hosts[claude-code].clientVersion', 'receipt.toolVersions.claude'],
      ] as const,
    },
    reliability: {
      cancelled: false as const,
      breakerState: 'closed' as const,
      retryCountRule: 'nonnegative-within-receipt-schema' as const,
      everyCommandCompleted: true as const,
    },
    coordination: {
      swarmId: 'non-empty' as const,
      taskId: 'non-empty' as const,
      rufloHive: 'non-empty' as const,
      rufloConsensus: 'non-empty' as const,
      routeSnapshotBinding: 'receipt.route.snapshotDigest-equals-attested-ruflo-route' as const,
      qeEvidenceDigests: 'exact-final-qe-digests-in-evidence-plan-order' as const,
      rufloEvidenceBinding: 'embedded-envelope-ruflo-evidence' as const,
    },
    envelope: {
      schemaVersion: 5 as const,
      authority: DEVELOPMENT_AUTHORITY,
      policyFingerprintBinding: 'externally-supplied-trusted-anchor' as const,
      receiptCount: 1 as const,
      diagnosticBlobPath: 'coding-harness/config/metaharness-diagnostics.json' as const,
      diagnosticBlobMaximumBytes: 1_000_000 as const,
      diagnosticBlobDigestRule: 'sha256(envelope.diagnosticBlob)' as const,
      diagnosticProtectedInputBinding:
        'receipt.protectedInputs[envelope.diagnosticBlobPath]-equals-envelope.diagnosticBlobDigest' as const,
      acceptanceDigestRule: 'digestValue(envelope.programmeAcceptance)' as const,
      envelopeDigestRule: 'digestValue(envelope-without-envelopeDigest)' as const,
    },
    diagnostics: {
      identity: {
        schemaVersion: 1 as const,
        authority: DEVELOPMENT_AUTHORITY,
        source: 'ruflo-metaharness-score-mcp' as const,
      },
      implementation: {
        ruflo: '3.38.20',
        claudeFlowCli: '3.34.0',
        metaharnessRange: '~0.3.0',
        metaharness: '0.3.2',
        wrapperDigest: 'e14b64f1bcd51c61d3a33c0f1c6712c248e71726f104cbab1d0e0ffc26d775d5',
        bridgeDigest: '9bd511d2ed8b52d40a911113169f8cb9075a124e0a49b966c3975e6959cd0302',
        scorecardDigest: '89af201762b2e1284ca0715bc77bbb5c76a9703113ac7e139f2be282db423a31',
        analyzerDigest: 'da6050f1db03a5f8a267074b6f8f06f05d7c8c5ea158b391edd9443cff9a55f9',
      },
      targets: [
        { target: 'repository', repositoryPath: '.', schema: 1, hardConstraintsTotal: 6 },
        {
          target: 'coding-harness', repositoryPath: 'coding-harness',
          schema: 1, hardConstraintsTotal: 6,
        },
      ] as const,
      targetCardinality: 'exactly-once-in-declared-order' as const,
      capturedAtRule: 'canonical-iso-timestamp' as const,
      blobDigestBinding: 'envelope.diagnosticBlobDigest' as const,
      pass: {
        success: true as const,
        degraded: false as const,
        exitCode: 0 as const,
        scaffoldReady: true as const,
        hardConstraintsPassed: 'equals-hardConstraintsTotal' as const,
      },
    },
  });
}

export type ProgrammeGateContractV1 = ReturnType<typeof createProgrammeGateContractV1>;

export function parseProgrammeGateContractV1(value: unknown): ProgrammeGateContractV1 {
  const input = asRecord(value, 'programme policy v5.gateContract');
  assertExactKeys(input, [
    'schemaVersion', 'authoritativeReplaySemantics', 'receiptSchemaVersion',
    'taskSchemaVersion', 'acceptanceSchemaVersion', 'acceptance', 'receipt', 'policyBindings',
    'protectedInputs', 'route', 'tools', 'attempts', 'candidate',
    'nativeControlPlane', 'reliability', 'coordination', 'envelope', 'diagnostics',
  ], 'programme policy v5.gateContract');
  const attempts = asRecord(input.attempts, 'programme policy v5.gateContract.attempts');
  const expected = createProgrammeGateContractV1(asInteger(
    attempts.maximumRepairs,
    'programme policy v5.gateContract.attempts.maximumRepairs',
  ));
  assertExactShape(input, expected, 'programme policy v5.gateContract');
  return expected;
}

function assertExactShape(actual: unknown, expected: unknown, label: string): void {
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual) || actual.length !== expected.length) invalid();
    expected.forEach((entry, index) => assertExactShape(
      (actual as unknown[])[index], entry, `${label}[${index}]`,
    ));
    return;
  }
  if (expected !== null && typeof expected === 'object') {
    const input = asRecord(actual, label);
    const keys = Object.keys(expected as Record<string, unknown>);
    assertExactKeys(input, keys, label);
    for (const key of keys) {
      assertExactShape(input[key], (expected as Record<string, unknown>)[key], `${label}.${key}`);
    }
    return;
  }
  if (!Object.is(actual, expected)) invalid();

  function invalid(): never {
    throw new Error('HARNESS_PROGRAMME_POLICY_GATE_CONTRACT_INVALID');
  }
}
