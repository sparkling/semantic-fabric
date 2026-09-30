//! FILTER and BIND lowering with explicit refusal outside admitted shapes.
use super::*;

/// Lower a FILTER expression to a [`SqlCond`] over raw columns + bound params
/// (ADR-0007 v1 subset: comparisons / `&&` / `||` / `!` / `BOUND`, plus the
/// ADR-0020 §2 near-free FTS baseline `CONTAINS`/`STRSTARTS`/`STRENDS`/`REGEX`,
/// plus ADR-0032 D3's `sameTerm` / constant-boolean / error-marker arms below).
/// Anything outside the subset is reported unsupported — never dropped (dropping a
/// FILTER would be unsound). `bindings` resolves a variable to its raw column (the
/// variable must be a plain `rr:column` binding in v1); `dialect` gates the
/// dialect-specific string-match pushdown (e.g. PostgreSQL regex).
pub(crate) fn filter_branch(
    expression: &Expression,
    branch: &crate::iq::Branch,
    dialect: Dialect,
) -> std::result::Result<SqlCond, String> {
    filter_scopes(expression, &branch.bindings, dialect, &[branch])
}

pub(crate) fn filter_scopes(
    expression: &Expression,
    bindings: &BTreeMap<String, TermDef>,
    dialect: Dialect,
    scopes: &[&crate::iq::Branch],
) -> std::result::Result<SqlCond, String> {
    let condition = filter_cond(expression, bindings, dialect)?;
    for branch in scopes {
        crate::iq::iri_cmp::validate_filter_source(&condition, branch, dialect)?;
    }
    Ok(condition)
}

pub fn filter_cond(
    expr: &Expression,
    bindings: &BTreeMap<String, TermDef>,
    dialect: Dialect,
) -> Result<SqlCond, String> {
    match expr {
        Expression::And(a, b) => Ok(SqlCond::And(vec![
            filter_cond(a, bindings, dialect)?,
            filter_cond(b, bindings, dialect)?,
        ])),
        Expression::Or(a, b) => Ok(SqlCond::Or(vec![
            filter_cond(a, bindings, dialect)?,
            filter_cond(b, bindings, dialect)?,
        ])),
        Expression::Not(a) => Ok(SqlCond::Not(Box::new(filter_cond(a, bindings, dialect)?))),
        Expression::Bound(v) => var_col(v, bindings).map(SqlCond::IsNotNull),
        Expression::Equal(a, b) => cmp(a, b, CmpOp::Eq, bindings, dialect),
        // Literal operands retain identity separately from numeric FILTER value
        // comparisons. IRI/constructed operands retain their existing lowering;
        // natural derived pairs still need live decoder authority on all drivers.
        Expression::SameTerm(a, b) => literal_cmp::filter(a, b, None, bindings)
            .map(Ok)
            .or_else(|| literal_cmp::template_identity(a, b, bindings))
            .unwrap_or_else(|| cmp(a, b, CmpOp::Eq, bindings, dialect)),
        Expression::Greater(a, b) => cmp(a, b, CmpOp::Gt, bindings, dialect),
        Expression::GreaterOrEqual(a, b) => cmp(a, b, CmpOp::Ge, bindings, dialect),
        Expression::Less(a, b) => cmp(a, b, CmpOp::Lt, bindings, dialect),
        Expression::LessOrEqual(a, b) => cmp(a, b, CmpOp::Le, bindings, dialect),
        // ADR-0032 D3 items 3-4: a constant `xsd:boolean` literal — the
        // representation `star::rewrite_expr` uses for `isTRIPLE`'s result
        // and an `=`/`sameTerm` "exactly one side composed" comparison.
        // Lowers to the SAME `1 = 1` / `1 = 0` sentinel `render_conjunction`
        // (an empty `SqlCond::And`) and `SqlCond::Or([])`'s own `render_cond`
        // arm already use — no new emission code, just these two constructors.
        Expression::Literal(l) => match xsd_boolean_value(l) {
            Some(true) => Ok(SqlCond::And(vec![])),
            Some(false) => Ok(SqlCond::Or(vec![])),
            None => Err(format!(
                "FILTER expression not supported in v1: non-boolean bare literal {l:?}"
            )),
        },
        // ADR-0032 D3 item 3's error marker (`star::error_marker_expr`'s doc
        // comment has the full rationale): a provably-non-composed
        // SUBJECT/PREDICATE/OBJECT argument — an erroring FILTER operand
        // eliminates the row, the same observable effect as constant false.
        Expression::FunctionCall(f, args) if is_error_marker(f, args) => Ok(SqlCond::Or(vec![])),
        Expression::FunctionCall(f, args) => str_match(f, args, bindings, dialect),
        other => Err(format!("FILTER expression not supported in v1: {other:?}")),
    }
}

