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
keys, ambient proxy variables, base-URL overrides, OpenRouter, and Requesty are
rejected. The controller injects only a loopback CONNECT endpoint backed by its
exact-origin Unix-socket broker.

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
  → receipt-bound seven-dimension programme envelope
```

Every repair resets the candidate and repeats admission, build, all verifier
lanes, and both reviews. Candidate commands require an OS network namespace;
dependency installation is a separate registry-pinned stage. Missing isolation
or either native host fails closed.

Native model execution uses independently enforced exact-origin, filesystem, and
resource boundaries. The trusted runtime exposes only a Git-masked candidate
snapshot, private output channel, empty private home, copied credential
capability, and one broker socket; it hides the controller, evaluator, verifier,
common Git object store, and other host paths. A systemd cgroup-v2 transient unit
enforces CPU, memory, PID, file-size, descriptor, and runtime ceilings. Timeout,
cancellation, or output overflow stops and verifies the exact unit before a
result can be accepted, and execution failure revokes active broker sessions.

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
- `issue-8-programme-envelope.ts` binds the project-owned acceptance score to
  the exact receipt and makes a rejected score a non-zero launcher result.
- `metaharness-diagnostics.ts` parses the protected native Ruflo score snapshot;
  its exact Git blob digest must match the candidate receipt.
- `.harness/manifest.json` is the canonical tracked control-plane manifest and
  identifies the repository's actual `.mcp.json` coordination surface.

The issue #8 acceptance definition lives in
`config/issue-8-acceptance.json`. It is not an evolution suite or a promotion
signal. Darwin/GEPA remains disabled until five training tasks and five sealed
holdouts satisfy the independent evaluator gate.
