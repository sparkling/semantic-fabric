# SDM-RDFizer: Operational Data Structures for Dedup-Efficient Materialization

**Topic**: SDM-RDFizer internals — dictionary encoding, PTT/PJTT dedup structures, operator complexity, planning phase, and what a Rust engine should adopt.

**Evidence quality**: High — primary sources are the peer-reviewed CIKM 2020 paper (arXiv preprint), the Semantic Web Journal extension (v4.5.6), the KGCW 2024/2025 challenge reports, and direct inspection of the Python source via the GitHub API.

---

## 1. What SDM-RDFizer Is

SDM-RDFizer is a Python-based RML mapping engine developed by the Scientific Data Management group at TIB Hannover. It was first published at [CIKM 2020](https://dl.acm.org/doi/abs/10.1145/3340531.3412881) ([arXiv preprint](https://arxiv.org/pdf/2008.07176)) and extended to version 4.x in a [Semantic Web Journal paper](https://www.semantic-web-journal.net/system/files/swj3246.pdf). The [GitHub repository](https://github.com/SDM-TIB/SDM-RDFizer) is actively maintained (current version 4.7.5.14, April 2026).

Its primary research claim is that conventional RML engines (RMLMapper, RocketRML) fail on large or high-duplicate datasets because they lack efficient data structures for join execution and duplicate elimination. SDM-RDFizer fixes this with three dedicated in-memory data structures and a planning phase.

While the engine targets RML (a superset of R2RML), its core dedup machinery is format-agnostic and directly applicable to R2RML-only engines.

---

## 2. Architecture: Two Modules

SDM-RDFizer has two pipeline stages, mirroring a query optimizer + executor split:

```
RML Triples Maps + Data Sources
        │
        ▼
 ┌─────────────────────────────┐
 │  Triples Map Planning (TMP) │   ← produces execution order + flush schedule
 └─────────────────────────────┘
        │ Ordered list of TMs
        ▼
 ┌──────────────────────────────┐
 │  Triples Map Execution (TME) │   ← DT + PTT + PJTT; emits triples
 └──────────────────────────────┘
        │
        ▼
   RDF Knowledge Graph (file/stream)
```

TME exposes three physical operators: **Simple Object Map (SOM)**, **Object Reference Map (ORM)**, and **Object Join Map (OJM)**.

---

## 3. Physical Data Structures (Core of the Contribution)

### 3.1 Dictionary Table (DT)

The DT is a global hash table that assigns a compact integer ID to every distinct RDF term (IRI or literal) seen during execution.

- **Key**: full RDF term string (e.g., `<http://example.org/Gene/ALDH3A1>`)
- **Value**: sequential integer encoded in **base 36**
- An `encode(.)` function converts any RDF resource to its corresponding integer on first seen, then all subsequent data structures reference the integer, not the string

The DT was introduced in v4.0. In v3.2 the PTT stored full URI strings as hash keys; the DT reduces the memory footprint of every PTT entry dramatically (a base-36 integer vs a 50-200 byte URI string).

### 3.2 Predicate Tuple Table (PTT)

One PTT is created per predicate `p` that appears in at least one TripleMap.

- **Implemented as**: a hash table
- **Hash key**: concatenation `encode(subject) + "_" + encode(object)` (two base-36 integers joined with underscore)
- **Value**: encoding of the full RDF triple

**Dedup algorithm** (per generated triple `t = (s, p, o)`):

```
key = encode(s) + "_" + encode(o)
if key ∈ PTT[p]:
    discard t                    # duplicate
else:
    PTT[p][key] = encode(t)      # record
    emit t to KG                 # output
```

PTTs are **flushed from main memory** by the Predicate List (see §4) as soon as no future TripleMap will generate triples for predicate `p`. This is the primary memory-reduction mechanism in v4.x.

### 3.3 Predicate Join Tuple Table (PJTT)

One PJTT is created per `(parent TM, join attribute)` pair when an OJM operator is needed.

- **Implemented as**: a nested hash table (index structure)
- **Outer key**: `parent_TM_id + "_" + join_attribute` (string)
- **Inner key**: `encode(join_attribute_value)` — the encoded value of the join condition column from the parent data source
- **Inner value**: set of `encode(subject)` of all parent rows with that join value

This makes the OJM operator an **index join**: for each child row, a single O(1) hash lookup finds all matching parent subjects. The PJTT is loaded once from the parent data source, then re-used for every child TM that references it.

In the source code (`inner_functions.py`), the global `join_table` dict maps `parent_TM_id + "_" + child_attr` to a nested dict of `{join_value: {subject_value: "object"}}`.

---

## 4. Planning Phase (TMP, v4.0+)

TMP defines two additional data structures that control execution order and memory flushing:

### 4.1 Organized Triples Maps List (OTML)

Groups TMs by their data source file, so each file is opened **exactly once** during execution. Each file's block of TMs is executed before moving on. This is the **data-driven** approach for file sources (CSV/JSON/XML); for relational databases the engine uses a **mapping-driven** approach (one SQL query per TM).

### 4.2 Predicate List (PL)

Maps each predicate to the list of TMs that still need to write triples for it. After each TM finishes:
- It is removed from the PL entry for each of its predicates
- If a predicate's PL entry becomes empty, its PTT is **flushed** from memory

PL also drives OTML ordering: TMs with the least overlapping predicates run first (to maximise the number of PTTs flushed after each TM completes). The paper describes this as reducing the "maximum amount of data kept in main memory."

---

## 5. Operator Complexity Analysis

The paper provides a formal comparison between the naive approach (merge-sort dedup) and SDM-RDFizer's hash-based approach. Let `|Np|` = total generated triples (with dups) for predicate `p`, `|Sp|` = unique triples for `p` (where `|Sp| << |Np|` in high-dup scenarios).

| Operator | Naive cost | SDM-RDFizer cost |
|---|---|---|
| **SOM** | `\|Np\| + \|Sp\| + Θ(Np log Np)` | `\|Np\| + 2\|Sp\|` |
| **ORM** | same as SOM | same as SOM |
| **OJM** | `\|Nparent\| × \|Nchild\| + \|Np\| + \|Sp\| + Θ(Np log Np)` | `2\|Nparent\| + \|Nchild\| + \|Np\| + 2\|Sp\|` |

At high duplicate rates (`|Sp| << |Np|`), the hash structures yield near-linear cost vs the naive super-linear cost. The OJM improvement is particularly large: the nested-loop join `O(|Nparent| × |Nchild|)` becomes `O(|Nparent| + |Nchild|)` via the PJTT index.

---

## 6. Performance Benchmark Results

### CIKM 2020 evaluation (v3.2, biomedical datasets, Intel Xeon E5-2603 1.6GHz, 64GB RAM)

- SOM/ORM/OJM mappings, 10K / 100K / 1M rows, 25% or 75% duplicate rates
- All engines tested 5 times; timeout = 5 hours
- **RMLMapper v4.7 and RocketRML v1.7.0**: time out on all 2-ORM and 5-ORM configurations at 1M rows (both 25% and 75% dups); RocketRML also produces incorrect OJM outputs (does not support N-M joins)
- **SDM-RDFizer v3.2**: completes all configurations; "outperforms state of the art by up to **three orders of magnitude**"

### SWJ v4.5.6 evaluation (GTFS-Madrid-Bench + SDM-Genomic-Datasets, same hardware)

Star-join motivating example (6 TMs, 5 child TMs sharing one parent, SDM-Genomic-Datasets):

| Engine | 100K rows 25% dup | 100K rows 75% dup |
|---|---|---|
| RMLMapper v6.0 | 11,961 s | 12,669 s |
| Morph-KGC v2.1.1 | 23 s | 23 s |
| SDM-RDFizer v3.2 | 79 s | 45 s |

At 5M rows: RMLMapper times out; Morph-KGC 8,092s / 7,307s (25%/75%); SDM-RDFizer v4.5.6 is the only engine that can handle all join configurations including N-M multi-join (Conf7/8/9) — Morph-KGC cannot execute these.

**Memory**: v4.5.6 with DT compression + PTT flushing shows significant reduction vs v3.2 (DT encodes URIs as short base-36 integers; each PTT entry stores two integers instead of two full URI strings). The paper states "by applying data compression, there is a significant reduction in memory consumption."

**Key competitive dynamic**: Morph-KGC wins on simple SOM/ORM cases (uses Pandas bulk `drop_duplicates()` which is highly optimised in native code); SDM-RDFizer wins on complex OJM multi-join configurations where Morph-KGC either times out or errors.

---

## 7. Conformance Relevance (W3C R2RML / RML Tests)

### KGCW 2024 (v4.7.5.x)

| Test set | Total | Passed | Failed |
|---|---|---|---|
| RML-Core (incl. R2RML-equivalent) | 238 | 238 | 0 |
| RML-FNML | 14 | 14 | 0 |
| RML-Star | 18 | 18 | 0 |
| RML-IO | 67 | 65 | 2 |
| RML-CC | 29 | 0 | 29 |
| **Total** | **366** | **335** | **31** |

### KGCW 2025 (v4.7.5.12.2)

| Test set | Total | Passed |
|---|---|---|
| RML-Core | 59 | 58 |
| RML-FNML | 17 | 16 |
| RML-Star | 18 | 18 |
| RML-IO | 73 | 64 |
| RML-IO-Registry | 103 | 70 |
| RML-CC | 35 | 35 |
| RML-LV | 32 | 32 |
| **Total** | **337** | **293** |

For a **pure R2RML engine** targeting the W3C R2RML test suite (a subset of RML-Core), SDM-RDFizer is essentially fully conformant. The failures are in RML-specific extensions (collections, remote IO registries).

---

## 8. What a Rust Engine (semantic-fabric) Should Adopt

### 8.1 Integer term encoding (DT) — adopt directly

Assign a `u64` ID to every distinct RDF term on first encounter. Store the term string once in a `HashMap<Arc<str>, u64>` (forward) and `Vec<Arc<str>>` (reverse). All dedup structures then operate on `u64` pairs, never on strings.

**Oxigraph angle**: Oxigraph's `RocksDB`/`memory` stores already encode terms as `EncodedTerm` (internally a compact integer or small-string). The semantic-fabric materialization path can reuse or shadow this encoding. The key insight from SDM-RDFizer is to maintain this encoding *in the mapping executor itself*, not just the triple store.

### 8.2 Per-predicate hash sets for dedup (PTT) — adopt directly

For each predicate (as a `u64` term ID), maintain a `HashSet<(u64, u64)>` keyed on `(subject_id, object_id)`. Before writing any triple, check membership. If absent, insert and emit.

In Rust:
```
// conceptual
type TermId = u64;
type Ptt = FxHashMap<TermId, FxHashSet<(TermId, TermId)>>;
```

Use `rustc-hash`'s `FxHashSet`/`FxHashMap` or `ahash`-backed equivalents for fast integer hashing. Avoid the Python engine's base-36 string key — in Rust, `(u64, u64)` is already a native hashable type.

### 8.3 Index join via pre-built hash map (PJTT) — adopt directly

For each R2RML `rr:joinCondition` referencing a parent TM, pre-scan the parent data source once and build:
```
// conceptual
type JoinKey = u64;   // encoded join-column value
type Pjtt = FxHashMap<JoinKey, SmallVec<[TermId; 4]>>;
```

Then process child rows: for each child row, look up `encode(child_join_value)` in the PJTT, iterate over matched parent subject IDs, generate triples, dedup via PTT. This converts a potentially O(N×M) nested-loop join to O(N+M).

Use `smallvec` (crate) for the value vectors since most join conditions are selective (small result sets per key); this avoids heap allocation in the common case.

### 8.4 PTT flushing via predicate dependency tracking (PL) — adopt

Build a dependency graph before execution: for each predicate, which future TMs (in execution order) still need to write it. After processing each TM, decrement the reference count for its predicates; when a predicate's count reaches zero, drop the PTT entry from memory. In Rust this is a simple `HashMap<TermId, usize>` (predicate → remaining writers).

### 8.5 Data-source grouping (OTML) — adapt

For file sources, group R2RML TriplesMap rules by their logical table / SQL query. When two TMs share the same base query (or the same physical table scan), run them together on a single pass through the result set. For relational databases specifically, consider merging compatible queries via SQL UNION or running multiple TMs over the same `sqlx::Row` stream.

### 8.6 What NOT to port from SDM-RDFizer

- **Python's `join_table` global dict**: thread-unsafe by design; Rust should use scoped state per mapping execution context, passed explicitly.
- **Base-36 integer encoding**: exists to produce compact strings for Python dict keys. In Rust, use `u64` values directly.
- **Full file buffering**: SDM-RDFizer reads entire CSV/DB result sets into memory before processing. A Rust engine should use row-streaming (e.g., iterate over `sqlx::Rows` or a CSV reader iterator) and process rows one at a time, maintaining only the PTT/PJTT in memory.
- **The `duplicate = "yes"` flag**: SDM-RDFizer has a config option to skip dedup. For a correctness-first engine targeting the W3C test suite, always deduplicate.

### 8.7 Ordering / planning heuristic — consider

The TMP predicate-overlap ordering heuristic is simple to implement and reduces peak memory meaningfully (especially on star-join patterns). For an initial Rust implementation, a greedy ordering (TMs with fewest shared predicates first) is worth including.

---

## 9. Open Questions

1. **PJTT memory bound**: SDM-RDFizer loads the entire parent TM result into the PJTT. For very large parent tables this is unacceptable. A Rust engine should provide a fallback to disk-based hash join (e.g., grace hash join or merge join on sorted streams) when the PJTT exceeds a configurable memory budget.
2. **PTT memory bound**: PTTs grow proportional to unique triples per predicate. In the worst case (no duplicates, huge dataset) the PTT holds every emitted triple ID. A disk-spill strategy (e.g., a sorted flat file) is needed for very large materialization jobs.
3. **Multi-threading**: SDM-RDFizer claims "multi-thread safe procedure" in its README but the code uses global dicts (`join_table`, `inner_join_table`) without locks. A Rust engine can exploit Rayon parallelism over independent TMs (different predicates / non-overlapping data sources) safely if the PTTs are partitioned by predicate.
4. **Morph-KGC's Pandas dedup**: For datasets with no join conditions, Morph-KGC's bulk `DataFrame.drop_duplicates()` outperforms SDM-RDFizer's per-row PTT check. Investigate whether a Rust vectorized approach (Arrow/DataFusion) can match this for the SOM operator.

---

## Sources

- [SDM-RDFizer CIKM 2020 paper (ACM DL)](https://dl.acm.org/doi/abs/10.1145/3340531.3412881)
- [SDM-RDFizer arXiv preprint](https://arxiv.org/pdf/2008.07176)
- [Empowering the SDM-RDFizer (SWJ, v4.5.6)](https://www.semantic-web-journal.net/system/files/swj3246.pdf)
- [GitHub: SDM-TIB/SDM-RDFizer](https://github.com/SDM-TIB/SDM-RDFizer)
- [KGCW 2024 Challenge Results: SDM-RDFizer](https://ceur-ws.org/Vol-3718/paper12.pdf)
- [KGCW 2025 Challenge Results: SDM-RDFizer](https://ceur-ws.org/Vol-3999/short3.pdf)
- [KROWN Benchmark (GitHub)](https://github.com/kg-construct/KROWN)
- [GTFS-Madrid-Bench (ScienceDirect)](https://www.sciencedirect.com/science/article/pii/S1570826820300354)
- [Morph-KGC: Scalable KG Materialization (Semantic Web Journal)](https://journals.sagepub.com/doi/10.3233/SW-223135)
