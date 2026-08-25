# semantic-fabric coding harness

Private, development-only control plane for verified repository changes. It
coordinates native Codex/ChatGPT and Claude Code subscriptions, but direct
repository evaluators decide whether a candidate passes.

The package has no CLI, MCP server, publication path, commit/push authority, or
evolution command. It returns evidence and a patch to an explicit integration
owner.

## Local verification

```bash
npm ci
npm run build
npm test
```

CI tests use fake model processes and make no provider calls. Runtime model
execution requires successful native subscription preflights. Provider API
keys, proxy variables, base-URL overrides, OpenRouter, and Requesty are rejected.

## Transaction

```text
frozen baseline + evaluator worktrees
  → distinct-host architecture and bounded critique
  → implementation
  → exact-path patch admission
  → clean candidate apply and offline build
  → public + independent + regression verification in parallel
  → independent Codex and Claude reviews
  → protected-input check and digest-chained receipt
```

Every repair resets the candidate and repeats admission, build, all verifier
lanes, and both reviews. Candidate commands require an OS network namespace;
dependency installation is a separate registry-pinned stage. Missing isolation
or either native host fails closed.

Native model execution additionally requires injected, independently enforced
origin-pinning and filesystem boundaries. The filesystem grant must expose only
a Git-masked candidate snapshot, private output channel, empty private home, and
brokered authentication; it must hide the controller, evaluator, verifier,
common Git object store, and all other host paths. This package validates that
contract but does not bundle a trusted boundary or auth broker, so a real
programme remains gated rather than treating an interface claim or login check
as isolation. CPU, memory, PID, and disk quotas are also required before running
untrusted model or Cargo workloads; the current wall-time/output limits do not
replace them.

## Main modules

- `kernel.ts` composes the real `HarnessKernel`, `AlgorithmRouter`,
  `VerifierRegistry`, `PolicyGate`, persistent routed pool, critique, consensus,
  and memory hooks.
- `candidate.ts`, `git-worktrees.ts`, and `repository-operations.ts` enforce the
  patched-candidate transaction and identity checks.
- `native-process.ts`, `network.ts`, and `models/` implement bounded native
  execution, first-party-only transport, routing, retry, breakers, and review
  independence.
- `contracts.ts`, `policy.ts`, `evidence.ts`, and `receipts.ts` validate task,
  Ruflo, Agentic-QE, protected-input, and receipt boundaries.
- `.harness/manifest.json` is the canonical tracked control-plane manifest and
  identifies the repository's actual `.mcp.json` coordination surface.

The issue #8 acceptance definition lives in
`config/issue-8-acceptance.json`. It is not an evolution suite or a promotion
signal. Darwin/GEPA remains disabled until five training tasks and five sealed
holdouts satisfy the independent evaluator gate.
