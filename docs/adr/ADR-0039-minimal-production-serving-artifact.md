---
status: proposed
date: 2026-08-28
updated: 2026-08-29
tags: [production, packaging, release, cargo, reproducibility, sbom, provenance, supply-chain]
supersedes: []
depends-on:
  - ADR-0006
  - ADR-0010
  - ADR-0012
  - ADR-0038
implements:
  - ADR-0038
---

# Minimal production serving artifact

## Status boundary

This ADR is a **proposal**, not an acceptance or implementation claim. Its
`implements` relationship means that it is the subordinate design lock requested
by ADR-0038 M0; no release is conformant until the acceptance gates below pass.

Interim M0 tooling records and verifies a host-observed non-closure observation
of the current all-in-one `sf-cli` executable. On 2026-08-28 the first exact
observation was captured from clean source commit
`5a06eacbf0164ca36a1421d3106247034e0d1e7b` and replayed structurally with the
same CLI. Its external `0600` receipt contains 363 raw final-link inputs, 357
canonical terminal inputs, and three one-hop alias records. It binds portable
authority digest
`72ce37b4320e5126ef32a693eab80f2f30df3757881c6e2495e8882068079b9a`,
host-observation digest
`024fbbbde94471f15a15e65193b1a0c66767f5590b2621f8d9c7b896381a3ad8`,
and receipt digest
`173d0698e7955da881a39574bb5a08d302b80b67fc3e89a95c27d282270e51ca`.
The receipt remains outside the repository and
has not been committed, uploaded, promoted, or made canonical; these summary
digests are not a substitute for its replayable bytes.

The linker exception is deliberately narrower than generic file authority. It
accepts only a final one-hop symlink named by the dependency file when both its
alias and independently normalized terminal map to `HostSystem`. The receipt
binds the alias, terminal, raw target topology, and terminal bytes; held alias
and terminal descriptors plus root guards are rechecked at phase boundaries.
Generic workspace, toolchain, Cargo, build-output, and other authority paths
remain symlink- and hard-link-rejecting. The raw primary/phony dependency-file
sequences must agree before normalization, and GNU build IDs are accepted only
from one structurally valid `.note.gnu.build-id` record whose owner, type,
declared size, and lowercase digest agree.

An additive runtime-linkage contract fixes a strict bounded parser for glibc
`ld.so --list`. Commit `863a058` added a private Linux holder that descriptor-
resolves the discovered artifact, loader and DSOs through guarded roots, rejects
nested mount crossings and noncanonical aliases, snapshots twice-verified bytes
into exactly sealed close-on-exec memfds, reparses them by ELF role, and checks
canonical identity plus static `DT_NEEDED` provider equality/reachability.

Commit `c8305c3` adds a private one-shot prepared executor. An independent
expectation binds exact bubblewrap path, digest, length and executable policy; the
root-owned inode is held and rechecked. Exact sealed-source duplicates are
identity-, seal-, digest-, length- and byte-checked before being passed through an
explicit FD allowlist. The child `execveat`s only the held bubblewrap inode with
an empty environment, per-process limits, parent-death signalling, pidfd/direct-
child and process-group cleanup, timeouts, and bounded cancellable output.
Bubblewrap unshares user/network/all namespaces, drops capabilities, constructs a
fresh bounded tmpfs with no host binds, copies only the sealed artifact/loader/DSO
bytes to fixed paths, remounts it read-only and runs the copied loader. Strict
`ld.so --inhibit-cache --glibc-hwcaps-mask "" --list /artifact` output must equal
prior discovery before the private in-memory observation is returned. The manual
workflow-dispatch smoke pins the exact host labels, test identity and bubblewrap
digest; it is not a merge or release gate.

Commit `805f413` maps a completed observation into a private canonical
`authority=none`, `non-admission-only` record. Domain-separated record and
receipt digests cover the exact semantic view, bubblewrap identity and policy,
ordered source/destination bindings, and bounded raw stdout. Thirty-four fixed
`not-attested` fields preserve the missing authorities. Provider-free semantic
replay reparses the embedded stdout and requires semantic equality without
executing a process or consulting the filesystem, network, model, or provider.
The exact-host smoke renders, parses, and replays the record only in memory; no
durable writer or importer exists.

