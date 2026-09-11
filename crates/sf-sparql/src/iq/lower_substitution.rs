//! Construction substitutions keep their original resolved-first, ordered fixpoint.
//! All newly controlled visits/copies/outputs share the request work identity.
use super::*;
use crate::build::control::BuildVec;
use crate::leftjoin::work::{binding_edit, push_owned};

/// Fold a `Construction` substitution into one branch's bindings (design §5
/// Construction). `Resolved(td)` inserts straight; `Expr(e)` resolves via the flat
/// [`bind_term_def`] against the now-known per-branch bindings (R4: the same fn the live
/// `Extend` arm calls, here per resulting branch). Resolved entries are folded first so a
/// `BIND` can reference a triple-resolved variable; symbolic entries then resolve in
/// dependency order (a multi-pass fixpoint so `BIND(?y:=?x) . BIND(?z:=?y)` resolves
/// regardless of the `BTreeMap` order — a still-unresolvable entry stays a sound 501).
/// Fold a `Construction`'s `subst` into one branch's bindings (design §5 Construction).
/// Returns `Ok(false)` when the fold proved the branch **unsatisfiable** (a shared-var
/// unify yielded `Empty`) so the caller drops it — mirroring the flat `merge` `None`
/// prune (`unfold.rs:1194`).
///
/// A variable the branch ALREADY binds (e.g. it joined a `Values` leaf that bound it
/// per-row, or two leaf-CQs share a constructed var) is NOT overwritten: the incoming
/// definition is **unified** against the existing one via the proven [`unify`] oracle —
/// `Sat` conds append to `where_conds` (the natural-join equality), `Empty` drops the
/// branch, `Unsupported` is a tracked sound-501. This is the same variable-by-variable
/// rule the flat [`merge`](crate::unfold) applies (`unfold.rs:1190-1201`); without it a
/// `Join(BGP, VALUES)` (or any pre-bound shared var) degenerates to a cartesian product.
pub(super) fn fold_subst(
    subst: &BTreeMap<Var, BindDef>,
    b: &mut Branch,
    mode: CompilerWorkMode<'_>,
) -> Result<bool> {
    let _span = tracing::debug_span!("sf.compiler.substitution").entered();
    let work = BuildWork::new(mode);
    work.charge(1)?;
    let mut pending = BuildVec::new(Vec::new());
    for (v, def) in subst {
        work.charge(1)?;
        match def {
            BindDef::Resolved(td) => {
                let td = match mode {
                    CompilerWorkMode::Uncontrolled => td.clone(),
                    CompilerWorkMode::Metered(cx) => cx.clone_optional_term_def(td)?,
                };
                if insert_or_unify(b, v, td, mode)? {
                    return Ok(false); // provably disjoint ⇒ drop the branch
                }
            }
            BindDef::Expr(e) => work.push(&mut pending, (v, e))?,
        }
    }
    let mut pending = pending.into_inner();
    while !pending.is_empty() {
        work.charge(1)?;
        let mut next = BuildVec::new(Vec::new());
        let mut last_err: Option<String> = None;
        let progressed_before = pending.len();
        for (v, e) in pending {
            work.charge(1)?;
            let result = match mode {
                CompilerWorkMode::Uncontrolled => bind_term_def(e, &b.bindings),
                CompilerWorkMode::Metered(cx) => cx.bind_definition(e, &b.bindings)?,
            };
            match result {
                Ok(td) => {
                    if insert_or_unify(b, v, td, mode)? {
                        return Ok(false);
                    }
                }
                Err(why) => {
                    last_err = Some(why);
                    work.push(&mut next, (v, e))?;
                }
            }
        }
        let next = next.into_inner();
        if next.len() == progressed_before {
            // A whole pass resolved nothing — the remaining entries are genuinely
            // unsupported / unbound (never silently dropped, design §5.1 R4).
            return Err(Error::Unsupported(last_err.unwrap_or_else(|| {
                "BIND expression could not be resolved at LOWER → 501".to_owned()
            })));
        }
        pending = next;
    }
    Ok(true)
}