/// Whether `l` is a plain `xsd:boolean` literal, and if so its value.
fn xsd_boolean_value(l: &Literal) -> Option<bool> {
    if l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#boolean" {
        Some(l.value() == "true")
    } else {
        None
    }
}

/// Whether `(f, args)` is exactly `star::error_marker_expr`'s shape — a
/// single `NamedNode` argument to `CONCAT`. See that function's doc comment
/// for the full rationale (this is the FILTER-side half; `bind_term_def`'s
/// EXISTING, unmodified `Function::Concat` arm already handles the BIND side).
fn is_error_marker(f: &Function, args: &[Expression]) -> bool {
    matches!(f, Function::Concat) && matches!(args, [Expression::NamedNode(_)])
}

/// Lower a `BIND(expr AS ?v)` expression to the [`TermDef`] for `?v`, reusing the
/// outer-projection term-construction lifting (ADR-0007): the value is built in
/// Rust at reconstruction, never inside a join/filter. Supported in this wave:
///
/// * a constant — an IRI (`Const` IRI) or a literal (`Const` literal);
/// * a bare variable `?y` — a column/term copy (clone `?y`'s binding);
/// * `CONCAT(a, b, …)` — each operand lowered recursively, reconstructed and
///   concatenated into a plain literal ([`TermDef::Concat`]).
///
/// Anything else (arithmetic, other built-ins, `IF`/`COALESCE`/`EXISTS`, …) is
/// reported unsupported → the whole query is `501` (never a silent wrong answer).
pub fn bind_term_def(
    expr: &Expression,
    bindings: &BTreeMap<String, TermDef>,
) -> Result<TermDef, String> {
    match expr {
        Expression::NamedNode(n) => Ok(TermDef::Const(Term::NamedNode(n.clone()))),
        Expression::Literal(l) => Ok(TermDef::Const(Term::Literal(l.clone()))),
        Expression::Variable(v) => bindings
            .get(v.as_str())
            .cloned()
            .ok_or_else(|| format!("BIND references unbound ?{}", v.as_str())),
        Expression::FunctionCall(Function::Concat, args) => {
            let mut parts = Vec::with_capacity(args.len());
            for a in args {
                parts.push(bind_term_def(a, bindings)?);
            }
            Ok(TermDef::Concat(parts))
        }
        other => Err(format!(
            "BIND expression not supported in v1 → 501: {other:?}"
        )),
    }
}

