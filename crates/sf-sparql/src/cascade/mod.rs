//! The optimizer cascade (ADR-0007 §Decision Outcome) — semantics-preserving
//! IQ→IQ rewrites over the unfolded base translation. **The order is
//! load-bearing** and runs after tier-0 elimination:
//!
//! 0. **tier-0 elimination** (up front) — a `refObjectMap` with no join inlines
//!    the parent IRI; a parent==child self-join on a PK collapses to a scan.
//! 1. **IRI-template-mismatch pruning** — drop branches whose key/constant
//!    equalities are provably unsatisfiable. *Must precede (2)* so empty branches
//!    are gone and the IRI-term equalities that license a self-join merge are
//!    established first.
//! 2. **self-join / self-left-join elimination** — merge two scans of the same
//!    table joined on a unique key (the same row); the left-join variant
//!    preserves the right-bound provenance.
//! 3. **functional-dependency inference** (transitive closure, through unions) —
//!    *must precede (4)*: eliminating a join is sound only once uniqueness **and**
//!    match-guarantee hold.
//! 4. **FK/PK join elimination** — drop a parent scan reached only for its PK via
//!    a NOT-NULL FK (referential integrity guarantees the match).
//! 5. **selection pushdown** — push FILTERs toward the scans.
//! 6. **distinct removal** — drop a DISTINCT already implied by a projected key.
//!
//! Every rule preserves `=_bag` w.r.t. the base translation and **fires only when
//! its integrity-constraint precondition is already established**. Where a
//! precondition cannot be proven (e.g. no schema is supplied), the pass is a
//! sound no-op — never an unsound transform (ADR-0007 *cascade invariants*).
//! All six passes now **fire when their precondition holds** (and are sound
//! no-ops otherwise): (3) derives a transitive-closure FD set, (4) drops a
//! parent scan reached only for its PK via a NOT-NULL FK, (5) hoists single-scan
//! selections toward their scans, (6) drops a DISTINCT implied by a projected
//! unique key. This is the Path-B engine-perf work (ADR-0013) under the hard
//! invariant that cost may choose only among `=_bag`-equivalent plans.

use sf_core::ir::LogicalSource;
use sf_sql::TableSchema;

use crate::iq::{collect_cond_cols, Branch, CmpOp, ColRef, SqlCond, TermDef};

mod joinelim;
#[cfg(test)]
mod tests;
/// WS-G — Ontop-parity oracle (ADR-0022): GREEN parity ports + `#[ignore]` WS-A specs.
#[cfg(test)]
mod ws_g;

/// Context the constraint-driven passes need beyond the per-branch shape:
/// whether a `DISTINCT` is requested and which variables the query projects
/// (resolved at the plan layer — pass (6) proves a projected key makes the
/// DISTINCT redundant). `project == None` ⇒ `SELECT *` / CONSTRUCT: every
/// binding is projected.
#[derive(Debug, Default, Clone, Copy)]
pub struct CascadeCtx<'a> {
    pub distinct: bool,
    pub project: Option<&'a [String]>,
}