/// Insert `td` as the branch's binding for `v`, or — when `v` is already bound —
/// **unify** the existing and incoming definitions (the flat `merge` rule, keeping the
/// existing binding and appending the equality conds). Returns `Ok(true)` iff the two
/// are provably disjoint (`Unify::Empty`), signalling the branch is unsatisfiable.
pub(super) fn insert_or_unify(
    b: &mut Branch,
    v: &Var,
    td: TermDef,
    mode: CompilerWorkMode<'_>,
) -> Result<bool> {
    let _span = tracing::debug_span!("sf.compiler.substitution_binding").entered();
    let work = BuildWork::new(mode);
    match lookup(&b.bindings, v, work)? {
        None => {
            binding_edit(mode, &b.bindings, v)?;
            b.bindings.insert(work.string(v)?, td);
            work.checkpoint()?;
            Ok(false)
        }
        Some(existing) => {
            // Clone so we can consult `b` (nullable-alias set, subplan joins) and then
            // mutate its bindings/conds below without a borrow conflict.
            let existing = match mode {
                CompilerWorkMode::Uncontrolled => existing.clone(),
                CompilerWorkMode::Metered(cx) => cx.clone_optional_term_def(existing)?,
            };
            // A shared variable may be UNBOUND when its EXISTING or incoming definition
            // reads a nullable (LEFT-JOINed) alias — a prior OPTIONAL's scan OR a
            // LEFT-JOINed SubPlan derived table (`Branch::nullable_aliases`). The plain
            // equality `unify` below then treats the unbound side as SQL NULL and DROPS the
            // row, but SPARQL compatible-merge (§18.5) KEEPS it and binds the variable from
            // the OTHER, mandatory side. This is the InnerJoin / BGP-merge entry point
            // (`IqNode::InnerJoin` folds the join's shared-var equality through the
            // `Construction` subst → here).
            let prep = crate::leftjoin::preparation::Preparation::new(b, mode)?;
            let existing_nullable = prep.nullable(&existing)?;
            let td_nullable = prep.nullable(&td)?;
            let nullable = existing_nullable || td_nullable;
            // A nullable SubPlan derived-table alias (`subplan_joins` with `left == true`,
            // ADR-0023 Item 1d) is emitted differently and the R1/R2 machinery below is NOT
            // verified for a subplan alias — keep the established sound 501 for that shape
            // (every OPTIONAL-decomposition / FILTER-EXISTS path already 501s it too).
            let subplan_nullable =
                reads_left_subplan(b, &existing, work)? || reads_left_subplan(b, &td, work)?;
            match crate::leftjoin::work::unify_terms(mode, &existing, &td)? {
                Unify::Sat(conds) => {
                    if conds.is_empty() {
                        // Identical defs (no correlating equality) — drops nothing, untouched.
                        return Ok(false);
                    }
                    if subplan_nullable {
                        return Err(Error::Unsupported(work.string(
                            "INNER JOIN correlating on a variable bound by a LEFT-JOINed \
                             SubPlan derived table (a modifier sub-SELECT attached as an \
                             OPTIONAL's right operand) is not yet supported → 501 (ADR-0023 \
                             Item 1d boundary — the plain-equality merge cannot null-safely \
                             compatible-merge it)",
                        )?));
                    }
                    if nullable {
                        // ADR-0025 Tier-1 (opts-nullability): a nullable (OPTIONAL-bound)
                        // shared var. BOTH sides nullable ⇒ the merged value genuinely
                        // depends per-row on which side is bound, needing a non-injective
                        // `COALESCE` that SQL-level DISTINCT/dedup cannot collapse (it dedups
                        // raw columns before term reconstruction). Sound 501 (ADR-0007;
                        // adversarial-review Bug B, both-nullable residual) rather than a wrong
                        // answer under DISTINCT/GROUP. Exactly-one-nullable is the common,
                        // safe case handled below.
                        if existing_nullable && td_nullable {
                            return Err(Error::Unsupported(work.string(
                                "INNER JOIN correlating on a variable bound by TWO OPTIONALs \
                                 (both sides nullable) is not yet supported → 501 (the \
                                 compatible-merge value needs a non-injective COALESCE that \
                                 SQL-level DISTINCT/dedup cannot collapse — ADR-0025)",
                            )?));
                        }
                        // R1 — `null_safe` equality (`NullSafeEq` = `a=b OR a IS NULL OR b IS
                        // NULL`) so an unbound (NULL) side is vacuously compatible, row KEPT.
                        for c in conds {
                            let c = crate::leftjoin::conditions::null_safe(c, true, mode)?;
                            push_owned(work, &mut b.where_conds, c)?;
                        }
                        // R2 — the row survives only where the two agree (R1), so the value
                        // equals the MANDATORY (non-nullable) side's raw def; use it directly
                        // — no COALESCE, so DISTINCT stays correct on the injective raw column.
                        let merged = if existing_nullable { td } else { existing };
                        binding_edit(mode, &b.bindings, v)?;
                        b.bindings.insert(work.string(v)?, merged);
                        work.checkpoint()?;
                    } else {
                        for c in conds {
                            push_owned(work, &mut b.where_conds, c)?;
                        }
                    }
                    Ok(false)
                }
                // A nullable side that is provably disjoint on VALUES can still be UNBOUND
                // (compatible) — dropping the whole branch would lose those unbound rows, and
                // the plain-equality fold cannot express "keep only the unbound-compatible
                // rows" from an `Empty` unify → sound 501 (ADR-0007) rather than silently
                // wrong. (A nullable side is a column, so `Empty` here is a rare edge.)
                Unify::Empty if nullable => Err(Error::Unsupported(work.string(
                    "INNER JOIN correlating on an OPTIONAL-bound (nullable) variable whose \
                     definitions are provably disjoint is not yet supported → 501 \
                     (compatible-merge must keep the unbound-compatible rows)",
                )?)),
                Unify::Empty => Ok(true),
                Unify::Unsupported(why) => Err(Error::Unsupported(why)),
            }
        }
    }
}

