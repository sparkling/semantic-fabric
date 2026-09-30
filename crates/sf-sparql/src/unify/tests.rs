use super::*;
use sf_core::ir::{Template, TermSpec};
use spargebra::term::{Literal, Variable};

fn col_binding(var: &str, col: &str) -> BTreeMap<String, TermDef> {
    let mut m = BTreeMap::new();
    m.insert(
        var.to_owned(),
        TermDef::Derived {
            term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
            alias: 0,
        },
    );
    m
}

fn const_binding(var: &str, term: Term) -> BTreeMap<String, TermDef> {
    let mut m = BTreeMap::new();
    m.insert(var.to_owned(), TermDef::Const(term));
    m
}

fn template_binding(var: &str, template: &str, alias: usize) -> BTreeMap<String, TermDef> {
    let mut m = BTreeMap::new();
    m.insert(
        var.to_owned(),
        TermDef::Derived {
            term_map: TermMap::Template(Template::parse(template).unwrap(), TermSpec::iri()),
            alias,
        },
    );
    m
}

fn var(v: &str) -> Expression {
    Expression::Variable(Variable::new(v).unwrap())
}

fn lit(s: &str) -> Expression {
    Expression::Literal(Literal::new_simple_literal(s))
}

fn func(f: Function, args: Vec<Expression>) -> Expression {
    Expression::FunctionCall(f, args)
}

/// On PostgreSQL (case-sensitive `LIKE`), CONTAINS/STRSTARTS/STRENDS lower to a
/// `LIKE` whose pattern is a **bound parameter** (never SQL text), with `%`/`_`
/// metachars escaped (ADR-0020 §2).
#[test]
fn string_filters_lower_to_bound_like_with_wildcards_and_escaping() {
    let b = col_binding("x", "name");
    // CONTAINS with a literal `%` ⇒ escaped middle, wildcard-wrapped.
    let c = filter_cond(
        &func(Function::Contains, vec![var("x"), lit("a%b")]),
        &b,
        Dialect::Postgres,
    )
    .unwrap();
    assert!(
        matches!(&c, SqlCond::StrMatch { op: StrMatchOp::Like, param, col }
            if param == "%a\\%b%" && &*col.column == "name"),
        "{c:?}"
    );
    // STRSTARTS ⇒ anchored prefix.
    let s = filter_cond(
        &func(Function::StrStarts, vec![var("x"), lit("foo")]),
        &b,
        Dialect::Postgres,
    )
    .unwrap();
    assert!(
        matches!(&s, SqlCond::StrMatch { op: StrMatchOp::Like, param, .. } if param == "foo%"),
        "{s:?}"
    );
    // STRENDS ⇒ anchored suffix; underscore metachar escaped.
    let e = filter_cond(
        &func(Function::StrEnds, vec![var("x"), lit("b_r")]),
        &b,
        Dialect::Postgres,
    )
    .unwrap();
    assert!(
        matches!(&e, SqlCond::StrMatch { op: StrMatchOp::Like, param, .. } if param == "%b\\_r"),
        "{e:?}"
    );
}

/// SPARQL CONTAINS/STRSTARTS/STRENDS are case-SENSITIVE, but SQLite `LIKE` is
/// ASCII-case-insensitive (and MySQL's default collations too). On those
/// dialects the lowering MUST decline (Unsupported → the FILTER falls back,
/// un-rewritten) rather than emit a case-folding `LIKE` that would return more
/// rows than SPARQL semantics (an unsound =_bag / NoREC divergence).
#[test]
fn case_sensitive_string_filters_fall_back_on_case_insensitive_dialects() {
    let b = col_binding("x", "name");
    for f in [Function::Contains, Function::StrStarts, Function::StrEnds] {
        for d in [Dialect::Sqlite, Dialect::MySql] {
            let r = filter_cond(&func(f.clone(), vec![var("x"), lit("foo")]), &b, d);
            assert!(
                r.is_err(),
                "{f:?} on {d:?} must not lower to a case-folding LIKE: {r:?}"
            );
        }
    }
}