/// Lower a string-match SPARQL function to a source-side [`SqlCond::StrMatch`]
/// (ADR-0020 §2 near-free FTS). The match operand is built as a **bound parameter
/// value**, never concatenated into SQL text (ADR-0010 R1):
///
/// * `CONTAINS(?x, "s")`  → `col LIKE '%s%'`  (param `%s%`, metachars escaped)
/// * `STRSTARTS(?x, "s")` → `col LIKE 's%'`
/// * `STRENDS(?x, "s")`   → `col LIKE '%s'`
/// * `REGEX(?x, "p" [,fl])`→ PostgreSQL `col ~ p` (`~*` with the `i` flag); on
///   SQLite/MySQL there is no built-in regex operator → unsupported (not dropped).
///
/// `CONTAINS`/`STRSTARTS`/`STRENDS` are **case-sensitive** in SPARQL, but only
/// PostgreSQL's `LIKE` is genuinely case-sensitive — SQLite `LIKE` is
/// ASCII-case-insensitive by default and MySQL's default collations are
/// case-insensitive. So the `LIKE` pushdown fires **only on PostgreSQL**; on the
/// other dialects the FILTER is left un-rewritten (Unsupported, never a wrong
/// case-folding LIKE). The match also fires only over a raw column-backed literal
/// var; if `?x` is a constructed term (template IRI etc.), [`var_col`] errors and
/// the FILTER is likewise left un-rewritten.
fn str_match(
    f: &Function,
    args: &[Expression],
    bindings: &BTreeMap<String, TermDef>,
    dialect: Dialect,
) -> Result<SqlCond, String> {
    let like = |col: ColRef, pat: String| SqlCond::StrMatch {
        col,
        op: StrMatchOp::Like,
        param: pat,
    };
    match f {
        Function::Contains | Function::StrStarts | Function::StrEnds => {
            let (var, lit) = str_fn_2(args)?;
            let col = var_col(var, bindings)?;
            // SPARQL CONTAINS/STRSTARTS/STRENDS are CASE-SENSITIVE. Only PostgreSQL's
            // LIKE is genuinely case-sensitive. SQLite LIKE is ASCII-case-INSENSITIVE
            // (PRAGMA case_sensitive_like defaults OFF) and MySQL's default collations
            // are case-insensitive — a LIKE there would match MORE rows than SPARQL
            // (an unsound =_bag / NoREC divergence). SQLite's case-sensitive GLOB
            // cannot round-trip the sqlparser AST (no GLOB operator in 0.62), and a
            // connection PRAGMA is not self-contained in the emitted SQL (ADR-0010).
            // So push down only on PostgreSQL; on other dialects leave the FILTER
            // un-rewritten (Unsupported → fall back; correctness over coverage).
            if !dialect.like_is_case_sensitive() {
                return Err(format!(
                    "case-sensitive {f:?} pushdown unsupported on {dialect:?}: \
                     only PostgreSQL LIKE is case-sensitive (never silently wrong)"
                ));
            }
            let esc = escape_like(&lit);
            let pat = match f {
                Function::Contains => format!("%{esc}%"),
                Function::StrStarts => format!("{esc}%"),
                Function::StrEnds => format!("%{esc}"),
                _ => unreachable!(),
            };
            Ok(like(col, pat))
        }
        Function::Regex => {
            // REGEX(text, pattern [, flags]) — pattern + flags must be literals.
            if args.len() < 2 || args.len() > 3 {
                return Err("REGEX needs (text, pattern [, flags])".to_owned());
            }
            let var = match &args[0] {
                Expression::Variable(v) => v,
                other => return Err(format!("REGEX text must be a variable: {other:?}")),
            };
            let col = var_col(var, bindings)?;
            let pattern = expr_str_literal(&args[1])?;
            let case_insensitive = match args.get(2) {
                Some(e) => expr_str_literal(e)?.contains('i'),
                None => false,
            };
            match dialect {
                Dialect::Postgres => Ok(SqlCond::StrMatch {
                    col,
                    op: if case_insensitive {
                        StrMatchOp::RegexMatchI
                    } else {
                        StrMatchOp::RegexMatch
                    },
                    param: pattern,
                }),
                // SQLite has no built-in REGEXP operator; MySQL regex pushdown is
                // not wired (stub dialect). Report unsupported — never silently drop.
                // All other dialects (added in ADR-0024 M8) are also not wired.
                _ => Err(
                    "REGEX pushdown unsupported on this dialect (no built-in regex operator)"
                        .to_owned(),
                ),
            }
        }
        other => Err(format!("FILTER function not supported in v1: {other:?}")),
    }
}

/// Extract `(variable, literal-value)` from a 2-arg string function. The first
/// operand must be a variable (resolved to a raw column by the caller); the second
/// must be a plain string literal (the search operand).
fn str_fn_2(args: &[Expression]) -> Result<(&Variable, String), String> {
    match args {
        [Expression::Variable(v), search] => Ok((v, expr_str_literal(search)?)),
        _ => Err("string FILTER needs (variable, string-literal)".to_owned()),
    }
}