Commit `9282e60` adds a caller-supplied expectation for the existing closed
runtime-ELF dynamic-tag/search/flag policy. The holder captures ID
`elf64-le-x86_64-closed-dynamic-tags-safe-search-flags-v1` and implementation-
source digest
`cd23f2d883c1e99b655395284e7d803e6d00b9eaf90a417560efca7ffde50b0a`
only after all sealed objects and their static dependency graph validate. Exact
equality is required before prepared-probe construction and again during the
immediate pre-run validation phase; ID or digest drift returns without invoking
the runner. The exact
native diagnostic maintains a separate literal, but the API authenticates no
reviewer. The actual pair remains private in-memory holder/observation metadata
and is not serialized. Receipt V1's schema and canonical serialization remain
unchanged with `runtime-elf-policy-replay=not-attested`, so this satisfies no artifact,
replay, provenance, admission or release gate.

Commit `73e9864` adds exact late syscall confinement to that prepared observation.
Policy `x86_64-prepared-loader-late-cbpf-default-kill-v1` contains 55 classic-BPF
instructions (440 bytes) and has SHA-256
`0092c69f902c071515f2f82c5aff75bf63f065148f1c0fb51af414787338e80a`.
A separately sealed, close-on-exec high-numbered FD is checked before and after
transfer and appears exactly once as `--seccomp <fd>` immediately before `--`.
The same late default-kill filter confines bubblewrap's namespace PID 1/reaper
and the copied loader child. A native identical-layout `fstat` control succeeds
while a `socket` canary dies by `SIGSYS`; the private live observation binds the
policy identity. Receipt V1 remains byte-compatible and records
`target-seccomp-or-syscall-trace=not-attested`, so it contains no syscall trace or
final-FD inventory and does not convert this diagnostic into replay authority.

On 2026-08-29, commit `50adc0a` added a pre-construction check over the exact
bounded bubblewrap bytes used for the authorized length and SHA-256: those same
bytes must parse as `RootPie` under the existing expected runtime-ELF policy,
and the held inode, digest, and policy are fenced around the native observation
and canary. Both exact native controls pass. This is static preflight only:
Receipt V1 is unchanged and non-attesting, and bubblewrap host runtime closure
remains unbound.

Commit `b34b6d7` adds a mutually separate canonical private `authority=none`
inventory under relation
`counterfactual-controlled-name-resolution-not-actual-exec`. It derives the
interpreter path and sorted direct `DT_NEEDED` names from the held `RootPie`
bytes, then clears the environment, sets `LC_ALL=C` and `/`, and runs that host
interpreter with `--inhibit-cache`, an empty `--glibc-hwcaps-mask`, and
`--list <authorized-bwrap-path>`. Pre/post checks revalidate held bwrap identity,
runtime policy, and the fixed process plan. A domain-separated inventory digest binds that identity/policy metadata,
bounded raw stdout and replayed SONAME/path bindings. The
interpreter, reported DSOs, and path-passed target are unheld and undigested;
bwrap is not executed, and provider-free replay proves only record consistency.
Receipt V1 remains unchanged. This does not accept this ADR or satisfy gates 1–3.

This does not accept this ADR. Discovery remains prior and unauthorized; the
artifact is not executed; relocation, symbol/version, initialization, `dlopen`,
NSS, VDSO and closure completeness remain unproven. The digest detects byte drift
in five embedded policy sources; it does not classify changes, prove review, or
bind compiled bytes, configuration, dependencies or toolchain. Opaque GNU-
property, hash, symbol, relocation, version, TLS and cross-table payload semantics
remain unproved. The loader
consumes tmpfs copies sourced from sealed memfds, not the original held inode capabilities.
Although counterfactual names and paths are now inventoried, actual interpreter,
DSO, and target-path byte consumption, time-of-use, default cache/hwcaps
equivalence, preload and LSM state remain unbound; kernel, bubblewrap, glibc,
copy and mount semantics are trusted. The exact late filter is
live observation only: there is no final-FD inventory or syscall trace, and
Receipt V1 does not attest it. There is no aggregate cgroup process/memory bound
or control-group kill. Same-principal/root ABA, rollback and hostile kernel/filesystem
resistance remain out of scope. There is no production caller or canonical
public/durable receipt path, signature, external witness, authenticated
execution/output provenance, SBOM, reproducibility, minimality, performance,
admission or release authority. Semantic replay is self-consistency validation,
not execution replay or authenticity; a fully reminted self-consistent record
remains a different unauthenticated non-admission record.

