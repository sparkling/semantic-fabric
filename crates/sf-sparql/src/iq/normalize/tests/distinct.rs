use super::*;

/// Ontop `ValuesNodeOptimization::test8DistinctUnionValuesNonValues`: `Distinct`
/// over `Union[distinct-Values, ext]` is a genuine no-op — covered FOR FREE:
/// `normalize_distinct` only recognizes `Values`/`Construction{Values}` as its
/// child directly, so a `Union` child (even one with an already-duplicate-free
/// `Values` arm) correctly declines and survives untouched. Nothing to dedup
/// here that the outer `Distinct` doesn't already have to do regardless.
#[test]
fn distinct_over_union_of_already_distinct_values_and_data_is_a_no_op() {
    let n = norm(
        "SELECT DISTINCT ?n WHERE { { VALUES ?n { \"a\" \"b\" } } \
         UNION { ?s <http://ex/name> ?n } }",
    );
    assert!(
        matches!(&n, IqNode::Distinct { child } if matches!(child.as_ref(), IqNode::Union { .. })),
        "no rewrite applies -- Distinct{{Union{{..}}}} survives untouched: {n:?}"
    );
}

/// Ontop `ValuesNodeOptimization::test9DistinctUnionValuesNonValues`: `Distinct`
/// over `Union[Values(dups), ext]` dedups the `Values` arm's OWN internal
/// duplicates in place; the outer `Distinct` still survives (cross-arm dedup
/// against the data arm isn't statically provable) and the data arm itself is
/// untouched.
#[test]
fn distinct_over_union_dedups_the_values_arm_keeps_distinct_and_data_arm() {
    let n = norm(
        "SELECT DISTINCT ?n WHERE { { VALUES ?n { \"a\" \"a\" \"b\" } } \
         UNION { ?s <http://ex/name> ?n } }",
    );
    let IqNode::Distinct { child } = &n else {
        panic!("the outer Distinct must survive (cross-arm dedup isn't provable): {n:?}")
    };
    let IqNode::Union { children, .. } = child.as_ref() else {
        panic!("expected the Union to survive: {child:?}")
    };
    assert_eq!(children.len(), 2, "both arms present: {children:?}");
    // The arm keeps its ORIGINAL identity-projection Construction wrapper
    // (confirmed empirically, same pattern as every other Values-as-union-arm
    // case in this file) -- `dedup_one_arm` rebuilds it around the deduped
    // Values leaf rather than unwrapping it.
    let IqNode::Construction { child: values, .. } = &children[0] else {
        panic!("expected the FIRST child to still be Construction-wrapped: {children:?}")
    };
    let IqNode::Values { rows, .. } = values.as_ref() else {
        panic!("expected the wrapped leaf to be the (deduped) Values arm: {values:?}")
    };
    assert_eq!(
        rows.iter().map(|r| row_str(r)).collect::<Vec<_>>(),
        vec!["a", "b"],
        "the Values arm's own duplicate \"a\" is gone: {rows:?}"
    );
    assert!(
        !matches!(children[1], IqNode::Values { .. }),
        "the data arm must still be the untouched real pattern: {:?}",
        children[1]
    );
}

/// The identical narrowing-projection hazard `same_var_set` guards against for
/// the single-`Values`-child case (test3's adversarial-review-caught bug)
/// applies per-arm here too: a `Values` arm whose own columns are a
/// STRICT SUPERSET of the Union's `project` (some column is projected away
/// above this level) must decline dedup on that arm -- collapsing on the FULL
/// pre-projection tuple could wrongly merge two rows that remain genuinely
/// distinct after the (elsewhere-applied) projection.
#[test]
fn distinct_over_union_declines_a_narrowed_values_arm() {
    // The data arm only ever binds ?n, so the Union's own project is [n] even
    // though this Values arm's OWN vars are [n, extra] -- same_var_set([n],
    // [n,extra]) is false, so dedup_one_arm must leave this arm untouched.
    let n = norm(
        "SELECT DISTINCT ?n WHERE { { VALUES (?n ?extra) { (\"a\" 1) (\"a\" 2) } } \
         UNION { ?s <http://ex/name> ?n } }",
    );
    let IqNode::Distinct { child } = &n else {
        panic!("expected Distinct to survive: {n:?}")
    };
    let IqNode::Union { children, .. } = child.as_ref() else {
        panic!("expected the Union to survive: {child:?}")
    };
    let IqNode::Construction { child: values, .. } = &children[0] else {
        panic!("expected the first child to still be Construction-wrapped: {children:?}")
    };
    let IqNode::Values { rows, .. } = values.as_ref() else {
        panic!("expected the wrapped leaf to still be the Values arm: {values:?}")
    };
    assert_eq!(
        rows.len(),
        2,
        "both rows must survive untouched -- a narrowing dedup here would be \
         the exact =_bag bug an adversarial review caught on the single-arm \
         rule: {rows:?}"
    );
}

/// Ontop `ValuesNodeOptimization::test25NoVariableTrueNodesAndValuesNodes`: a
/// zero-var `Union` of bare `{}` groups (each an `IqNode::True`) folds to a
/// "counting" `Values` leaf -- zero columns, one empty-tuple row per arm.
#[test]
fn zero_var_union_of_true_arms_folds_to_counting_values() {
    let n = norm("SELECT * WHERE { {} UNION {} UNION {} }");
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper: {n:?}")
    };
    let IqNode::Values { vars, rows } = child.as_ref() else {
        panic!("expected the Union to fold to a counting Values leaf: {child:?}")
    };
    assert!(vars.is_empty(), "zero columns: {vars:?}");
    assert_eq!(rows.len(), 3, "one empty-tuple row per True arm: {rows:?}");
    assert!(
        rows.iter().all(|r| r.is_empty()),
        "every row is the empty tuple: {rows:?}"
    );
}