/// Escape SQL `LIKE` metacharacters in a user literal so it matches literally
/// (ADR-0020 §2): `%`, `_` and the escape char `\` itself are each `\`-prefixed,
/// to be used with `… ESCAPE '\'`. So `CONTAINS("a%b")` matches a literal percent.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\\' || c == '%' || c == '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The lexical value of a string-literal operand (a search/pattern argument).
fn expr_str_literal(e: &Expression) -> Result<String, String> {
    match e {
        Expression::Literal(l) => Ok(l.value().to_owned()),
        other => Err(format!("expected a string literal operand: {other:?}")),
    }
}

fn cmp(
    a: &Expression,
    b: &Expression,
    op: CmpOp,
    bindings: &BTreeMap<String, TermDef>,
    dialect: Dialect,
) -> Result<SqlCond, String> {
    if let Some(result) = str_comparison::comparison(a, b, op, bindings) {
        return result;
    }
    if [a, b]
        .iter()
        .any(|e| matches!(e, Expression::Variable(v) if !bindings.contains_key(v.as_str())))
    {
        return Ok(SqlCond::ExpressionError);
    }
    if let Some(comparison) = literal_cmp::kind_mismatch(a, b, op, bindings) {
        return Ok(comparison);
    }
    if let Some(comparison) = literal_cmp::filter(a, b, Some(op), bindings) {
        return Ok(comparison);
    }
    if let Some(comparison) = iri_cmp::filter(a, b, op, bindings, dialect) {
        return Ok(comparison);
    }
    if literal_cmp::template_value_needs_construction(a, b, bindings) {
        return Err("typed literal-template FILTER requires qualified value construction, not RDF-key equality".into());
    }
    match (a, b) {
        // ADR-0032 D3 item 4: `star::rewrite_equality`'s "both composed"
        // component-wise conjunction compares two component VARIABLES
        // directly (e.g. `?t1_s = ?t2_s`), not a variable against a constant
        // — `SqlCond::ColEq` (column = column, the SAME condition ordinary
        // join-key equality already uses) covers `Eq`; the other operators
        // have no column-vs-column `SqlCond` and stay unsupported (never
        // silently wrong — sound over complete).
        (Expression::Variable(v1), Expression::Variable(v2)) => {
            if op != CmpOp::Eq {
                return Err(format!(
                    "{op:?} between two variables needs a constant operand in v1"
                ));
            }
            if let Some(cond) = var_var_eq_beyond_column(v1, v2, bindings)? {
                return Ok(cond);
            }
            let c1 = var_col(v1, bindings)?;
            let c2 = var_col(v2, bindings)?;
            Ok(SqlCond::ColEq(c1, c2))
        }
        (Expression::Variable(v), rhs) => {
            let col = var_col(v, bindings)?;
            Ok(SqlCond::Cmp(col, op, expr_const(rhs)?))
        }
        (lhs, Expression::Variable(v)) => {
            let col = var_col(v, bindings)?;
            Ok(SqlCond::Cmp(col, flip(op), expr_const(lhs)?))
        }
        _ => Err("comparison needs a variable operand in v1".to_owned()),
    }
}

fn flip(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Lt => CmpOp::Gt,
        CmpOp::Le => CmpOp::Ge,
        CmpOp::Gt => CmpOp::Lt,
        CmpOp::Ge => CmpOp::Le,
        other => other,
    }
}

