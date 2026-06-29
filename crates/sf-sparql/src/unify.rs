//! Variable unification and FILTER lowering — the raw-column half of the base
//! translation (ADR-0007 *Term-construction lifting*).
//!
//! When a SPARQL variable occurs in two patterns, its two [`TermDef`]s must be
//! made equal. Unification reduces that to **raw-column equalities** whenever the
//! two term definitions are structurally compatible (the lifting that keeps joins
//! on indexed key columns); proves the join **empty** when the IRI/literal shapes
//! are disjoint (the seed the IRI-template-mismatch cascade pass formalises); and
//! otherwise reports the case **unsupported** (deferred, never silently wrong).

use sf_core::ir::{Segment, TermMap, TermType};
use sf_core::Term;

use sf_sql::Dialect;

use crate::iq::{CmpOp, ColRef, SqlCond, StrMatchOp, TermDef};

/// The result of unifying two term definitions for the same variable.
pub enum Unify {
    /// Satisfiable under these (raw-column / constant) conditions.
    Sat(Vec<SqlCond>),
    /// Provably disjoint — the containing branch is pruned (0 rows).
    Empty,
    /// A correct reduction is not yet implemented (deferred; ADR-0007 v1).
    Unsupported(String),
}

/// Unify two definitions of the same variable.
pub fn unify(a: &TermDef, b: &TermDef) -> Unify {
    match (a, b) {
        (TermDef::Const(x), TermDef::Const(y)) => {
            if x == y {
                Unify::Sat(vec![])
            } else {
                Unify::Empty
            }
        }
        (TermDef::Const(c), TermDef::Derived { term_map, alias })
        | (TermDef::Derived { term_map, alias }, TermDef::Const(c)) => {
            unify_const_derived(c, term_map, *alias)
        }
        (
            TermDef::Derived {
                term_map: t1,
                alias: a1,
            },
            TermDef::Derived {
                term_map: t2,
                alias: a2,
            },
        ) => unify_derived(t1, *a1, t2, *a2),
        // A COALESCE'd (multi-OPTIONAL shared var) or CONCAT'd (BIND-computed)
        // binding is a multi-source constructed term; reducing it to raw-column
        // equalities is deferred (ADR-0007 v1 — never silently wrong). So sharing a
        // BIND variable with a later pattern (a join/filter on it) defers to 501.
        // An aggregate result is produced post-grouping and is never re-unified into
        // a join/filter (the group is the outermost pattern in v1).
        (TermDef::Coalesce(..) | TermDef::Concat(..) | TermDef::Agg { .. }, _)
        | (_, TermDef::Coalesce(..) | TermDef::Concat(..) | TermDef::Agg { .. }) => {
            Unify::Unsupported(
                "unification of a COALESCE'd / CONCAT'd / aggregate (multi-source / computed) \
                 binding"
                    .to_owned(),
            )
        }
    }
}

/// Unify a constant against a column/template term map: the raw column(s) must
/// equal the constant's lexical form.
fn unify_const_derived(c: &Term, tm: &TermMap, alias: usize) -> Unify {
    let want = match const_lexical(c, term_map_type(tm)) {
        Ok(v) => v,
        Err(()) => return Unify::Empty, // term-kind mismatch ⇒ disjoint
    };
    match tm {
        TermMap::Constant(_) => unreachable!("Derived never wraps a constant term map"),
        TermMap::Column(col, _) => Unify::Sat(vec![SqlCond::Cmp(
            ColRef::new(alias, col.clone()),
            CmpOp::Eq,
            want,
        )]),
        TermMap::Template(t, _) => match split_template(t) {
            TemplateShape::AllLiteral(text) => {
                if text == want {
                    Unify::Sat(vec![])
                } else {
                    Unify::Empty
                }
            }
            TemplateShape::SingleSlot {
                prefix,
                column,
                suffix,
            } => {
                if want.len() < prefix.len() + suffix.len()
                    || !want.starts_with(&prefix)
                    || !want.ends_with(&suffix)
                {
                    return Unify::Empty;
                }
                // v1: the extracted middle is used verbatim (no percent-decode);
                // sound for the common no-special-char key case (ADR-0007 v1).
                let middle = &want[prefix.len()..want.len() - suffix.len()];
                Unify::Sat(vec![SqlCond::Cmp(
                    ColRef::new(alias, column),
                    CmpOp::Eq,
                    middle.to_owned(),
                )])
            }
            TemplateShape::MultiSlot => {
                Unify::Unsupported("constant vs multi-slot template".to_owned())
            }
        },
    }
}