/// Run the cascade over a bag-union of branches in the load-bearing order.
/// `schema` (by table name) is the catalog read that gates the constraint-driven
/// passes; an empty slice makes (2)–(4)/(6) sound no-ops. Pass (6) inspects
/// `ctx` and, for the single-branch case, records its DISTINCT decision on the
/// returned branch's [`Branch::distinct`] flag.
pub fn run(branches: Vec<Branch>, schema: &[TableSchema], ctx: &CascadeCtx) -> Vec<Branch> {
    let mut out: Vec<Branch> = branches
        .into_iter()
        .filter_map(|mut b| {
            // A recursive property-path closure has no base scans to rewrite — the
            // constraint-driven passes are inapplicable; pass it through untouched.
            // A MINUS anti-join branch likewise carries a correlated `NotExists`
            // whose subquery scans the constraint passes do not model; pass it
            // through so a self-join/FK rewrite never silently corrupts the
            // correlation (the anti-join is already a sound base translation).
            // A GROUP BY + aggregates branch (SPARQL §11) carries an `Aggregation`
            // over its inner FROM/WHERE; the constraint-driven passes model neither
            // grouping nor aggregate columns, so pass it through untouched (a sound
            // base translation already — never risk corrupting the grouping).
            if b.path.is_some() || b.agg.is_some() || branch_has_not_exists(&b) {
                return Some(b);
            }
            tier0_eliminate(&mut b, schema); // 0
            if !prune_iri_template_mismatch(&b) {
                return None; // 1 — unsatisfiable branch
            }
            self_join_elimination(&mut b, schema); // 2 (inner-join variant)
            self_left_join_elimination(&mut b, schema); // 2 (left-join variant — Q5)
            let fds = infer_functional_dependencies(&b, schema); // 3
            joinelim::fk_pk_join_elimination(&mut b, schema, &fds); // 4
            selection_pushdown(&mut b); // 5
            Some(b)
        })
        .collect();
    // 6 — distinct removal. For a multi-branch bag union, DISTINCT cannot be proved
    // redundant per-branch, so it is *enforced* by deduplicating projected solutions
    // in `exec::for_each_solution` (the streaming core); this pass only *removes* a
    // DISTINCT a single branch's projected key already makes redundant (pushed into
    // the SQL). So the proof here applies only to the single-branch case.
    if ctx.distinct && out.len() == 1 && out[0].path.is_none() && out[0].agg.is_none() {
        out[0].distinct = true;
        distinct_removal(&mut out[0], schema, ctx.project);
    }
    out
}

/// Whether a branch carries a MINUS anti-join (`NotExists`) anywhere in its
/// `where_conds` — such branches bypass the constraint-driven cascade passes.
fn branch_has_not_exists(b: &Branch) -> bool {
    fn has(c: &SqlCond) -> bool {
        match c {
            SqlCond::NotExists { .. } => true,
            SqlCond::Not(c) => has(c),
            SqlCond::And(cs) | SqlCond::Or(cs) => cs.iter().any(has),
            _ => false,
        }
    }
    b.where_conds.iter().any(has)
}

// --- 0. tier-0 elimination -------------------------------------------------

/// Tier-0 (ADR-0007 step 4). The no-join refObjectMap inlining is already done
/// during unfold (the parent subject is built from the child row); the
/// parent==child PK self-join collapse is handled by pass (2). Kept as an
/// explicit, documented stage so the pipeline shape matches the ADR.
fn tier0_eliminate(_b: &mut Branch, _schema: &[TableSchema]) {}

// --- 1. IRI-template-mismatch pruning -------------------------------------

/// Returns `false` if the branch is provably empty: a column is constrained by
/// two `=` constants with different values (the algebra-level disjointness the
/// ADR calls IRI-template-mismatch). Sound: such a branch yields no rows.
fn prune_iri_template_mismatch(b: &Branch) -> bool {
    let mut eqs: Vec<(&ColRef, &str)> = Vec::new();
    for cond in &b.where_conds {
        if let SqlCond::Cmp(col, CmpOp::Eq, val) = cond {
            if let Some((_, prev)) = eqs.iter().find(|(c, _)| *c == col) {
                if *prev != val.as_str() {
                    return false;
                }
            } else {
                eqs.push((col, val));
            }
        }
    }
    true
}

// --- 2. self-join / self-left-join elimination ----------------------------

/// Merge two core scans of the **same table** joined on a **non-nullable unique
/// key** (equal key value ⇒ same row, a 1:1 match → `=_bag` preserved). Fires only
/// when the schema proves the equated column is a single-column unique key **that
/// is NOT NULL** (ADR-0007: a nullable unique column is *not* a true key — the
/// `NULL = NULL ⇒ UNKNOWN` join already excludes its NULL rows, so collapsing to a
/// bare scan would re-admit them and break `=_bag`). Otherwise a no-op.
fn self_join_elimination(b: &mut Branch, schema: &[TableSchema]) {
    while let Some((keep, drop, cond_idx)) = find_self_join(b, schema) {
        // Remove exactly the key equality that licenses *this* merge (by index,
        // before the rewrite). Removing every trivial `x = x` would also drop a
        // genuine `?x :p ?x` self-comparison, which is an effective `IS NOT NULL`
        // guard and must survive (ADR-0007 R3/=_bag).
        b.where_conds.remove(cond_idx);
        rewrite_alias(b, drop, keep);
        b.core.retain(|s| s.alias != drop);
    }
}

