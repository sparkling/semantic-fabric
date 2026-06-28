# Vendored: GTFS-Madrid-Bench (provenance)

These files are vendored **verbatim** from the official GTFS-Madrid-Bench, the
reference OBDA / query-rewriting benchmark used by `sf-bench` (ADR-0005).

- **Upstream:** <https://github.com/oeg-upm/gtfs-bench>
- **Commit:** `7fcdaa7456ef506d9eb2d7354550d35eb0209932`
- **License:** Apache-2.0 (see `LICENSE`)
- **Authors:** Ontology Engineering Group (OEG), Universidad Politécnica de Madrid
- **Reference:** Chaves-Fraga, Priyatna, Cimmino, Toledo, Ruckhaus, Corcho,
  "GTFS-Madrid-Bench: A benchmark for virtual knowledge graph access in the
  transport domain", *Journal of Web Semantics*, 2020.

## Files

| File | Upstream path | Role here |
|---|---|---|
| `gtfs-rdb.r2rml.ttl` | `mappings/gtfs-rdb.r2rml.ttl` | The official R2RML mapping (relational → GTFS RDF). Reference artifact. |
| `postgresql.sql` | `utils/postgresql.sql` | The official relational schema (table/column shape). Reference artifact. |
| `LICENSE` | `LICENSE` | Apache-2.0. |

## How `sf-bench` uses these

`sf-bench` drives the engine with a **self-contained, cross-reference-consistent
subset** derived from these artifacts — see `crates/sf-bench/src/workload.rs`
(`MAPPING_TTL`) and `crates/sf-bench/README.md`. The derived subset:

- keeps the GTFS vocabulary (`http://vocab.gtfs.org/terms#`) and the
  `http://transport.linkeddata.es/madrid/metro/` subject IRIs from the official
  mapping;
- mirrors the official table/column names from `postgresql.sql` (six core tables:
  `AGENCY`, `CALENDAR`, `ROUTES`, `STOPS`, `TRIPS`, `STOP_TIMES`);
- makes every `rr:parentTriplesMap` resolvable within the subset (so any query is
  valid at any scale), and removes the `gtfs:headsign` predicate collision
  between `TRIPS` and `STOP_TIMES` so the OPTIONAL query (Q5) resolves to a single
  branch within the v1 support surface (ADR-0007).

The official mapping is retained here unmodified as the faithful reference.