/// A non-column-backed var (a constructed IRI template) is NOT rewritten — the
/// FILTER falls through to unsupported, never a wrong LIKE (ADR-0020 §2 rule 3).
#[test]
fn string_filter_over_constructed_term_is_not_rewritten() {
    let mut b = BTreeMap::new();
    b.insert(
        "x".to_owned(),
        TermDef::Derived {
            term_map: TermMap::Template(
                Template::parse("http://ex/{id}").unwrap(),
                TermSpec::iri(),
            ),
            alias: 0,
        },
    );
    let r = filter_cond(
        &func(Function::Contains, vec![var("x"), lit("z")]),
        &b,
        Dialect::Sqlite,
    );
    assert!(
        r.is_err(),
        "constructed-term CONTAINS must not lower to LIKE: {r:?}"
    );
}

/// REGEX is dialect-split: PostgreSQL `~`/`~*`; SQLite has no regex operator.
#[test]
fn regex_is_dialect_split() {
    let b = col_binding("x", "name");
    let pg = filter_cond(
        &func(Function::Regex, vec![var("x"), lit("^a.*")]),
        &b,
        Dialect::Postgres,
    )
    .unwrap();
    assert!(
        matches!(&pg, SqlCond::StrMatch { op: StrMatchOp::RegexMatch, param, .. } if param == "^a.*"),
        "{pg:?}"
    );
    // The `i` flag ⇒ case-insensitive `~*`.
    let pgi = filter_cond(
        &func(Function::Regex, vec![var("x"), lit("^a.*"), lit("i")]),
        &b,
        Dialect::Postgres,
    )
    .unwrap();
    assert!(
        matches!(
            &pgi,
            SqlCond::StrMatch {
                op: StrMatchOp::RegexMatchI,
                ..
            }
        ),
        "{pgi:?}"
    );
    // SQLite: unsupported (not silently dropped).
    assert!(filter_cond(
        &func(Function::Regex, vec![var("x"), lit("^a.*")]),
        &b,
        Dialect::Sqlite
    )
    .is_err());
}

/// ADR-0032 D3: a constant `xsd:boolean` literal FILTER expression
/// (`star::rewrite_expr`'s isTRIPLE / one-side-composed-equality output)
/// lowers to the pre-existing empty-And/Or sentinels.
#[test]
fn boolean_literal_filter_lowers_to_the_constant_true_false_sentinels() {
    let bool_lit = |v: &str| {
        Expression::Literal(Literal::new_typed_literal(
            v,
            sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#boolean"),
        ))
    };
    let b = BTreeMap::new();
    assert!(matches!(
        filter_cond(&bool_lit("true"), &b, Dialect::Sqlite),
        Ok(SqlCond::And(v)) if v.is_empty()
    ));
    assert!(matches!(
        filter_cond(&bool_lit("false"), &b, Dialect::Sqlite),
        Ok(SqlCond::Or(v)) if v.is_empty()
    ));
    // A non-boolean bare literal is NOT a FILTER expression in v1
    // (unrelated to ADR-0032 — a pre-existing gap, still 501s).
    assert!(filter_cond(&lit("plain"), &b, Dialect::Sqlite).is_err());
}

/// ADR-0032 D3 item 3: `star::error_marker_expr`'s exact shape
/// (`CONCAT(<a NamedNode>)`) lowers to the constant-false sentinel in
/// FILTER context (an erroring operand eliminates the row).
#[test]
fn error_marker_filter_lowers_to_constant_false() {
    let marker = func(
        Function::Concat,
        vec![Expression::NamedNode(sf_core::NamedNode::new_unchecked(
            "urn:sf-star:error-marker",
        ))],
    );
    let b = BTreeMap::new();
    assert!(matches!(
        filter_cond(&marker, &b, Dialect::Sqlite),
        Ok(SqlCond::Or(v)) if v.is_empty()
    ));
}