/// Unify two column/template term maps → raw-column equalities, or a disjointness
/// proof, or unsupported.
fn unify_derived(t1: &TermMap, a1: usize, t2: &TermMap, a2: usize) -> Unify {
    if let (Some(k1), Some(k2)) = (term_map_type(t1), term_map_type(t2)) {
        if k1 != k2 {
            return Unify::Empty; // an IRI can never equal a literal, etc.
        }
    }
    // `sameTerm` for literals also requires matching datatype + language (SPARQL
    // §17.4.1.7): two literal term maps whose static *effective* datatype/language
    // differ can never be sameTerm (`"1990"^^xsd:integer` vs `"1990"^^xsd:string`,
    // `"Ada"@en` vs `"Ada"@fr`), so they are disjoint — never equate them by a raw
    // lexical column. Closes a false-match in MINUS anti-joins and BGP joins alike.
    if let (Some(l1), Some(l2)) = (literal_key(t1), literal_key(t2)) {
        if l1 != l2 {
            return Unify::Empty;
        }
    }
    match (t1, t2) {
        (TermMap::Column(c1, _), TermMap::Column(c2, _)) => Unify::Sat(vec![SqlCond::ColEq(
            ColRef::new(a1, c1.clone()),
            ColRef::new(a2, c2.clone()),
        )]),
        (TermMap::Template(x, _), TermMap::Template(y, _)) => align_templates(x, a1, y, a2),
        _ => Unify::Unsupported("column vs template unification".to_owned()),
    }
}

/// Two templates unify iff their segment kind-sequence matches (same fixed text
/// at each literal position, a column slot at each column position). Aligned
/// columns become pairwise raw-column equalities; a fixed-text mismatch proves
/// disjointness; a kind/length mismatch is conservatively unsupported (never an
/// unsound prune).
fn align_templates(
    x: &sf_core::ir::Template,
    a1: usize,
    y: &sf_core::ir::Template,
    a2: usize,
) -> Unify {
    let (sx, sy) = (x.segments(), y.segments());
    if sx.len() != sy.len() {
        return Unify::Unsupported("template length mismatch".to_owned());
    }
    let mut eqs = Vec::new();
    for (p, q) in sx.iter().zip(sy.iter()) {
        match (p, q) {
            (Segment::Literal(l), Segment::Literal(r)) => {
                if l != r {
                    return Unify::Empty;
                }
            }
            (Segment::Column(c1), Segment::Column(c2)) => {
                eqs.push(SqlCond::ColEq(
                    ColRef::new(a1, c1.clone()),
                    ColRef::new(a2, c2.clone()),
                ));
            }
            _ => return Unify::Unsupported("template shape mismatch".to_owned()),
        }
    }
    Unify::Sat(eqs)
}

enum TemplateShape {
    AllLiteral(String),
    SingleSlot {
        prefix: String,
        column: Box<str>,
        suffix: String,
    },
    MultiSlot,
}