/// ADR-0032 D3 item 4's component-wise equality recursion
/// (`star::rewrite_equality`) compares two composed variables' components
/// directly (e.g. `?t1_s = ?t2_s`) — `var_col` (below) only resolves a bare
/// `rr:column` binding, a pre-existing v1 scope limit
/// (`differential_star.rs`'s `equality_and_same_term_over_composed_variables`
/// doc comment). Two of the shapes a component binding can ALSO take are
/// resolvable without that restriction, so `cmp`'s variable-vs-variable arm
/// consults this FIRST:
///
/// * **Both constant** — RDF 1.2 §3.1 predicates are always IRIs, and
///   `sf-mapping`'s quoted-shape compiler bakes a quoted predicate in as a
///   fixed constant, never a per-row column (`r2rml/star.rs`'s `quote_shape`:
///   "the predicate must be compile-time known ... never as a per-row
///   column") — so this is the shape EVERY composed-variable equality's
///   PREDICATE component reaches, not a rare case. Resolved statically, no
///   SQL at all.
/// * **Both template-bound** — e.g. two composed variables' SUBJECT
///   component, an `rr:template`-mapped `rr:subjectMap`. Unifies exactly
///   like a JOIN key already does: [`align_templates`] (reused verbatim,
///   the SAME function `unify_derived` calls) proves same-shape templates
///   pairwise-column-equal, a leading-literal-prefix conflict provably
///   disjoint (constant false), a genuine shape mismatch a `SqlCond::
///   TemplateEq` rendered-concat comparison (Run 4 Wave B3, IRI/plain-string
///   classes only), or reports the remaining shape mismatches Unsupported —
///   sound over complete, same as everywhere else in this file.
///
/// `Ok(None)` for every other shape (a bare column on either side, a
/// `Coalesce`/`Concat`/`Agg`/`ComposedTriple`, or a constant/template
/// MIXED with something else) — falls through to the ordinary `var_col`
/// path below, unchanged.
fn var_var_eq_beyond_column(
    v1: &Variable,
    v2: &Variable,
    bindings: &BTreeMap<String, TermDef>,
) -> Result<Option<SqlCond>, String> {
    let (Some(d1), Some(d2)) = (bindings.get(v1.as_str()), bindings.get(v2.as_str())) else {
        return Ok(None);
    };
    match (d1, d2) {
        (TermDef::Const(a), TermDef::Const(b)) => Ok(Some(if a == b {
            SqlCond::And(vec![])
        } else {
            SqlCond::Or(vec![])
        })),
        (TermDef::R2rmlBlank { .. }, TermDef::R2rmlBlank { .. }) => match unify(d1, d2) {
            Unify::Sat(conditions) => Ok(Some(SqlCond::And(conditions))),
            Unify::Empty => Ok(Some(SqlCond::Or(vec![]))),
            Unify::Unsupported(why) => Err(format!(
                "FILTER equality on ?{}/?{}: scoped R2RML blank nodes cannot align ({why})",
                v1.as_str(),
                v2.as_str()
            )),
        },
        (
            TermDef::Derived {
                term_map: TermMap::Template(t1, spec1),
                alias: a1,
            },
            TermDef::Derived {
                term_map: TermMap::Template(t2, spec2),
                alias: a2,
            },
        ) => match align_templates(t1, spec1, *a1, t2, spec2, *a2) {
            Unify::Sat(eqs) => Ok(Some(SqlCond::And(eqs))),
            Unify::Empty => Ok(Some(SqlCond::Or(vec![]))),
            Unify::Unsupported(why) => Err(format!(
                "FILTER equality on ?{}/?{}: template-bound components with a shape v1 cannot \
                 align ({why})",
                v1.as_str(),
                v2.as_str()
            )),
        },
        _ => Ok(None),
    }
}

fn var_col(v: &Variable, bindings: &BTreeMap<String, TermDef>) -> Result<ColRef, String> {
    match bindings.get(v.as_str()) {
        Some(TermDef::Derived {
            term_map: TermMap::Column(col, _),
            alias,
        }) => Ok(ColRef::new(*alias, col.clone())),
        Some(_) => Err(format!(
            "FILTER on ?{} needs a plain column binding in v1",
            v.as_str()
        )),
        None => Err(format!("FILTER references unbound ?{}", v.as_str())),
    }
}

fn expr_const(e: &Expression) -> Result<String, String> {
    match e {
        Expression::Literal(l) => Ok(l.value().to_owned()),
        Expression::NamedNode(n) => Ok(n.as_str().to_owned()),
        other => Err(format!("FILTER constant operand not supported: {other:?}")),
    }
}