/// Returns `(keep_alias, drop_alias, where_cond_index)` of an eliminable
/// self-join, or `None`.
fn find_self_join(b: &Branch, schema: &[TableSchema]) -> Option<(usize, usize, usize)> {
    for (idx, cond) in b.where_conds.iter().enumerate() {
        let SqlCond::ColEq(a, c) = cond else { continue };
        if a.alias == c.alias || a.column != c.column {
            continue; // a same-alias `?x :p ?x` guard is not a self-join
        }
        let ta = scan_table(b, a.alias)?;
        let tc = scan_table(b, c.alias)?;
        if ta != tc {
            continue;
        }
        if let Some(t) = schema.iter().find(|t| t.name == ta) {
            if t.is_unique_key(&a.column) && key_is_non_null(t, &a.column) {
                // Keep the lower alias, drop the higher.
                let (keep, drop) = if a.alias < c.alias {
                    (a.alias, c.alias)
                } else {
                    (c.alias, a.alias)
                };
                return Some((keep, drop, idx));
            }
        }
    }
    None
}

/// Collapse a self-**LEFT**-join — an `OPTIONAL` right side that is a provable 1:1
/// match of a kept core scan — by rebinding it onto that scan and dropping the
/// `LEFT JOIN` (the Q5 `?t a :Trip . OPTIONAL { ?t :headsign ?hs }` fix). Unlike
/// pass (2)'s inner-join variant the redundant side lives in [`Branch::opts`], not
/// `where_conds`, so this scans `opts`. Loops to a fixpoint to handle several
/// eliminable OPTIONALs; a no-op when no precondition holds.
///
/// `=_bag`-safe: the OPTIONAL reads the SAME base table as a kept core scan, joined
/// on a SINGLE shared NON-NULL unique key. Equal NON-NULL key value ⇒ the optional
/// row *is* the core row (exactly one, by uniqueness; the null-safe `ON`'s `IS NULL`
/// disjuncts are dead for a NOT-NULL key). The LEFT JOIN therefore always matches
/// 1:1 and only contributes columns of the already-read row — eliminating it
/// preserves multiplicities and every binding value. A nullable unique determinant
/// is *not* a true key (the null-safe `ON` would admit NULL rows the bare scan
/// re-reads differently → `=_bag` break), so the pass refuses (ADR-0007).
fn self_left_join_elimination(b: &mut Branch, schema: &[TableSchema]) {
    while let Some((keep, opt_alias, opt_idx)) = find_self_left_join(b, schema) {
        // The ON lived on the OptJoin, never in `where_conds`; remove the whole
        // OptJoin, then rebind every reference to the optional scan onto the kept
        // scan (rewrite_alias recurses through Coalesce bindings).
        b.opts.remove(opt_idx);
        rewrite_alias(b, opt_alias, keep);
    }
}

/// Returns `(keep_alias, opt_alias, opt_index)` of an eliminable self-left-join, or
/// `None`. Fires only when ALL hold (else a sound no-op): the OptJoin right side is
/// a single base-table scan whose table a core scan also reads; `on` is EXACTLY one
/// shared-key compatibility condition (`NullSafeEq`/`ColEq`, same column on both
/// sides, one side the kept scan and the other the optional scan); that column is a
/// NON-NULL single-column unique key; and `extra` is empty (a FILTER inside the
/// OPTIONAL makes the match conditional → not always-matching → not eliminable).
fn find_self_left_join(b: &Branch, schema: &[TableSchema]) -> Option<(usize, usize, usize)> {
    for (idx, opt) in b.opts.iter().enumerate() {
        // A FILTER inside the OPTIONAL can make the match conditional → keep it.
        if !opt.extra.is_empty() {
            continue;
        }
        // The right side must be a single base-table scan.
        let LogicalSource::Table(opt_table) = &opt.scan.source else {
            continue;
        };
        let opt_alias = opt.scan.alias;
        // Exactly one shared-key compatibility condition, same column on both sides.
        let [cond] = opt.on.as_slice() else { continue };
        let (SqlCond::NullSafeEq(a, c) | SqlCond::ColEq(a, c)) = cond else {
            continue;
        };
        if a.column != c.column {
            continue;
        }
        // One side on the optional scan, the other on a kept core scan.
        let keep = if a.alias == opt_alias && c.alias != opt_alias {
            c
        } else if c.alias == opt_alias && a.alias != opt_alias {
            a
        } else {
            continue;
        };
        // The kept side must be a core scan reading the SAME table.
        if scan_table(b, keep.alias).as_deref() != Some(opt_table.as_str()) {
            continue;
        }
        // The shared column must be a NON-NULL single-column unique key.
        if let Some(t) = schema.iter().find(|t| &t.name == opt_table) {
            if t.is_unique_key(&keep.column) && key_is_non_null(t, &keep.column) {
                return Some((keep.alias, opt_alias, idx));
            }
        }
    }
    None
}

