---
status: proposed
date: 2026-08-28
updated: 2026-08-28
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

This observation is about the artifact which exists today, not this proposal's
`sf-server`, and it is not a complete binary closure, SBOM, reproducibility
result, production-minimality proof, or admission receipt. Its configured tool
identities and final-link dependency file are observations, not complete tool-
execution evidence or proof of exclusive linker authorship. Link-input bytes are
observed after linking, so linker time-of-use and path-resolution race resistance
remain explicitly unattested. Authority ancestry and held-directory checks fail
closed on untrusted or persistent replacement, while same-principal/root ABA
resistance still requires an exclusive, quiescent builder principal. CI
exercises only the parser/contract on its mutable hosted runner and does not
capture or publish an observation. This advances the implementation baseline
without accepting this ADR, satisfying gates 1–3, or resolving the proposed
packaging decision.

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
