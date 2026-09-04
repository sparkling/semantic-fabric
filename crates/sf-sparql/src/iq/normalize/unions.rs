use super::values::{constant_row_from_subst, constant_subst_can_form_row, same_var_set};
use crate::iq::node::{IqNode, Var};
use crate::iq::TermDef;
use crate::Result;

// ---- (c) union: flatten + prune (NO arm-merge) -----------------------------------

/// Normalize a `Union` over already-normalized `children` (design §4(c), ledger R5).
/// Flattens nested `Union`s (associativity — keeps every arm), prunes `Empty` arms
/// (the `Union` identity), and unwraps a one-arm `Union`. Beyond that it performs
/// **NO multiplicity-losing arm-merge / dedup** — a multiplicity-bearing arm is never
/// collapsed into a sibling — with exactly one exception: [`fold_constant_union`]
/// (ADR-0023 optimizer-residue Wave C, §4.15) combines arms that are ALL bare
/// constant tuples into one `Values` leaf, which is bag-preserving (N constant arms
/// → N rows), not a lossy dedup.
pub(super) fn normalize_union(children: Vec<IqNode>, project: Vec<Var>) -> Result<IqNode> {
    let mut arms: Vec<IqNode> = Vec::new();
    for c in children {
        match c {
            IqNode::Empty { .. } => {} // prune the Union identity (keep the signature)
            IqNode::Union {
                children: inner, ..
            } => arms.extend(inner), // flatten (associativity — NOT dedup)
            other => arms.push(other),
        }
    }
    Ok(match arms.len() {
        0 => IqNode::Empty { vars: project },
        1 => arms.pop().expect("len checked == 1"),
        _ if arms
            .iter()
            .all(|arm| constant_arm_is_foldable(arm, &project)) =>
        {
            fold_constant_union(arms, project)
        }
        _ => fold_partial_constant_runs(arms, project),
    })
}

/// Fold a `Union` of ALL-CONSTANT arms into one `Values` node (Ontop
/// `ValuesNodeOptimization::test14ConstructionUnionTrueTrue`): when every arm is a
/// `Construction` over `True` (i.e. genuinely data-free — a `BIND`-only row, no
/// pattern underneath) with every one of its bindings already a resolved constant,
/// the whole `Union` is exactly the literal table of those constant tuples — the
/// SAME `=_bag` multiset (one row per arm), just represented as a `Values` leaf
/// instead of an N-arm `Union` of single-row `Construction`s.
///
/// The caller preflights every arm and uses [`fold_partial_constant_runs`] instead
/// when a binding is not compile-time constant or an arm contains a real pattern.
///
/// Ontop `ValuesNodeOptimization::test25NoVariableTrueNodesAndValuesNodes`: a bare
/// `IqNode::True` arm (the zero-var identity — no `Construction` at all, since it
/// binds nothing) is ALSO foldable, contributing exactly one empty-tuple row, but
/// ONLY when `project` is itself empty (a `True` arm cannot supply a value for any
/// projected variable, so it is never a valid fold candidate otherwise). This is
/// the ONLY place `project.is_empty()` matters: every other arm shape already
/// degrades correctly on an empty `project` with no special-casing (the per-`var`
/// loop below simply doesn't execute, producing an empty row, exactly the `Values`
/// leaf's own "counting" row shape for this case).
///
/// A `BIND`-only arm's binding is still a **symbolic** `BindDef::Expr` at NORMALIZE
/// time (FILTER/BIND resolve per-leaf-CQ at LOWER, not here — confirmed empirically:
/// `{ BIND("a" AS ?x) } UNION { BIND("b" AS ?x) }` normalizes to `Union[Construction{
/// subst: {x: Expr(Literal("a"))}, child: True}, …]`, never a pre-`Resolved` constant).
/// [`crate::unify::bind_term_def`] is reused with an EMPTY bindings map to recognize a genuine
/// constant without resolving it against any column: it only ever succeeds on a bare
/// IRI/literal or a `CONCAT` of recursively-constant parts (`Expression::Variable`
/// always fails against an empty map), so a successful result is *provably*
/// column-free — safe to embed directly in a core-less `Values` row.
fn fold_constant_union(arms: Vec<IqNode>, project: Vec<Var>) -> IqNode {
    debug_assert!(arms.len() >= 2);
    let mut rows = Vec::with_capacity(arms.len());
    for arm in arms {
        rows.extend(
            into_const_rows(arm, &project).expect("constant_arm_is_foldable checked every arm"),
        );
    }
    IqNode::Values {
        vars: project,
        rows,
    }
}

/// Whether an arm can be consumed into constant rows without retaining the arm.
fn constant_arm_is_foldable(arm: &IqNode, project: &[Var]) -> bool {
    match arm {
        IqNode::Values { vars, .. } => same_var_set(vars, project),
        IqNode::Construction { child, subst, .. } if matches!(**child, IqNode::True) => {
            constant_subst_can_form_row(subst, project)
        }
        IqNode::True => project.is_empty(),
        _ => false,
    }
}