/// A key column is non-NULL iff it is a primary-key column (PK ⇒ NOT NULL by SQL
/// semantics) or the catalog declares the column `NOT NULL`. A nullable UNIQUE
/// column is therefore *not* treated as a safe self-join determinant (ADR-0007).
fn key_is_non_null(t: &TableSchema, col: &str) -> bool {
    let is_pk = t.primary_key.iter().any(|c| c == col);
    is_pk || t.column(col).is_some_and(|c| c.not_null)
}

/// The table name a scan reads, if it is a base table (`rr:tableName`).
fn scan_table(b: &Branch, alias: usize) -> Option<String> {
    b.core
        .iter()
        .find(|s| s.alias == alias)
        .and_then(|s| match &s.source {
            LogicalSource::Table(t) => Some(t.clone()),
            LogicalSource::Query(_) => None,
        })
}

/// Rewrite every reference to alias `from` → `to` (bindings, conditions, opts).
fn rewrite_alias(b: &mut Branch, from: usize, to: usize) {
    let fix = |c: &mut ColRef| {
        if c.alias == from {
            c.alias = to;
        }
    };
    for def in b.bindings.values_mut() {
        rewrite_def_alias(def, from, to);
    }
    for cond in &mut b.where_conds {
        rewrite_cond_alias(cond, &fix);
    }
    for opt in &mut b.opts {
        for cond in opt.on.iter_mut().chain(opt.extra.iter_mut()) {
            rewrite_cond_alias(cond, &fix);
        }
    }
}

/// Rewrite the scan alias inside a term def (recursing through a `Coalesce`).
fn rewrite_def_alias(def: &mut TermDef, from: usize, to: usize) {
    match def {
        TermDef::Const(_) => {}
        TermDef::Derived { alias, .. } => {
            if *alias == from {
                *alias = to;
            }
        }
        TermDef::Coalesce(l, r) => {
            rewrite_def_alias(l, from, to);
            rewrite_def_alias(r, from, to);
        }
        TermDef::Concat(parts) => {
            for p in parts {
                rewrite_def_alias(p, from, to);
            }
        }
        TermDef::Agg { col, .. } => {
            if col.alias == from {
                col.alias = to;
            }
        }
    }
}

fn rewrite_cond_alias(cond: &mut SqlCond, fix: &impl Fn(&mut ColRef)) {
    match cond {
        SqlCond::ColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
            fix(a);
            fix(b);
        }
        SqlCond::Cmp(a, _, _) | SqlCond::IsNotNull(a) | SqlCond::IsNull(a) => fix(a),
        SqlCond::StrMatch { col, .. } => fix(col),
        SqlCond::Not(c) => rewrite_cond_alias(c, fix),
        SqlCond::And(cs) | SqlCond::Or(cs) => {
            for c in cs {
                rewrite_cond_alias(c, fix);
            }
        }
        // A `NotExists` correlates on outer (left) aliases, which a self-join merge
        // may rename; recurse so those references track the kept alias. (Inner scan
        // aliases are globally unique and never a merge target.)
        SqlCond::NotExists { conds, .. } => {
            for c in conds {
                rewrite_cond_alias(c, fix);
            }
        }
    }
}

// --- 3. functional-dependency inference -----------------------------------