This observation is about the artifact which exists today, not this proposal's
`sf-server`, and it is not a complete binary closure, SBOM, reproducibility
result, production-minimality proof, or admission receipt. Its configured tool
identities and final-link dependency file are observations, not complete tool-
execution evidence or proof of exclusive linker authorship. Link-input bytes are
observed after linking, so linker time-of-use and path-resolution race resistance
remain explicitly unattested. Authority ancestry and held-directory checks fail
closed on untrusted or persistent replacement, while same-principal/root ABA
resistance still requires an exclusive, quiescent builder principal. Hosted CI
exercises parser/contract mutants; the exact prepared smoke is manual and private.
Neither publishes an authoritative observation. This advances the
implementation baseline without accepting this ADR, satisfying gates 1–3, or
resolving the proposed packaging decision.

## Context and problem statement

ADR-0006 fixed one `sf-cli` package and one `semantic-fabric` binary containing
`serve`, `conformance`, and `bench`. The current package therefore depends on
`sf-serve`, `sf-conformance`, and `sf-bench`. Feature unification through the
evidence crates also brings SHACL, SQL Server, REST/cloud prototypes, and their
dependencies into builds that should represent only the production server.

That shape no longer gives a trustworthy answer to “what code is in the shipped
server?” The workspace version is also `0.0.0`, and a green workspace build does
not prove that the application artifact was built from the tracked lockfile or
that its dependency closure matches the admitted serving surface. ADR-0038 M0
and M7 require a minimal, reproducible, auditable artifact without changing the
semantic compiler.

## Considered options

- **Keep the all-in-one CLI and rely on optional features.** Rejected. The public
  command surface and Cargo feature unification would continue to make evidence
  tooling part of the production packaging boundary.
- **Ship `sf-serve` directly.** Rejected. `sf-serve` is a library/runtime boundary,
  not a versioned product binary with an explicit release closure and command
  contract.
- **Create a production binary crate and a separately named developer tool.**
  Proposed. Cargo package boundaries make the release closure mechanically
  inspectable and keep evidence dependencies available without shipping them.

## Proposed decision

### 1. Artifact identities and command surface

Add a structurally separate Cargo package named **`sf-server`**. It owns the
production `[[bin]]` named **`semantic-fabric`** and depends only on the runtime
libraries needed to serve admitted backends. The public invocation retains
`semantic-fabric serve` for compatibility. It exposes no `conformance`, `bench`,
fixture-generation, or prototype-adapter command.

Rename the current tooling package from `sf-cli` to **`sf-dev-cli`**, with a
developer-only binary named **`semantic-fabric-dev`**. Conformance, benchmark,
fixture, and other evidence commands live there. It is built in developer and CI
evidence lanes, but is neither installed nor packed by the product release job.

The split is structural, not a pair of feature aliases on one package. The
production crate must not have optional dependency edges to the evidence crates;
otherwise a feature or workspace build could silently reintroduce them.

### 2. Exact default production closure

The `sf-server` default backend set is exactly:

1. SQLite;
2. PostgreSQL; and
3. MySQL.

Its normal/build dependency graph may contain the shared semantic, mapping, SQL,
SPARQL, serving, configuration, security, telemetry, and serialization runtime
needed for those three backends. It must exclude every dependency reachable
solely through:

- `sf-conformance`, `sf-bench`, SHACL or in-memory oracle tooling;
- SQL Server, DuckDB, HANA, Oracle, ODBC, MonetDB, REST, cloud, or other prototype
  adapters;
- Criterion, test-fixture generation, EARL production, or benchmark/reporting
  code; and
- `sf-dev-cli` itself.

Non-default experimental features elsewhere in the workspace do not make a
backend production-supported. A future backend joins this closure only through
the ADR-0038 capability/admission process and a reviewed closure update.

### 3. Version and locked build contract

The first qualifying product release is `0.1.0` unless the release process has
already advanced to a higher SemVer value. `0.0.0`, an empty version, a dirty-tree
suffix, or a version that differs between the binary, SBOM, provenance, and
package metadata fails the release.

The canonical build is package-specific, never `--workspace`:

```text
cargo build --locked --release -p sf-server --bin semantic-fabric
```

The tracked root `Cargo.lock`, pinned Rust toolchain, exact source revision, target
triple, build flags, and generated dependency-closure digest are release inputs.
No release job may regenerate or update the lockfile.