/// Whether `def` reads a LEFT-JOINed SubPlan derived-table alias of `b` (`subplan_joins`
/// with `left == true`) — i.e. a value that may be UNBOUND when the derived-table LEFT
/// JOIN finds no match. `TermDef::columns` recurses through `Coalesce`/`Concat`, so a
/// composite binding over such an alias is caught too. Empty (a no-op) whenever `b`
/// carries no `left == true` SubPlan — the flat path and every non-subplan branch.
pub(super) fn reads_left_subplan(b: &Branch, def: &TermDef, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    let alias = |id| -> Result<bool> {
        for sp in &b.subplan_joins {
            work.charge(1)?;
            if sp.left && sp.alias == id {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let map = |tm: &sf_core::ir::TermMap, id| -> Result<bool> {
        work.charge(1)?;
        match tm {
            sf_core::ir::TermMap::Constant(_) => Ok(false),
            sf_core::ir::TermMap::Column(_, _) => alias(id),
            sf_core::ir::TermMap::Template(t, _) => {
                for s in t.segments() {
                    work.charge(1)?;
                    if matches!(s, sf_core::ir::Segment::Column(_)) && alias(id)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    };
    match def {
        TermDef::Const(_) => Ok(false),
        TermDef::Derived { term_map, alias } => map(term_map, *alias),
        TermDef::R2rmlBlank {
            term_map,
            alias,
            graph,
        } => {
            if map(term_map, *alias)? {
                return Ok(true);
            }
            work.charge(1)?;
            match graph {
                R2rmlGraphScope::Default => Ok(false),
                R2rmlGraphScope::Mapped { term_map, alias } => map(term_map, *alias),
            }
        }
        TermDef::Coalesce(a, b2) => {
            Ok(reads_left_subplan(b, a, work)? || reads_left_subplan(b, b2, work)?)
        }
        TermDef::Concat(parts) => {
            for part in parts {
                if reads_left_subplan(b, part, work)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        TermDef::Agg { col, .. } => alias(col.alias),
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => Ok(reads_left_subplan(b, subject, work)?
            || reads_left_subplan(b, predicate, work)?
            || reads_left_subplan(b, object, work)?),
    }
}

fn lookup<'a>(
    bindings: &'a BTreeMap<String, TermDef>,
    name: &str,
    work: BuildWork<'_>,
) -> Result<Option<&'a TermDef>> {
    work.charge(1)?;
    for (key, value) in bindings {
        work.charge(1)?;
        work.charge(key.len().min(name.len()))?;
        match key.as_str().cmp(name) {
            std::cmp::Ordering::Equal => return Ok(Some(value)),
            std::cmp::Ordering::Greater => break,
            std::cmp::Ordering::Less => (),
        }
    }
    Ok(None)
}