/// The functional dependencies that hold over a branch's row stream. An entry
/// `(det, alias)` means **`det` determines every column of scan `alias`** (a
/// superkey of that scan's projection). Built for pass (4): a join may be
/// eliminated only once uniqueness is proven, and uniqueness is exactly "the
/// join column is a key" — an FD whose determinant is that column.
#[derive(Debug, Default)]
pub struct Fds {
    /// `(det, alias)`: `det` functionally determines all columns of `alias`.
    deps: Vec<(ColRef, usize)>,
}

impl Fds {
    fn has(&self, det: &ColRef, alias: usize) -> bool {
        self.deps.iter().any(|(d, a)| d == det && *a == alias)
    }

    fn add(&mut self, det: ColRef, alias: usize) -> bool {
        if self.has(&det, alias) {
            false
        } else {
            self.deps.push((det, alias));
            true
        }
    }

    /// Is `c` a key — does it determine its own scan's whole row? This is the
    /// uniqueness precondition pass (4) consults.
    pub fn is_key(&self, c: &ColRef) -> bool {
        self.has(c, c.alias)
    }
}

/// Derive the FD set with its **transitive closure** (ADR-0007 step iii — "FD
/// inference, transitive closure, through unions, *must precede* FK/PK join
/// elimination"). Seeds each single-column unique key as a key→row FD, then
/// closes to a fixpoint under two sound rules:
///
/// * **equality** — for a core key equality `ColEq(a, b)` (`a` and `b` hold the
///   same value on every surviving row), anything `a` determines `b` also
///   determines, and vice-versa.
/// * **transitivity** — if `x` determines all of scan `m` and a column `y` of
///   `m` determines scan `n`, then `x` determines `n`.
///
/// "Through unions" is honoured at the branch granularity: each UNION arm is a
/// separate [`Branch`], so its FDs are inferred independently and a join is
/// eliminated per-arm only on that arm's proven keys.
fn infer_functional_dependencies(b: &Branch, schema: &[TableSchema]) -> Fds {
    let mut fds = Fds::default();
    // Seed: every single-column unique key (PK or UNIQUE) determines its row.
    for scan in &b.core {
        if let LogicalSource::Table(t) = &scan.source {
            if let Some(ts) = schema.iter().find(|s| &s.name == t) {
                for col in single_col_keys(ts) {
                    fds.add(ColRef::new(scan.alias, col), scan.alias);
                }
            }
        }
    }
    // Closure to a fixpoint.
    loop {
        let mut changed = false;
        for cond in &b.where_conds {
            if let SqlCond::ColEq(a, c) = cond {
                changed |= propagate_eq(&mut fds, a, c);
                changed |= propagate_eq(&mut fds, c, a);
            }
        }
        // transitivity — snapshot the current edges to avoid borrow conflicts.
        let snapshot: Vec<(ColRef, usize)> = fds.deps.clone();
        for (x, m) in &snapshot {
            for (y, n) in &snapshot {
                if y.alias == *m && fds.add(x.clone(), *n) {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    fds
}

/// Equality rule: given `ColEq(from, to)` (equal values), copy every FD whose
/// determinant is `from` onto `to`. Returns whether anything was added.
fn propagate_eq(fds: &mut Fds, from: &ColRef, to: &ColRef) -> bool {
    let targets: Vec<usize> = fds
        .deps
        .iter()
        .filter(|(d, _)| d == from)
        .map(|(_, a)| *a)
        .collect();
    let mut changed = false;
    for a in targets {
        changed |= fds.add(to.clone(), a);
    }
    changed
}

/// The single-column unique keys of a table (the determinants that fix a row):
/// a single-column primary key, plus any single-column `UNIQUE` constraint.
fn single_col_keys(ts: &TableSchema) -> Vec<String> {
    let mut keys = Vec::new();
    if ts.primary_key.len() == 1 {
        keys.push(ts.primary_key[0].clone());
    }
    for u in &ts.unique {
        if u.len() == 1 && !keys.contains(&u[0]) {
            keys.push(u[0].clone());
        }
    }
    keys
}

// --- 5/6 + helpers live below; pass 4 is in `joinelim`. -------------------

// --- 5. selection pushdown -------------------------------------------------

/// Push FILTERs toward their scans at the algebra level (the source optimizer
/// then does the *physical* pushdown — ADR-0006: the source DB does the
/// set-work). Two semantics-preserving steps: flatten nested `AND`s so every
/// conjunct is an independent, pushable predicate; then **stable-hoist**
/// single-scan selections (`col <op> ?`, `IS [NOT] NULL`, a `?x :p ?x` self
/// guard) ahead of the multi-scan join equalities, so each scan's restriction
/// sits next to it in the emitted conjunction. A conjunction is commutative, so
/// reordering preserves `=_bag`; R5 is respected because OPTIONAL `ON`/`extra`
/// conditions live on [`crate::iq::OptJoin`], never in `where_conds` — an outer
/// FILTER is never pushed onto the preserved side.
fn selection_pushdown(b: &mut Branch) {
    let mut flat = Vec::new();
    for cond in std::mem::take(&mut b.where_conds) {
        flatten_and(cond, &mut flat);
    }
    // Stable partition: single-scan selections first, joins after.
    let (mut sels, joins): (Vec<SqlCond>, Vec<SqlCond>) =
        flat.into_iter().partition(is_single_scan_selection);
    sels.extend(joins);
    b.where_conds = sels;
}

fn flatten_and(cond: SqlCond, out: &mut Vec<SqlCond>) {
    match cond {
        SqlCond::And(cs) => {
            for c in cs {
                flatten_and(c, out);
            }
        }
        other => out.push(other),
    }
}

/// A predicate over the columns of a single scan (a restriction the source can
/// push to that scan), as opposed to a cross-scan join equality.
fn is_single_scan_selection(cond: &SqlCond) -> bool {
    let mut aliases = Vec::new();
    collect_cond_cols(cond, &mut |c| {
        if !aliases.contains(&c.alias) {
            aliases.push(c.alias);
        }
    });
    aliases.len() <= 1
}

// --- 6. distinct removal ---------------------------------------------------

/// Drop a `DISTINCT` already implied by a **projected unique key** (DISTINCT over
/// a key is a no-op — R4: never *add* DISTINCT, only remove a provably redundant
/// one). Sound proof: the branch is a single base-table scan with no OPTIONAL, so
/// output rows are a subset of source rows (FILTERs only remove); the scan has a
/// single-column unique key `K`; and a **projected** variable's term is built
/// from `K`, so distinct source rows (distinct `K`) yield distinct solution
/// tuples — the DISTINCT removes nothing. `project == None` ⇒ every binding is
/// projected (`SELECT *` / CONSTRUCT). Any join/OPTIONAL ⇒ a non-key projection
/// could hide duplicates ⇒ no-op.
fn distinct_removal(b: &mut Branch, schema: &[TableSchema], project: Option<&[String]>) {
    if !b.distinct || b.core.len() != 1 || !b.opts.is_empty() {
        return;
    }
    let scan = &b.core[0];
    let LogicalSource::Table(table) = &scan.source else {
        return;
    };
    let Some(ts) = schema.iter().find(|t| &t.name == table) else {
        return;
    };
    // Only a NOT-NULL single-column key proves DISTINCT redundant: a nullable
    // UNIQUE column permits multiple NULL rows (SQL UNIQUE allows many NULLs), and
    // build_term emits an unbound solution per NULL row — so `SELECT K` keeps both
    // NULL rows while `SELECT DISTINCT K` collapses them. Dropping the DISTINCT
    // would then ADD a row vs the base (=_bag broken). Mirror pass (2)'s
    // `key_is_non_null` guard (ADR-0007 cascade invariants).
    let keys: Vec<String> = single_col_keys(ts)
        .into_iter()
        .filter(|k| key_is_non_null(ts, k))
        .collect();
    let projected = |var: &str| project.is_none_or(|p| p.iter().any(|v| v == var));
    let redundant = b.bindings.iter().any(|(var, def)| {
        projected(var)
            && keys
                .iter()
                .any(|k| def.columns().contains(&ColRef::new(scan.alias, k.clone())))
    });
    if redundant {
        b.distinct = false;
    }
}

/// Columns referenced by a branch's conditions (test/diagnostic helper).
pub fn condition_columns(b: &Branch) -> Vec<ColRef> {
    let mut cols = Vec::new();
    for cond in &b.where_conds {
        collect_cond_cols(cond, &mut |c| {
            if !cols.contains(c) {
                cols.push(c.clone());
            }
        });
    }
    cols
}
