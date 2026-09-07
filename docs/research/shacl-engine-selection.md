# SOTA Rust SHACL Engine — selection for the M⋈T gate

**Research key:** `shacl-engine-selection`
**Date:** 2026-06-27 (round-2 deep-research)
**Scope:** the Rust SHACL runner for ADR-0005's four-shape `M ⋈ T` mapping-output gate: three Core shapes plus one sealed datatype `sh:sparql` component. It must be Rust-native, in-process, no-JVM and oxrdf-aligned.
**Decision recorded in:** ADR-0005 (decision update 2026-06-27).

> **Implementation reconciliation (2026-09-06).** The active sealed evaluator pins `shacl`, `rudof_rdf` and `sparql_service` 0.3.14, `oxigraph` 0.5.9, `spareval` 0.2.6, `spargebra` 0.4.6 and `oxrdf` 0.3.3 plus their reviewed resolved feature profile. Earlier 0.3.4 examples below are superseded. Receipt policy v2 binds those identities and the execution topology; dependency availability alone grants no admission.

## Bottom line

**Adopt the lean rudof crates on the oxrdf stack.** Run the three sealed Core shapes in `ShaclValidationMode::Native`; extract the fourth shape's exact parsed and digest-pinned datatype `sh:select`, deactivate only that shape on the Native branch, and execute the query once globally over the same store. This avoids rudof's per-focus quadratic setup without introducing another RDF stack. The public outcome is count-faithful but report-lossy; raw blank POM focus retains the old redacted failure boundary because the product projection is IRI-skolemised.

## Contradiction resolved

| Prior claim | Verdict | Why |
|---|---|---|
| "rudof = primary candidate" | **Correct** | oxrdf-native, no-JVM, full Core in source, peer-reviewed lineage |
| "oxirs-shacl = production, 27/27 W3C Core" | **Misleading** | "27/27 W3C" is **not** the official W3C suite (**121 tests**); oxirs is **absent** from the W3C implementation report; AI-generated-claim markers (0 issues across 26 crates; "43,500 tests/100%"); validates `oxirs-core` types (+ `scirs2-*` platform), **not** oxrdf |
| "grafeo = 28/28 + SHACL-SPARQL" | **Real, wrong architecture** | healthy project (678★) but a **separate graph DB** — data must live in GrafeoDB = a second RDF stack |

Decisive framing: the 8 components we need (`NodeShape`, `property`, `class`, `datatype`, `nodeKind`, `minCount`/`maxCount`, `in`, `hasValue`) are the **most trivial, universally-supported** subset of SHACL Core — all three support them, so coverage is **not** the differentiator; **integration + credibility + binary footprint** are. And **none of the three has an official W3C implementation-report submission** — every "27/27"/"28/28" number is self-reported.

## Candidate facts

- **rudof** — WESO group (Labra Gayo, U. Oviedo), MIT OR Apache-2.0. CI runs the W3C Core suite through its NativeEngine; the complete Core constraint set is present in source. Perf (CEUR/ISWC-2024 LUBM): rudof **7.90 ms** vs RDF4J 1.64 ms vs Jena 60.36 ms vs TopQuadrant 85.74 ms. Predecessor **shaclex** (Scala) is in the official W3C report at **98/121 (81%)**. **Risk:** per-focus execution of the sealed datatype SPARQL rule took about 114 s on Product Mock, so the reviewed rule is batched exactly once with differential and structural guards.
- **oxirs-shacl** — 0.3.1 (2026-06-06), Apache-2.0, 71★; cool-japan/OxiRS; not recommended (see table). Validates `oxirs-core` (the "zero-dependency" claim is false — ~50+ deps incl. `scirs2-*`, `tokio`, `reqwest`, `tower`, `wasmi`).
- **grafeo-engine** — 0.5.42 (2026-05-04), Apache-2.0, 678★; genuine, healthy pure-Rust graph DB; wrong fit (separate store).

## The spike (now a standard unit test, not a gate)

Flow: parse bounded `M ⋈ T` → exact logical-work/result preflight → reject blank POM focus → structurally verify four named target shapes and exactly one BasicSparql component → execute that parsed query once → run the other three shapes through Native → compose violation/warning counts under the preflight bound. Policy-v2 KAT/mutation tests cover every execution input. The ignored exact static Product Mock test asserts 47,463 ontology + 3,064 projection = 50,527 closure triples and 0/0. Uncontrolled validation-only observations of about 1.3–1.9 s are diagnostic, not a benchmark or admission result.

## Evidence grades
- rudof srdf = oxrdf-0.3/oxttl-0.2-native, no endpoint — **High** (docs.rs/srdf deps).
- rudof implements all 8 + full Core; the exact Native/global split is product-tested — **High** (source plus differential/mutation tests).
- LUBM benchmark numbers — **High** (CEUR Vol-3828 paper32).
- W3C suite = 121 tests; none of the three submitted; shaclex 81% — **High** (W3C report).
- oxirs "27/27" ≠ W3C suite; validates oxirs-core+scirs2 — **High** (W3C report + docs.rs deps).
- oxirs AI-generated-claim pattern — **Med** (README + repo metrics).

## Sources
- https://github.com/rudof-project/rudof · https://crates.io/crates/rudof · https://docs.rs/shacl_validation/ · https://docs.rs/srdf/latest/srdf/ · https://docs.rs/crate/rudof_lib/latest
- https://ceur-ws.org/Vol-3828/paper32.pdf (rudof CEUR/ISWC-2024 + LUBM SHACL benchmark)
- https://w3c.github.io/data-shapes/data-shapes-test-suite/ (official SHACL implementation report — 121 tests)
- https://github.com/cool-japan/oxirs · https://docs.rs/crate/oxirs-shacl/latest · https://docs.rs/oxirs-core/latest/oxirs_core/
- https://github.com/GrafeoDB/grafeo · https://grafeo.dev/ · https://lib.rs/crates/grafeo-engine
- https://www.w3.org/TR/shacl/ (SHACL Core — 28 constraint components)
