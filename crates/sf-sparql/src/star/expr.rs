//! ADR-0032 D3 item 3-4 — expression-tree rewriting: the five triple-term
//! functions (SUBJECT/PREDICATE/OBJECT/isTRIPLE/TRIPLE, [`rewrite_function_call`]),
//! composed-aware `=`/`sameTerm` ([`rewrite_equality`]), and the general
//! recursive `Expression` walker ([`rewrite_expr`]) that dispatches to both
//! (plus [`super::walk::rewrite_pattern`] for `EXISTS`/`NOT EXISTS` bodies —
//! the only `Expression` variant carrying a nested `GraphPattern`).
//! [`error_marker_expr`]/[`bool_literal_expr`] are the two "resolve to a
//! plain `Expression` FILTER/BIND already understand" leaves these rewrites
//! bottom out in; see [`error_marker_expr`]'s own doc comment for why the
//! former is an encoding trick rather than a dedicated type (ledger F3).

use spargebra::algebra::{AggregateExpression, Expression, Function, OrderExpression};
use spargebra::term::{Literal, NamedNode};

use crate::{Error, Result};

use super::env::StarEnv;
use super::util::{ERROR_MARKER_IRI, XSD_BOOLEAN};
use super::walk::rewrite_pattern;

/// Rule R5a: recurse through an expression tree looking for `EXISTS`/`NOT
/// EXISTS` bodies (the only `Expression` variant carrying a `GraphPattern`) —
/// reachable from FILTER, BIND, ORDER BY, and OPTIONAL's ON-expression.
/// Structural recursion otherwise, EXCEPT `Equal`/`SameTerm` (ADR-0032 D3
/// item 4 — [`rewrite_equality`]) and `FunctionCall` (item 3 —
/// [`rewrite_function_call`]), which resolve composed-variable / triple-term-
/// literal operands STATICALLY wherever possible before falling back to
/// ordinary structural recursion.
pub(super) fn rewrite_expr(
    expr: &Expression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<Expression> {
    use Expression::*;
    Ok(match expr {
        NamedNode(_) | Literal(_) | Variable(_) | Bound(_) => expr.clone(),
        Or(a, b) => Or(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        And(a, b) => And(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Equal(a, b) => rewrite_equality(a, b, n, env, false)?,
        SameTerm(a, b) => rewrite_equality(a, b, n, env, true)?,
        Greater(a, b) => Greater(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        GreaterOrEqual(a, b) => GreaterOrEqual(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Less(a, b) => Less(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        LessOrEqual(a, b) => LessOrEqual(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        In(a, list) => In(
            Box::new(rewrite_expr(a, n, env)?),
            list.iter()
                .map(|e| rewrite_expr(e, n, env))
                .collect::<Result<_>>()?,
        ),
        Add(a, b) => Add(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Subtract(a, b) => Subtract(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Multiply(a, b) => Multiply(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Divide(a, b) => Divide(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        UnaryPlus(a) => UnaryPlus(Box::new(rewrite_expr(a, n, env)?)),
        UnaryMinus(a) => UnaryMinus(Box::new(rewrite_expr(a, n, env)?)),
        Not(a) => Not(Box::new(rewrite_expr(a, n, env)?)),
        Exists(gp) => Exists(Box::new(rewrite_pattern(gp, n, env)?)),
        If(a, b, c) => If(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
            Box::new(rewrite_expr(c, n, env)?),
        ),
        Coalesce(list) => Coalesce(
            list.iter()
                .map(|e| rewrite_expr(e, n, env))
                .collect::<Result<_>>()?,
        ),
        FunctionCall(f, args) => rewrite_function_call(f, args, n, env)?,
    })
}

/// ADR-0032 D3 item 3 — the five triple-term functions, resolved statically
/// wherever possible (engine-totality: relational data can never contain a
/// native triple term, so composed-ness is always statically known to this
/// rewrite via [`StarEnv`] / a literal `TRIPLE(...)`/`<<(...)>>` operand —
/// `<<( … )>>` inside an expression position parses to the SAME
/// `FunctionCall(Function::Triple, [s,p,o])` shape as an explicit `TRIPLE(...)`
/// call, spargebra `parser.rs`'s `ExprTripleTerm` rule — verified in the pinned
/// 0.4.6 source; there is no separate AST node to special-case).
fn rewrite_function_call(
    f: &Function,
    args: &[Expression],
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<Expression> {
    match (f, args) {
        (Function::Subject, [arg]) => {
            let (_, composed) = rewrite_and_check_composed(arg, n, env)?;
            Ok(match composed {
                Some((s, _, _)) => s,
                None => error_marker_expr(),
            })
        }
        (Function::Predicate, [arg]) => {
            let (_, composed) = rewrite_and_check_composed(arg, n, env)?;
            Ok(match composed {
                Some((_, p, _)) => p,
                None => error_marker_expr(),
            })
        }
        (Function::Object, [arg]) => {
            let (_, composed) = rewrite_and_check_composed(arg, n, env)?;
            Ok(match composed {
                Some((_, _, o)) => o,
                None => error_marker_expr(),
            })
        }
        // §17.4.6 asymmetry: isTRIPLE NEVER errors, unlike SUBJECT/PREDICATE/
        // OBJECT — always resolves to a definite boolean, both composed and
        // non-composed arms bind (never leave unbound), so the plain boolean
        // `Literal` (which `unify::bind_term_def`/`filter_cond` already, or
        // now, understand) is correct in EVERY context, unlike the error
        // marker (see `error_marker_expr`'s doc comment).
        (Function::IsTriple, [arg]) => {
            let (_, composed) = rewrite_and_check_composed(arg, n, env)?;
            Ok(bool_literal_expr(composed.is_some()))
        }
        // TRIPLE(e1,e2,e3) is "statically routable" only through call sites
        // that recognize it BEFORE generic recursion reaches here: a BIND
        // target (`rewrite_extend`) and an equality/sameTerm/SUBJECT/
        // PREDICATE/OBJECT/isTRIPLE operand (`rewrite_and_check_composed`,
        // used by both). Reaching this arm means neither applied — e.g. a
        // bare `FILTER(TRIPLE(...))`, or TRIPLE nested as an argument to some
        // other function — genuinely not statically routable in this wave.
        (Function::Triple, _) => Err(Error::Unsupported(
            "TRIPLE(...) outside a BIND target, an equality/sameTerm operand, or a \
             SUBJECT/PREDICATE/OBJECT/isTRIPLE argument is not statically routable \
             (ADR-0032 D3) → 501"
                .to_owned(),
        )),
        _ => Ok(Expression::FunctionCall(
            f.clone(),
            args.iter()
                .map(|e| rewrite_expr(e, n, env))
                .collect::<Result<_>>()?,
        )),
    }
}

/// ADR-0032 D3 item 4 — `=`/`sameTerm` where either side is composed
/// (§17.4.2): both composed → component-wise conjunction (subject/predicate
/// compared directly — RDF 1.2 §3.1: they can never themselves be triple
/// terms, so no recursion is needed there; object recurses, so nested
/// composed terms compare structurally all the way down); exactly one
/// composed → the constant `false` (a triple term can never equal a
/// non-triple-term value — well-defined, never an error, for BOTH operators);
/// neither composed → ordinary (unchanged) `Equal`/`SameTerm`.
fn rewrite_equality(
    a: &Expression,
    b: &Expression,
    n: &mut usize,
    env: &mut StarEnv,
    same_term: bool,
) -> Result<Expression> {
    let (ra, ca) = rewrite_and_check_composed(a, n, env)?;
    let (rb, cb) = rewrite_and_check_composed(b, n, env)?;
    let wrap = |l: Expression, r: Expression| {
        if same_term {
            Expression::SameTerm(Box::new(l), Box::new(r))
        } else {
            Expression::Equal(Box::new(l), Box::new(r))
        }
    };
    Ok(match (ca, cb) {
        (Some((sa, pa, oa)), Some((sb, pb, ob))) => {
            let cmp_s = wrap(sa, sb);
            let cmp_p = wrap(pa, pb);
            let cmp_o = rewrite_equality(&oa, &ob, n, env, same_term)?;
            Expression::And(
                Box::new(Expression::And(Box::new(cmp_s), Box::new(cmp_p))),
                Box::new(cmp_o),
            )
        }
        (Some(_), None) | (None, Some(_)) => bool_literal_expr(false),
        (None, None) => wrap(ra, rb),
    })
}

/// A composed expression's three (subject, predicate, object) sub-expressions
/// — [`rewrite_and_check_composed`]'s result shape, factored out for clippy's
/// `type_complexity` lint (mirrors [`super::env::ComposedInfo`]'s own reason
/// for existing).
type ComposedComponents = (Expression, Expression, Expression);

/// Rewrite `arg` (resolving any nested star construct it contains — e.g.
/// `OBJECT(?t)` where `?t` is composed resolves to `?t`'s own object
/// component var, which is then re-checked here), and additionally return its
/// three component sub-expressions if the REWRITTEN result is itself composed:
/// either an env-composed variable ([`StarEnv`]) or a literal
/// `TRIPLE(e1,e2,e3)`/`<<( e1 e2 e3 )>>` call (ADR-0032 D3 item 4's "ground
/// triple term literals in expressions ⇒ composed constants" — checked on the
/// RAW shape FIRST, before generic recursion, since `Function::Triple` is
/// Unsupported through any OTHER path — see [`rewrite_function_call`]).
fn rewrite_and_check_composed(
    arg: &Expression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<(Expression, Option<ComposedComponents>)> {
    if let Expression::FunctionCall(Function::Triple, parts) = arg {
        if let [e1, e2, e3] = parts.as_slice() {
            let (r1, _) = rewrite_and_check_composed(e1, n, env)?;
            let (r2, _) = rewrite_and_check_composed(e2, n, env)?;
            let (r3, _) = rewrite_and_check_composed(e3, n, env)?;
            let rewritten = Expression::FunctionCall(
                Function::Triple,
                vec![r1.clone(), r2.clone(), r3.clone()],
            );
            return Ok((rewritten, Some((r1, r2, r3))));
        }
    }
    let rewritten = rewrite_expr(arg, n, env)?;
    let composed = match &rewritten {
        Expression::Variable(v) => env.get(v).map(|info| {
            (
                Expression::Variable(info.s_var.clone()),
                Expression::Variable(info.p_var.clone()),
                Expression::Variable(info.o_var.clone()),
            )
        }),
        _ => None,
    };
    Ok((rewritten, composed))
}

/// SPARQL §17.4.6 SUBJECT/PREDICATE/OBJECT error on a provably-non-composed
/// argument (engine-totality — see [`rewrite_function_call`]'s doc comment):
/// this rewrite happens BEFORE it is known whether the containing context is
/// boolean (FILTER) or value (BIND), so it must pick ONE `Expression` shape
/// that is correct under BOTH downstream consumers (R5 — no silently
/// conflating error with a wrong bound value):
///
/// * FILTER (`unify::filter_cond`): this wave adds a `Function::Concat`
///   recognizer that treats this EXACT shape as the constant `false` — an
///   erroring FILTER operand eliminates the row, the same observable effect.
/// * BIND (`unify::bind_term_def`): its EXISTING `Function::Concat` arm
///   requires every operand to reconstruct as a `Term::Literal`
///   (`exec_core::build_term`'s refutable `let Some(Term::Literal(l)) = …
///   else { return Ok(None) }`); a `NamedNode` constant operand fails that
///   match, so `build_term` ALREADY (zero new runtime code) reconstructs this
///   to `None` — the exact §10 ASSIGN "expression error ⇒ variable unbound"
///   behavior.
///
/// A bare/fresh unbound variable was considered and rejected: both
/// `filter_cond`'s `var_col` and `bind_term_def`'s `Variable` arm require the
/// variable to already be a KNOWN column binding, so a truly-fresh name 501s
/// the WHOLE QUERY at translate time (wrong — this must be a per-row/
/// deterministic-always effect, not a translate-time failure) rather than
/// eliminating a row / leaving one BIND target unbound.
///
/// **Ledger F3 assessment — kept as an encoding trick, not promoted to a
/// dedicated type.** `Expression` is `spargebra::algebra::Expression`, a type
/// this crate consumes but does not own — a genuine `Expression::Error`
/// variant is not ours to add. The alternative within OUR OWN types (an
/// out-of-band `Result<Expression, ErrorMarker>` return threaded alongside
/// the ordinary `Result<Expression>`) would have to survive being embedded
/// arbitrarily deep in an expression tree built by [`rewrite_expr`]'s ~20-arm
/// structural recursion (`Or`/`And`/`Greater`/.../`Coalesce`) — EVERY arm
/// would need a case for "one operand errored", not just the 2 sites that
/// actually care today. Reusing an ordinary, always-legally-shaped
/// `Expression` node instead — `CONCAT` of a single `NamedNode` argument,
/// a shape no real SPARQL query can ever produce (`CONCAT` requires string
/// operands, SPARQL §17.4.5.4) — needs no special-casing anywhere in that
/// recursion: it just flows through as data, and is inspected at exactly the
/// two REAL leaf-consumption points: `unify::filter_cond`'s `is_error_marker`
/// recognizer (a dedicated ~4-line function), and `exec_core::build_term`'s
/// PRE-EXISTING `TermDef::Concat` arm, which needs no change at all (a
/// `NamedNode` operand simply fails its existing `Term::Literal` match).
/// Two touch points beats N (one per recursion arm) — the trick stays.
pub(super) fn error_marker_expr() -> Expression {
    Expression::FunctionCall(
        Function::Concat,
        vec![Expression::NamedNode(NamedNode::new_unchecked(
            ERROR_MARKER_IRI,
        ))],
    )
}

/// A plain `xsd:boolean` literal — `isTRIPLE`'s always-a-value result
/// (§17.4.6), and the leaf of an `=`/`sameTerm` "exactly one side composed"
/// comparison ([`rewrite_equality`]). Already understood, unchanged, by
/// `unify::bind_term_def` (`Expression::Literal` arm) and (this wave)
/// `unify::filter_cond`'s new boolean-literal arm.
pub(super) fn bool_literal_expr(v: bool) -> Expression {
    Expression::Literal(Literal::new_typed_literal(
        if v { "true" } else { "false" },
        NamedNode::new_unchecked(XSD_BOOLEAN),
    ))
}

pub(super) fn rewrite_order_expr(
    oe: &OrderExpression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<OrderExpression> {
    Ok(match oe {
        OrderExpression::Asc(e) => OrderExpression::Asc(rewrite_expr(e, n, env)?),
        OrderExpression::Desc(e) => OrderExpression::Desc(rewrite_expr(e, n, env)?),
    })
}

pub(super) fn rewrite_agg_expr(
    ae: &AggregateExpression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<AggregateExpression> {
    Ok(match ae {
        AggregateExpression::CountSolutions { distinct } => AggregateExpression::CountSolutions {
            distinct: *distinct,
        },
        AggregateExpression::FunctionCall {
            name,
            expr,
            distinct,
        } => AggregateExpression::FunctionCall {
            name: name.clone(),
            expr: rewrite_expr(expr, n, env)?,
            distinct: *distinct,
        },
    })
}