/// Literal sameTerm retains construction metadata independently of FILTER =.
#[test]
fn literal_same_term_retains_identity_instead_of_value_comparison() {
    let b = col_binding("x", "name");
    let eq = filter_cond(
        &Expression::Equal(Box::new(var("x")), Box::new(lit("Ada"))),
        &b,
        Dialect::Sqlite,
    )
    .unwrap();
    let st = filter_cond(
        &Expression::SameTerm(Box::new(var("x")), Box::new(lit("Ada"))),
        &b,
        Dialect::Sqlite,
    )
    .unwrap();
    assert!(
        matches!((&eq, &st), (SqlCond::LiteralCmp(value), SqlCond::LiteralCmp(identity))
            if matches!(value.value_op, Some(CmpOp::Eq)) && identity.value_op.is_none()),
        "eq={eq:?} st={st:?}"
    );
}

/// Literal variables retain value-comparison roles, not raw join authority.
#[test]
fn literal_variable_comparisons_preserve_value_roles() {
    let mut b = col_binding("t1_s", "col_a");
    b.extend(col_binding("t2_s", "col_b"));
    let cond = filter_cond(
        &Expression::Equal(Box::new(var("t1_s")), Box::new(var("t2_s"))),
        &b,
        Dialect::Sqlite,
    )
    .unwrap();
    assert!(
        matches!(&cond, SqlCond::LiteralCmp(cmp) if matches!(cmp.value_op, Some(CmpOp::Eq))
            && cmp.columns().map(|c| c.column.as_ref()).collect::<Vec<_>>() == ["col_a", "col_b"]),
        "{cond:?}"
    );
    // Both literal operands are now represented for ordered comparisons too.
    assert!(filter_cond(
        &Expression::Greater(Box::new(var("t1_s")), Box::new(var("t2_s"))),
        &b,
        Dialect::Sqlite
    )
    .is_ok());
}

// -- ADR-0032 D6 lift: align_templates literal-prefix disjointness -----

/// Two templates whose LEADING LITERAL prefixes CONFLICT (differ at a
/// shared position) are provably disjoint — pruned even though their
/// lengths differ, the exact case the old length-check-only conservatism
/// could not resolve (`differential_star.rs`'s object-side-nesting-
/// depth-2 test: two distinct quoted shapes' description maps).
#[test]
fn align_templates_conflicting_prefix_is_disjoint_even_with_different_lengths() {
    let x = Template::parse("http://ex.org/leaf/{id}").unwrap();
    let y = Template::parse("http://ex.org/person/{id}/{sub}").unwrap();
    assert!(matches!(
        align_templates(&x, &TermSpec::iri(), 0, &y, &TermSpec::iri(), 1),
        Unify::Empty
    ));
}

/// Same leading literal prefix, but different overall length, over a
/// TYPED-LITERAL class (Run 4 Wave B3's `TemplateEq` fallback excludes
/// it — `lexical_eq_is_term_eq`'s doc comment) — the prefix alone does
/// NOT prove disjointness, and the shape mismatch is NOT one of the
/// classes the fallback can soundly close, so this stays the
/// pre-existing conservative Unsupported — never an unsound prune.
/// Companion `..._now_resolves_via_template_eq_for_iri` below is the
/// IDENTICAL shape over the IRI class, which now DOES close.
#[test]
fn align_templates_same_prefix_different_length_stays_unsupported_for_typed_literal() {
    let spec = TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(
        "http://www.w3.org/2001/XMLSchema#integer",
    ));
    let x = Template::parse("http://ex.org/person/{id}").unwrap();
    let y = Template::parse("http://ex.org/person/{id}/{sub}").unwrap();
    assert!(matches!(
        align_templates(&x, &spec, 0, &y, &spec, 1),
        Unify::Unsupported(_)
    ));
}