/// A DATA arm (a real triple pattern, not a bare constant) blocks the FULL
/// fold (`fold_constant_union`) — no `Values` leaf. With only ONE constant
/// arm here (the class-atom `?s <rdf:type> ?x` itself expands to a 2-way
/// per-table union during BUILD, so this is 1 constant + 2 data arms, not 1+1),
/// the PARTIAL fold (`fold_partial_constant_runs`, test15) declines too
/// (nothing to combine — see that function's own doc comment); see
/// `partial_fold_combines_multiple_constant_arms_keeps_data_arm` below for the
/// 2-constant-arms case that DOES partially fold.
#[test]
fn union_with_a_data_arm_does_not_fold() {
    let n = norm(&format!(
        "SELECT ?x WHERE {{ {{ BIND(\"a\" AS ?x) }} UNION {{ ?s <{RDF_TYPE}> ?x }} }}"
    ));
    assert!(
        !matches!(n, IqNode::Values { .. }),
        "a real pattern in one arm must block the constant fold: {n:?}"
    );
}

/// Ontop `ValuesNodeOptimization::test15ConstructionUnionTrueTrueDataNode`:
/// with TWO OR MORE constant arms alongside a data arm, the full fold still
/// declines (a real pattern is present), but the PARTIAL fold now combines
/// just the constant arms into one `Values`, keeping the data arm(s) as
/// sibling `Union` arms — fewer arms, same `=_bag` multiset.
///
/// The DATA arm is deliberately FIRST here (`A UNION B UNION C` is
/// left-associative — `(A UNION B) UNION C`): with the data arm LAST, the
/// inner `{BIND a} UNION {BIND b}` pair would fully fold via the PRE-EXISTING
/// `fold_constant_union` (test14, both arms constant) before this rule
/// ever runs, making the test pass regardless of whether this new function
/// exists at all — a mistake caught empirically via this test's OWN
/// revert-proof (bypassing `fold_partial_constant_runs` produced no
/// failure with the data arm last, exposing the vacuous ordering).
#[test]
fn partial_fold_combines_multiple_constant_arms_keeps_data_arm() {
    let n = norm(&format!(
        "SELECT ?x WHERE {{ {{ ?s <{RDF_TYPE}> ?x }} UNION {{ BIND(\"a\" AS ?x) }} \
         UNION {{ BIND(\"b\" AS ?x) }} }}"
    ));
    let IqNode::Union { children, .. } = &n else {
        panic!("expected a Union of [data-arm, data-arm, folded-Values]: {n:?}")
    };
    assert_eq!(
        children.len(),
        3,
        "2 constant arms fold to 1 Values arm + the class-atom's own 2-way \
         per-table data union = 3 total (down from 4): {children:?}"
    );
    // The two constant arms (BIND a, BIND b) come LAST in this query's own
    // source order (the class-atom's 2-way data union comes first), so the
    // fold lands LAST too -- never reordered relative to the data arms
    // (a real bug an adversarial review caught in an earlier version of
    // this rule: unconditionally prepending the fold silently changed which
    // rows a bare LIMIT kept relative to the flat oracle's as-written
    // order; fixed by folding each maximal contiguous run of constant arms
    // AT its own starting position instead).
    assert!(
        children[..2]
            .iter()
            .all(|c| !matches!(c, IqNode::Values { .. } | IqNode::Union { .. })),
        "the first two (data) arms are untouched, no further folding: {children:?}"
    );
    // The folded Values arrives back here wrapped in an identity Construction
    // (`lift_construction`'s catch-all re-wraps every non-Union/Construction/
    // Empty arm when the enclosing top-level query Construction re-processes
    // this already-normalized Union -- the same double-pass mechanism found
    // during the test25 investigation), not bare -- confirmed empirically, not
    // assumed.
    let IqNode::Construction { child, subst, .. } = &children[2] else {
        panic!(
            "expected the LAST child to be the folded constant Values leaf \
                 (at the position its own 2 source arms started): {children:?}"
        )
    };
    assert!(
        subst.is_empty(),
        "an identity wrapper, no bindings: {subst:?}"
    );
    let IqNode::Values { rows, .. } = child.as_ref() else {
        panic!("expected an identity-Construction-wrapped Values leaf: {child:?}")
    };
    assert_eq!(rows.len(), 2, "one row per constant arm: {rows:?}");
}

/// A variable-referencing `BIND` (not a compile-time constant) also blocks the
/// fold — `bind_term_def` against an empty bindings map cannot resolve it.
#[test]
fn union_with_a_variable_referencing_bind_does_not_fold() {
    let n = norm(&format!(
        "SELECT ?x ?y WHERE {{ {{ ?s <{RDF_TYPE}> ?y BIND(?y AS ?x) }} \
         UNION {{ BIND(\"b\" AS ?x) }} }}"
    ));
    assert!(
        !matches!(n, IqNode::Values { .. }),
        "a variable-dependent binding must block the constant fold: {n:?}"
    );
}