/// Consume the constant row(s) this arm contributes to a folded `Values` leaf.
fn into_const_rows(arm: IqNode, project: &[Var]) -> Option<Vec<Vec<Option<TermDef>>>> {
    match arm {
        // `A UNION B UNION C` is left-associative (`(A UNION B) UNION C`): the
        // inner `(A UNION B)` normalizes (and, when both are constant, this SAME
        // rule folds it) *before* the outer Union ever sees it, so an
        // already-folded `Values` arm must be absorbed directly, not just a
        // `Construction`. Ontop `ValuesNodeOptimization::test26MergeableCombination`:
        // the arm's own column ORDER need not match `project`'s — `same_var_set`
        // (SAME variables, order-independent) is the acceptance test, and
        // `reorder_row` permutes each row by variable NAME to `project`'s order
        // before absorbing it (a no-op permutation when the orders already agree).
        IqNode::Values { vars, rows } if same_var_set(&vars, project) => Some(
            rows.into_iter()
                .map(|row| reorder_row(vars.as_slice(), row, project))
                .collect(),
        ),
        IqNode::Construction { child, subst, .. } if matches!(child.as_ref(), IqNode::True) => {
            Some(vec![constant_row_from_subst(subst, project)?])
        }
        // test25: a bare `True` arm binds nothing, so it can only fold when
        // there is nothing TO bind -- an empty `project` -- contributing one
        // empty-tuple row (the same "counting" shape a zero-column `Values`
        // leaf already has). Revert-proof note: removing just this arm does NOT
        // change correctness -- `lift_construction`'s Construction-over-Union
        // case (this file) individually wraps every bare `True` arm in an
        // identity `Construction` before a SECOND fold pass, which the
        // `Construction{child:True,..}` case above already handles once the
        // `project.is_empty()` guard is lifted. What this arm changes is WHICH
        // of the two fold opportunities fires first: with it, the plain
        // (non-lifted) `normalize()` Union dispatch folds immediately,
        // preserving the outer identity-Construction wrapper this whole file's
        // other tests consistently expect; without it, the fold still happens
        // (via `lift_construction`'s second pass) but that path REPLACES the
        // Construction outright, leaving a bare `Values` -- an equally
        // `=_bag`-correct but inconsistent shape. Kept for that consistency,
        // not because the fold would otherwise fail.
        IqNode::True if project.is_empty() => Some(vec![Vec::new()]),
        _ => None, // a DATA arm (real pattern), or a shape not covered above
    }
}

/// Partial version of [`fold_constant_union`] (Ontop
/// `ValuesNodeOptimization::test15ConstructionUnionTrueTrueDataNode`): when SOME
/// (but not all) of a `Union`'s arms are constant, fold JUST those into one
/// `Values` arm, keeping the rest (the DATA arms — real patterns underneath)
/// untouched as sibling `Union` arms. The SAME `=_bag` multiset either way (a
/// `Union` distributes row-membership independently of how its arms are grouped);
/// only the SQL shape changes — fewer arms to plan/execute.
///
/// Returns the original owned arms when NO maximal contiguous run of 2+ constant
/// arms exists (a single isolated constant arm gains nothing from being wrapped
/// in a one-arm `Values`). The caller handles the all-constant case first with
/// [`fold_constant_union`].
///
/// Folds each maximal contiguous run of 2+ constant arms into one `Values` arm
/// AT that run's own starting position — never combining constant arms
/// separated by a DATA arm. A LIMIT without ORDER BY is implementation-defined
/// SPARQL-wise, but flat and tree are two implementations of the SAME engine
/// that must agree with EACH OTHER on which rows survive (the `=_bag` gate this
/// whole differential proves); flat iterates every arm in its own as-written
/// order, so combining two constant arms that straddle a data arm would move
/// the SECOND one's row earlier (or the data arm's rows later) relative to
/// flat — a real bug an adversarial review caught (`SELECT ?n WHERE {{data}}
/// UNION {{c1}} UNION {{c2}} LIMIT 2` returned the data arm's rows on the flat
/// side but the folded constants on the tree side, because an earlier version
/// of this function unconditionally prepended the fold regardless of position).
fn fold_partial_constant_runs(arms: Vec<IqNode>, project: Vec<Var>) -> IqNode {
    let foldable: Vec<bool> = arms
        .iter()
        .map(|arm| constant_arm_is_foldable(arm, &project))
        .collect();
    if !foldable.windows(2).any(|pair| pair[0] && pair[1]) {
        return IqNode::Union {
            children: arms,
            project,
        };
    }

    let mut children = Vec::with_capacity(arms.len());
    let mut source = arms.into_iter();
    let mut i = 0;
    while i < foldable.len() {
        if !foldable[i] {
            children.push(source.next().expect("foldability mirrors source arms"));
            i += 1;
            continue;
        }

        let run_end = foldable[i..]
            .iter()
            .position(|foldable| !foldable)
            .map_or(foldable.len(), |offset| i + offset);
        let run_len = run_end - i;
        if run_len >= 2 {
            let mut run_rows = Vec::new();
            for _ in 0..run_len {
                let arm = source.next().expect("foldability mirrors source arms");
                run_rows.extend(
                    into_const_rows(arm, &project)
                        .expect("constant_arm_is_foldable checked the run"),
                );
            }
            children.push(IqNode::Values {
                vars: project.to_vec(),
                rows: run_rows,
            });
        } else {
            children.push(source.next().expect("foldability mirrors source arms"));
        }
        i = run_end;
    }

    if children.len() == 1 {
        children.pop().expect("len checked == 1")
    } else {
        IqNode::Union { children, project }
    }
}

/// Permute one row's cells from `from_order` to `to_order` by variable NAME (both
/// lists name the SAME set of variables — the caller checks with [`same_var_set`]
/// first; a name absent from `from_order` here would panic, which never happens
/// under that precondition). A no-op permutation when the two orders already agree.
fn reorder_row(
    from_order: &[Var],
    mut row: Vec<Option<TermDef>>,
    to_order: &[Var],
) -> Vec<Option<TermDef>> {
    if from_order == to_order {
        return row;
    }
    to_order
        .iter()
        .map(|v| {
            let i = from_order
                .iter()
                .position(|w| w == v)
                .expect("same_var_set checked by the caller");
            std::mem::take(&mut row[i])
        })
        .collect()
}