### 4. Release evidence bound to the exact artifact

One release transaction produces and binds:

- the packed `semantic-fabric` binary or OCI payload and its SHA-256 checksum;
- a CycloneDX or SPDX SBOM containing package versions, sources, licences, target,
  enabled features, and the root `Cargo.lock` digest;
- signed SLSA-compatible provenance naming the revision, toolchain, builder,
  exact locked command, target, closure digest, SBOM digest, and artifact digest;
- licence/source/duplicate/advisory results for the reachable production closure,
  including owner, expiry, reachability, and controls for every waiver; and
- clean-machine smoke receipts for SQLite, PostgreSQL, and MySQL using the exact
  checked artifact, not a later local rebuild.

Checksums, SBOM, and provenance must verify independently and offline from the
release bundle. Signing does not rescue a closure, test, audit, or reproducibility
failure.

### 5. Relationship to ADR-0006

If accepted, this ADR narrowly amends ADR-0006's **single all-in-one `sf-cli`
packaging clause**, its `sf-cli --help` confirmation, and the corresponding
“single-binary ethos” wording. The public product remains one
`semantic-fabric` server binary; only developer evidence tooling moves to a
separate, unmistakably non-production artifact.

No compiler or execution architecture changes: `sf-core`, `sf-mapping`,
`sf-sparql`, `sf-sql`, the IQ/cascade, dialect emission, native streaming, and
term reconstruction remain intact. Until this proposal is accepted, ADR-0006's
historical text and status are unchanged.

## Exact acceptance gates

This proposal may move to `accepted` only when all of the following evidence is
reviewed against one immutable candidate:

1. `cargo metadata` identifies distinct `sf-server` and `sf-dev-cli` packages,
   and only `sf-server` produces the public `semantic-fabric` binary.
2. `semantic-fabric --help` exposes serving only, while
   `semantic-fabric-dev --help` owns the evidence commands.
3. A package-specific, target-specific `cargo tree --locked -e normal,build`
   receipt has exactly the SQLite/PostgreSQL/MySQL backend features and none of
   the excluded crates, features, native libraries, or prototype transports.
4. A mutation that adds `sf-conformance`, `sf-bench`, SHACL, SQL Server, REST,
   cloud, or prototype reachability makes the closure gate fail.
5. Two isolated clean builders using the same tracked lockfile and toolchain
   produce the same closure digest and byte-identical packed artifact; any
   documented platform-signing envelope is applied only after that comparison.
6. The binary and every bound release document report the same non-zero SemVer.
7. SQLite, PostgreSQL, and MySQL clean-machine smoke and backend admission tests
   pass against the artifact checksum named in the provenance.
8. SBOM completeness, checksum, signature, provenance, licence, source,
   duplicate, and reachable-vulnerability verification all pass with no
   unwaived critical/high finding.
9. Existing semantic, conformance, and benchmark gates still run through the
   developer/evidence lane, proving that separation did not delete evidence.

Passing only `cargo build`, or building a different artifact than the one
checked, is not acceptance evidence.

## Consequences

- Good: the public artifact has a small, mechanically testable attack and supply-
  chain surface.
- Good: conformance and benchmark tooling remain first-class without being
  confused with product runtime capability.
- Good: the change leaves the proven semantic compiler and backend ports intact.
- Cost: CI must build and test two command artifacts and guard against workspace
  feature unification in release jobs.
- Cost: reproducible packaging, SBOM, provenance, and three-backend smoke require
  controlled builders and retained evidence.

## Rules

- **R1** — `semantic-fabric` is produced only by `sf-server`; evidence tooling is
  produced only by `sf-dev-cli` as `semantic-fabric-dev`.
- **R2** — the default production backend closure is exactly SQLite, PostgreSQL,
  and MySQL; no prototype closure is latent in the product package.
- **R3** — every release build is package-specific, release-mode, and `--locked`.
- **R4** — version, lockfile, closure, SBOM, provenance, checksum, and smoke
  receipts bind the same immutable artifact.
- **R5** — this packaging split does not authorize semantic-compiler changes or
  claim that ADR-0038 is implemented.

## More information

- Programme decision: ADR-0038, especially M0 and M7.
- Existing packaging/performance decision amended only if accepted: ADR-0006.
- Serving security and resource boundary: ADR-0010.
- Deterministic evidence authority: ADR-0012.