fn split_template(t: &sf_core::ir::Template) -> TemplateShape {
    let segs = t.segments();
    let slots: Vec<usize> = segs
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, Segment::Column(_)))
        .map(|(i, _)| i)
        .collect();
    match slots.as_slice() {
        [] => {
            let mut s = String::new();
            for seg in segs {
                if let Segment::Literal(l) = seg {
                    s.push_str(l);
                }
            }
            TemplateShape::AllLiteral(s)
        }
        [i] => {
            let mut prefix = String::new();
            let mut suffix = String::new();
            for seg in &segs[..*i] {
                if let Segment::Literal(l) = seg {
                    prefix.push_str(l);
                }
            }
            for seg in &segs[*i + 1..] {
                if let Segment::Literal(l) = seg {
                    suffix.push_str(l);
                }
            }
            let column = match &segs[*i] {
                Segment::Column(c) => c.clone(),
                Segment::Literal(_) => unreachable!(),
            };
            TemplateShape::SingleSlot {
                prefix,
                column,
                suffix,
            }
        }
        _ => TemplateShape::MultiSlot,
    }
}

fn term_map_type(tm: &TermMap) -> Option<TermType> {
    crate::iq::term_map_type(tm)
}

/// The effective `(datatype-IRI, language)` of a *literal* term map — a plain
/// literal normalises to `xsd:string`, a lang-tagged literal to `rdf:langString` —
/// i.e. the key under which two literals are `sameTerm` (SPARQL §17.4.1.7). `None`
/// for a non-literal term map (IRI / blank node) or a wrapped constant.
fn literal_key(tm: &TermMap) -> Option<(&str, Option<&str>)> {
    let spec = match tm {
        TermMap::Column(_, s) | TermMap::Template(_, s) => s,
        TermMap::Constant(_) => return None,
    };
    if spec.term_type != TermType::Literal {
        return None;
    }
    if let Some(lang) = spec.language.as_deref() {
        Some((
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
            Some(lang),
        ))
    } else if let Some(dt) = &spec.datatype {
        Some((dt.as_str(), None))
    } else {
        Some(("http://www.w3.org/2001/XMLSchema#string", None))
    }
}

/// The lexical form a constant must take to match a term map of the given type.
/// `Err(())` when the kinds are incompatible (e.g. an IRI term map vs a literal
/// constant) — a disjointness proof.
fn const_lexical(c: &Term, want: Option<TermType>) -> Result<String, ()> {
    match (c, want) {
        (Term::NamedNode(n), Some(TermType::Iri) | None) => Ok(n.as_str().to_owned()),
        (Term::BlankNode(b), Some(TermType::BlankNode) | None) => Ok(b.as_str().to_owned()),
        (Term::Literal(l), Some(TermType::Literal) | None) => Ok(l.value().to_owned()),
        _ => Err(()),
    }
}

// --- FILTER lowering ------------------------------------------------------

use spargebra::algebra::{Expression, Function};
use spargebra::term::Variable;
use std::collections::BTreeMap;

/// Lower a FILTER expression to a [`SqlCond`] over raw columns + bound params
/// (ADR-0007 v1 subset: comparisons / `&&` / `||` / `!` / `BOUND`, plus the
/// ADR-0020 §2 near-free FTS baseline `CONTAINS`/`STRSTARTS`/`STRENDS`/`REGEX`).
/// Anything outside the subset is reported unsupported — never dropped (dropping a
/// FILTER would be unsound). `bindings` resolves a variable to its raw column (the
/// variable must be a plain `rr:column` binding in v1); `dialect` gates the
/// dialect-specific string-match pushdown (e.g. PostgreSQL regex).
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
        Expression::Equal(a, b) => cmp(a, b, CmpOp::Eq, bindings),
        Expression::Greater(a, b) => cmp(a, b, CmpOp::Gt, bindings),
        Expression::GreaterOrEqual(a, b) => cmp(a, b, CmpOp::Ge, bindings),
        Expression::Less(a, b) => cmp(a, b, CmpOp::Lt, bindings),
        Expression::LessOrEqual(a, b) => cmp(a, b, CmpOp::Le, bindings),
        Expression::FunctionCall(f, args) => str_match(f, args, bindings, dialect),
        other => Err(format!("FILTER expression not supported in v1: {other:?}")),
    }
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
            if dialect != Dialect::Postgres {
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
                Dialect::Sqlite | Dialect::MySql => Err(
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
) -> Result<SqlCond, String> {
    match (a, b) {
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

#[cfg(test)]
mod tests {
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
}