/// Run 4 Wave B3: the IDENTICAL same-prefix/different-length shape as
/// the typed-literal companion above, but over the IRI class — now
/// resolves to a `SqlCond::TemplateEq` (render-and-compare) instead of
/// Unsupported.
#[test]
fn align_templates_same_prefix_different_length_now_resolves_via_template_eq_for_iri() {
    let x = Template::parse("http://ex.org/person/{id}").unwrap();
    let y = Template::parse("http://ex.org/person/{id}/{sub}").unwrap();
    assert!(matches!(
        align_templates(&x, &TermSpec::iri(), 0, &y, &TermSpec::iri(), 1),
        Unify::Sat(eqs) if matches!(eqs.as_slice(), [SqlCond::TemplateEq(..)])
    ));
}

/// Identical prefix AND identical shape (same length, same segment
/// kinds) — unchanged from the pre-lift behavior: the columns align
/// pairwise into a satisfiable raw-column equality.
#[test]
fn align_templates_identical_prefix_and_columns_unify_normally() {
    let x = Template::parse("http://ex.org/person/{id}").unwrap();
    let y = Template::parse("http://ex.org/person/{pid}").unwrap();
    assert!(matches!(
        align_templates(&x, &TermSpec::iri(), 0, &y, &TermSpec::iri(), 1),
        Unify::Sat(eqs) if eqs.len() == 1
    ));
}

// -- var_var_eq_beyond_column: composed-variable equality beyond a bare
// column binding (ledger closeout boundary B) ---------------------------

/// ADR-0032 D3 item 4: two composed variables' PREDICATE components are
/// ALWAYS both `TermDef::Const` (RDF 1.2 §3.1: a quoted predicate is
/// baked in as a fixed constant, never a per-row column —
/// `r2rml/star.rs`'s `quote_shape` doc comment) — equal constants
/// resolve to the "always true" sentinel, unequal ones to "always
/// false", neither ever reaching `var_col`.
#[test]
fn const_vs_const_equality_resolves_to_true_false_sentinels() {
    let iri = |s: &str| Term::NamedNode(sf_core::NamedNode::new_unchecked(s));
    let mut equal = const_binding("t1_p", iri("http://example.com/hasAge"));
    equal.extend(const_binding("t2_p", iri("http://example.com/hasAge")));
    assert!(matches!(
        filter_cond(
            &Expression::Equal(Box::new(var("t1_p")), Box::new(var("t2_p"))),
            &equal,
            Dialect::Sqlite,
        ),
        Ok(SqlCond::And(v)) if v.is_empty()
    ));

    let mut unequal = const_binding("t1_p", iri("http://example.com/hasAge"));
    unequal.extend(const_binding("t2_p", iri("http://example.com/hasName")));
    assert!(matches!(
        filter_cond(
            &Expression::Equal(Box::new(var("t1_p")), Box::new(var("t2_p"))),
            &unequal,
            Dialect::Sqlite,
        ),
        Ok(SqlCond::Or(v)) if v.is_empty()
    ));
}

/// Two SAME-SHAPE template-bound components (e.g. two composed
/// variables' SUBJECT, an `rr:template`-mapped `rr:subjectMap`) align
/// pairwise-column-equal — reusing `align_templates` verbatim (the SAME
/// function `unify_derived`'s join-key case calls), never reaching
/// `var_col`.
#[test]
fn template_vs_template_same_shape_equality_lowers_to_col_eq() {
    let mut b = template_binding("t1_s", "http://ex.org/person/{person_id}", 0);
    b.extend(template_binding(
        "t2_s",
        "http://ex.org/person/{person_id}",
        1,
    ));
    let cond = filter_cond(
        &Expression::Equal(Box::new(var("t1_s")), Box::new(var("t2_s"))),
        &b,
        Dialect::Sqlite,
    )
    .unwrap();
    assert!(
        matches!(&cond, SqlCond::And(v)
            if matches!(v.as_slice(), [SqlCond::ColEq(a, b)] if a.alias == 0 && b.alias == 1)),
        "{cond:?}"
    );
}

/// Two template-bound components with CONFLICTING leading literal
/// prefixes are provably disjoint — constant false, reusing
/// `align_templates`'s own disjointness proof inline (no separate
/// `templates_provably_disjoint` call needed).
#[test]
fn template_vs_template_disjoint_prefix_equality_is_constant_false() {
    let mut b = template_binding("t1_s", "http://ex.org/leaf/{id}", 0);
    b.extend(template_binding(
        "t2_s",
        "http://ex.org/person/{id}/{sub}",
        1,
    ));
    assert!(matches!(
        filter_cond(
            &Expression::Equal(Box::new(var("t1_s")), Box::new(var("t2_s"))),
            &b,
            Dialect::Sqlite,
        ),
        Ok(SqlCond::Or(v)) if v.is_empty()
    ));
}

/// Run 4 Wave B3: a genuine template SHAPE mismatch (same prefix,
/// different length) over the IRI class — `template_binding` always
/// builds `TermSpec::iri()` — now resolves via the `SqlCond::TemplateEq`
/// fallback instead of erroring. Companion
/// `template_vs_template_shape_mismatch_stays_a_sharpened_501_for_typed_literal`
/// below is the IDENTICAL shape over a typed-literal class, which still
/// 501s with the SAME sharpened message this test used to assert.
#[test]
fn template_vs_template_shape_mismatch_now_resolves_via_template_eq() {
    let mut b = template_binding("t1_s", "http://ex.org/person/{id}", 0);
    b.extend(template_binding(
        "t2_s",
        "http://ex.org/person/{id}/{sub}",
        1,
    ));
    let cond = filter_cond(
        &Expression::Equal(Box::new(var("t1_s")), Box::new(var("t2_s"))),
        &b,
        Dialect::Sqlite,
    )
    .unwrap();
    assert!(
        matches!(&cond, SqlCond::And(v)
            if matches!(v.as_slice(), [SqlCond::TemplateEq(..)])),
        "{cond:?}"
    );
}

/// The SAME same-prefix/different-length shape mismatch as above, but
/// over a TYPED-LITERAL class — `lexical_eq_is_term_eq` excludes it (a
/// datatype's value space can equate two different lexical forms), so
/// this stays the pre-existing Unsupported, with the SAME sharpened
/// message the fix's predecessor test asserted for every shape mismatch.
#[test]
fn template_vs_template_shape_mismatch_stays_a_sharpened_501_for_typed_literal() {
    let typed = |var: &str, template: &str, alias: usize| {
        let mut m = BTreeMap::new();
        m.insert(
            var.to_owned(),
            TermDef::Derived {
                term_map: TermMap::Template(
                    Template::parse(template).unwrap(),
                    TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(
                        "http://www.w3.org/2001/XMLSchema#integer",
                    )),
                ),
                alias,
            },
        );
        m
    };
    let mut b = typed("t1_s", "http://ex.org/person/{id}", 0);
    b.extend(typed("t2_s", "http://ex.org/person/{id}/{sub}", 1));
    let err = filter_cond(
        &Expression::Equal(Box::new(var("t1_s")), Box::new(var("t2_s"))),
        &b,
        Dialect::Sqlite,
    )
    .unwrap_err();
    assert!(err.contains("qualified value construction"), "{err}");
}

/// A MIXED shape (one side template-bound, the other a bare column or a
/// constant) is NOT intercepted by `var_var_eq_beyond_column` — it falls
/// through to `var_col`, which still 501s exactly as before this ledger
/// closeout (out of the bounded subset: "everything else keeps the
/// 501").
#[test]
fn template_vs_column_equality_still_falls_through_to_var_col() {
    let mut b = template_binding("t1_s", "http://ex.org/person/{id}", 0);
    b.extend(col_binding("plain", "some_col"));
    let err = filter_cond(
        &Expression::Equal(Box::new(var("t1_s")), Box::new(var("plain"))),
        &b,
        Dialect::Sqlite,
    )
    .unwrap_err();
    assert!(err.contains("needs a plain column binding"), "{err}");
}
