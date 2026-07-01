//! Execute — run the emitted SQL on a live SQLite source, reconstruct `oxrdf`
//! bindings/triples, and stream results (ADR-0006 *Streaming & bounded memory*;
//! ADR-0007 step 7).
//!
//! The reconstruction is the **single term-gen path** (ADR-0003 R3): the SQL
//! projects raw key columns and `sf-core`'s `generate_into` materialises the RDF
//! term per output position — terms are built here, in the outermost projection,
//! never inside a join/filter (ADR-0007 lifting). Streaming uses `sf-sql`'s
//! bounded SQLite cursor ([`sf_sql::sqlite_for_each`]) — one row in flight, so
//! memory is independent of result size. CPU-bound term-gen belongs on the
//! dedicated rayon pool ([`crate::pool`]); the sync SQLite path here generates
//! inline (no async runtime to protect — ADR-0006).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use rusqlite::Connection;
use sf_core::datatype::{self, XsdTypeCode};
use sf_core::ir::{TermMap, TermType};
use sf_core::{Literal, Row, Term, Triple};

use spargebra::algebra::{Expression, Function};

use crate::iq::{AggKind, Branch, ColRef, OrderKey, RustAgg, RustGroup, TermDef};
use crate::{Error, Plan, Result};

/// One projected result row's raw column values plus each value's resolved §10
/// type (declared type, else storage-class fallback), addressed by [`ColRef`].
/// `pub(crate)` so the PostgreSQL executor ([`crate::exec_pg`]) drives the same
/// single term-gen path (ADR-0003 R3) with PG-extracted values.
pub(crate) struct RawRow<'a> {
    pub(crate) schema: &'a [ColRef],
    pub(crate) values: &'a [Option<String>],
    pub(crate) codes: &'a [Option<XsdTypeCode>],
}

impl RawRow<'_> {
    /// The resolved §10 XSD type of `column` under `alias`, if any.
    fn code_for(&self, alias: usize, column: &str) -> Option<XsdTypeCode> {
        self.schema
            .iter()
            .position(|c| c.alias == alias && &*c.column == column)
            .and_then(|i| self.codes[i])
    }
}

/// A view of a [`RawRow`] scoped to one scan alias, so a mapping term map's
/// column lookups resolve to that scan's projected columns ([`sf_core::Row`]).
struct AliasRow<'a> {
    raw: &'a RawRow<'a>,
    alias: usize,
}

impl Row for AliasRow<'_> {
    fn value(&self, column: &str) -> Option<&str> {
        self.raw
            .schema
            .iter()
            .position(|c| c.alias == self.alias && &*c.column == column)
            .and_then(|i| self.raw.values[i].as_deref())
    }
}

/// Materialise a term definition into an `oxrdf` term, or `None` if a referenced
/// column is NULL/absent (R2RML §11: no value ⇒ no term ⇒ unbound).
fn build_term(def: &TermDef, raw: &RawRow<'_>) -> Result<Option<Term>> {
    match def {
        TermDef::Const(t) => Ok(Some(t.clone())),
        TermDef::Derived { term_map, alias } => derived_term(term_map, *alias, raw),
        // R2 COALESCE: the preserved (left) side wins when bound; otherwise the
        // optional (right) value (ADR-0007). `None` from `left` = its source
        // columns were NULL (the optional did not match), so fall back to `right`.
        TermDef::Coalesce(l, r) => match build_term(l, raw)? {
            Some(t) => Ok(Some(t)),
            None => build_term(r, raw),
        },
        // BIND(CONCAT(…)) — SPARQL §17.4.5.4. Every operand must be a string literal
        // (xsd:string, simple, or lang-tagged); an unbound / IRI / blank-node operand
        // or a non-string *typed* literal is an expression error, so the BIND variable
        // is left unbound (Ok(None)) — never a wrong value. The result carries the
        // common language tag iff every operand shares it, else a simple literal.
        TermDef::Concat(parts) => {
            let mut s = String::new();
            let mut common_lang: Option<Option<String>> = None; // unset | mixed | lang
            for p in parts {
                let Some(Term::Literal(l)) = build_term(p, raw)? else {
                    return Ok(None);
                };
                let lang = l.language();
                if lang.is_none() && l.datatype() != sf_core::vocab::xsd::STRING {
                    return Ok(None); // a non-string typed literal ⇒ type error
                }
                s.push_str(l.value());
                let this = lang.map(str::to_owned);
                common_lang = Some(match common_lang {
                    None => this,                       // first operand sets it
                    Some(prev) if prev == this => prev, // still consistent
                    Some(_) => None,                    // diverged ⇒ no common tag
                });
            }
            let term = match common_lang.flatten() {
                Some(lang) => Literal::new_language_tagged_literal(s, lang)
                    .map_err(|e| Error::Core(e.to_string()))?,
                None => Literal::new_simple_literal(s),
            };
            Ok(Some(Term::Literal(term)))
        }
        // An aggregate result (SPARQL §11): the value is the SQL aggregate computed
        // at `col`. A NULL value is an empty multiset: SUM (and COUNT, defensively —
        // SQL `COUNT` never NULLs) over an empty multiset is `"0"^^xsd:integer`,
        // while AVG/MIN/MAX (and SAMPLE) are UNBOUND (§11). The §10 type is
        // `fixed_type` when the function pins it (COUNT ⇒ integer), else the
        // column's resolved decltype/storage class (SUM/MIN/MAX keep the source
        // numeric type). AVG (§11.4) follows the OPERAND numeric type under XPath
        // promotion — resolved from `operand`'s §10 type, since SQLite's `AVG`
        // always yields a REAL (the operand is projected bare on SQLite; on PG it is
        // absent and `avg()`'s own promoted result type is used).
        TermDef::Agg {
            col,
            kind,
            operand,
            fixed_type,
        } => {
            let row = AliasRow {
                raw,
                alias: col.alias,
            };
            let Some(value) = row.value(&col.column) else {
                return match kind {
                    AggKind::Sum | AggKind::Count => {
                        Ok(Some(natural_literal("0", XsdTypeCode::Integer)?))
                    }
                    AggKind::Avg | AggKind::Min | AggKind::Max => Ok(None),
                };
            };
            let code = match kind {
                AggKind::Avg => {
                    let operand_code = operand
                        .as_ref()
                        .and_then(|o| raw.code_for(o.alias, &o.column))
                        .or_else(|| raw.code_for(col.alias, &col.column))
                        .unwrap_or(XsdTypeCode::Decimal);
                    avg_result_code(operand_code)
                }
                _ => fixed_type
                    .or_else(|| raw.code_for(col.alias, &col.column))
                    .unwrap_or(XsdTypeCode::String),
            };
            Ok(Some(natural_literal(value, code)?))
        }
    }
}

/// Build a derived term, applying the R2RML §10 natural datatype mapping
/// (ADR-0015) when — and only when — the term map is a column-valued literal with
/// no explicit `rr:datatype` / `rr:language`. Templates, IRIs, blank nodes, and
/// explicitly-typed/lang-tagged literals go through the plain `sf-core` term-gen
/// path unchanged.
fn derived_term(term_map: &TermMap, alias: usize, raw: &RawRow<'_>) -> Result<Option<Term>> {
    if let TermMap::Column(col, spec) = term_map {
        if spec.term_type == TermType::Literal && spec.datatype.is_none() && spec.language.is_none()
        {
            let row = AliasRow { raw, alias };
            let Some(value) = row.value(col) else {
                return Ok(None);
            };
            let code = raw.code_for(alias, col).unwrap_or(XsdTypeCode::String);
            return Ok(Some(natural_literal(value, code)?));
        }
    }
    let row = AliasRow { raw, alias };
    sf_core::term::generate(term_map, &row).map_err(|e| Error::Core(e.to_string()))
}

/// Produce the RDF literal for a value under its §10 natural XSD type, in the
/// XSD-canonical lexical form (ADR-0015 chokepoint, `sf_core::datatype`).
/// `HexBinary` values arrive already uppercase-hex-encoded from blob extraction.
fn natural_literal(value: &str, code: XsdTypeCode) -> Result<Term> {
    let literal = match code {
        XsdTypeCode::String => Literal::new_simple_literal(value),
        XsdTypeCode::HexBinary => Literal::new_typed_literal(value, code.iri()),
        _ => {
            let mut buf = String::new();
            datatype::canonical_lexical(value, code, &mut buf)
                .map_err(|e| Error::Core(e.to_string()))?;
            Literal::new_typed_literal(buf, code.iri())
        }
    };
    Ok(Term::Literal(literal))
}

/// The §10 result datatype of `AVG(operand)` (SPARQL §11.4: AVG = SUM/COUNT under
/// XPath numeric type promotion). The result follows the operand numeric type:
/// `xsd:double` is preserved (so is `xsd:float`, which this codebase folds into
/// `xsd:double`); `xsd:integer` and `xsd:decimal` promote to `xsd:decimal`.
fn avg_result_code(operand: XsdTypeCode) -> XsdTypeCode {
    match operand {
        XsdTypeCode::Double => XsdTypeCode::Double,
        _ => XsdTypeCode::Decimal,
    }
}

/// Reconstruct all bound variables of `branch` for one raw row. `pub(crate)` so
/// the PostgreSQL executor reuses the identical reconstruction (ADR-0003 R3).
pub(crate) fn reconstruct(branch: &Branch, raw: &RawRow<'_>) -> Result<BTreeMap<String, Term>> {
    let mut out = BTreeMap::new();
    for (var, def) in &branch.bindings {
        if let Some(term) = build_term(def, raw)? {
            out.insert(var.clone(), term);
        }
    }
    Ok(out)
}

/// Compare two solutions by the ORDER BY keys (SPARQL §15.1), honoring each key's
/// direction with explicit UNBOUND placement: an unbound key sorts FIRST for ASC
/// and LAST for DESC — matching the SQL `NULLS FIRST/LAST` the single-branch path
/// emits, so single- and multi-branch orderings agree. Bound terms order
/// blank-node < IRI < literal; numeric-typed literals compare by value (so
/// xsd:integer 2 < 10, not lexical "10" < "2"). `pub(crate)` so the PostgreSQL
/// executor sorts its bag-union identically.
pub(crate) fn order_cmp(
    order: &[OrderKey],
    a: &BTreeMap<String, Term>,
    b: &BTreeMap<String, Term>,
) -> Ordering {
    for key in order {
        let ord = match (a.get(&key.var), b.get(&key.var)) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => {
                if key.descending {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (Some(_), None) => {
                if key.descending {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (Some(x), Some(y)) => {
                let c = cmp_term(x, y);
                if key.descending {
                    c.reverse()
                } else {
                    c
                }
            }
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// SPARQL term order extended to a total order for sorting: blank node < IRI <
/// literal; within a kind by value.
fn cmp_term(a: &Term, b: &Term) -> Ordering {
    fn rank(t: &Term) -> u8 {
        match t {
            Term::BlankNode(_) => 0,
            Term::NamedNode(_) => 1,
            Term::Literal(_) => 2,
            _ => 3, // quoted triple (RDF-star) — sorts last, by lexical form
        }
    }
    match (a, b) {
        (Term::BlankNode(x), Term::BlankNode(y)) => x.as_str().cmp(y.as_str()),
        (Term::NamedNode(x), Term::NamedNode(y)) => x.as_str().cmp(y.as_str()),
        (Term::Literal(x), Term::Literal(y)) => cmp_literal(x, y),
        _ => rank(a)
            .cmp(&rank(b))
            .then_with(|| a.to_string().cmp(&b.to_string())),
    }
}

/// Compare two literals: numerically when both carry a numeric XSD datatype, else
/// by lexical value, then datatype IRI, then language tag.
fn cmp_literal(x: &Literal, y: &Literal) -> Ordering {
    if let (Some(nx), Some(ny)) = (numeric_value(x), numeric_value(y)) {
        return nx.partial_cmp(&ny).unwrap_or(Ordering::Equal);
    }
    x.value()
        .cmp(y.value())
        .then_with(|| x.datatype().as_str().cmp(y.datatype().as_str()))
        .then_with(|| x.language().unwrap_or("").cmp(y.language().unwrap_or("")))
}

/// The `f64` value of a numeric-XSD-typed literal, else `None` (a non-numeric
/// datatype is ordered lexically, never coerced).
fn numeric_value(l: &Literal) -> Option<f64> {
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
    let local = l.datatype().as_str().strip_prefix(XSD)?;
    let numeric = matches!(
        local,
        "integer"
            | "decimal"
            | "double"
            | "float"
            | "long"
            | "int"
            | "short"
            | "byte"
            | "nonNegativeInteger"
            | "nonPositiveInteger"
            | "negativeInteger"
            | "positiveInteger"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
    );
    if numeric {
        l.value().parse::<f64>().ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// SPARQL expression evaluator for ORDER BY expression keys
// ---------------------------------------------------------------------------
// Evaluates a SPARQL expression against a solution binding map, returning the
// result as an RDF term, or `None` on type error / unbound input. Covers the
// subset needed for ORDER BY expression keys: arithmetic, string built-ins,
// IF, BOUND, comparisons, COALESCE, boolean connectives. On unsupported
// sub-expressions returns None (unbound), which ORDER BY treats as sorting
// first/last per direction — sound but never silently wrong.

/// Evaluate a SPARQL expression to an RDF term, or `None` if indeterminate.
pub(crate) fn eval_expr(expr: &Expression, b: &BTreeMap<String, Term>) -> Option<Term> {
    match expr {
        Expression::Variable(v) => b.get(v.as_str()).cloned(),
        Expression::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        Expression::Literal(l) => Some(Term::Literal(l.clone())),
        Expression::Bound(v) => {
            let bound = b.contains_key(v.as_str());
            Some(Term::Literal(Literal::new_typed_literal(
                if bound { "true" } else { "false" },
                sf_core::NamedNode::new("http://www.w3.org/2001/XMLSchema#boolean").ok()?,
            )))
        }
        Expression::If(cond, then, els) => {
            if eval_bool(cond, b)? {
                eval_expr(then, b)
            } else {
                eval_expr(els, b)
            }
        }
        Expression::Coalesce(args) => args.iter().find_map(|a| eval_expr(a, b)),
        Expression::Not(a) => {
            let v = eval_bool(a, b)?;
            bool_literal(!v)
        }
        Expression::And(a, c) => {
            let av = eval_bool(a, b)?;
            let bv = eval_bool(c, b)?;
            bool_literal(av && bv)
        }
        Expression::Or(a, c) => {
            let av = eval_bool(a, b)?;
            let bv = eval_bool(c, b)?;
            bool_literal(av || bv)
        }
        Expression::Equal(a, c) => {
            let cmp = cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref());
            bool_literal(matches!(cmp, Some(Ordering::Equal)))
        }
        Expression::Less(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Less)
        )),
        Expression::Greater(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Greater)
        )),
        Expression::LessOrEqual(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Less | Ordering::Equal)
        )),
        Expression::GreaterOrEqual(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Greater | Ordering::Equal)
        )),
        Expression::Add(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x + y),
        Expression::Subtract(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x - y),
        Expression::Multiply(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x * y),
        Expression::Divide(a, c) => {
            let bv = term_to_f64(&eval_expr(c, b)?)?;
            if bv == 0.0 {
                return None;
            }
            num_binop(
                eval_expr(a, b)?,
                Term::Literal(Literal::new_simple_literal("0")),
                |x, _| x / bv,
            )
        }
        Expression::UnaryMinus(a) => {
            let v = term_to_f64(&eval_expr(a, b)?)?;
            f64_to_term(-v)
        }
        Expression::FunctionCall(func, args) => eval_function(func, args, b),
        _ => None,
    }
}

fn eval_bool(expr: &Expression, b: &BTreeMap<String, Term>) -> Option<bool> {
    match eval_expr(expr, b)? {
        Term::Literal(l) => {
            const XSD_BOOL: &str = "http://www.w3.org/2001/XMLSchema#boolean";
            if l.datatype().as_str() == XSD_BOOL {
                Some(l.value() == "true")
            } else {
                // Effective boolean value per SPARQL §17.2.2
                let v = l.value();
                Some(!v.is_empty() && v != "0" && v != "0.0" && v != "false")
            }
        }
        _ => None,
    }
}

fn bool_literal(v: bool) -> Option<Term> {
    Some(Term::Literal(Literal::new_typed_literal(
        if v { "true" } else { "false" },
        sf_core::NamedNode::new("http://www.w3.org/2001/XMLSchema#boolean").ok()?,
    )))
}

fn term_to_f64(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) => l.value().parse::<f64>().ok(),
        _ => None,
    }
}

fn f64_to_term(v: f64) -> Option<Term> {
    let code = if v.fract() == 0.0 && v.abs() < 1e15 {
        XsdTypeCode::Integer
    } else {
        XsdTypeCode::Double
    };
    natural_literal(&v.to_string(), code).ok()
}

fn num_binop(a: Term, b: Term, op: impl Fn(f64, f64) -> f64) -> Option<Term> {
    let av = term_to_f64(&a)?;
    let bv = term_to_f64(&b)?;
    f64_to_term(op(av, bv))
}

fn cmp_option(a: Option<&Term>, b: Option<&Term>) -> Option<Ordering> {
    Some(cmp_term(a?, b?))
}

fn eval_function(func: &Function, args: &[Expression], b: &BTreeMap<String, Term>) -> Option<Term> {
    fn str_val(t: &Term) -> Option<String> {
        match t {
            Term::Literal(l) => Some(l.value().to_owned()),
            _ => None,
        }
    }
    match func {
        Function::StrLen => {
            let t = eval_expr(args.first()?, b)?;
            let s = str_val(&t)?;
            natural_literal(&s.chars().count().to_string(), XsdTypeCode::Integer).ok()
        }
        Function::UCase => {
            let t = eval_expr(args.first()?, b)?;
            Some(Term::Literal(Literal::new_simple_literal(
                str_val(&t)?.to_uppercase(),
            )))
        }
        Function::LCase => {
            let t = eval_expr(args.first()?, b)?;
            Some(Term::Literal(Literal::new_simple_literal(
                str_val(&t)?.to_lowercase(),
            )))
        }
        Function::Str => {
            let t = eval_expr(args.first()?, b)?;
            let s = match &t {
                Term::Literal(l) => l.value().to_owned(),
                Term::NamedNode(n) => n.as_str().to_owned(),
                _ => return None,
            };
            Some(Term::Literal(Literal::new_simple_literal(s)))
        }
        Function::Concat => {
            let mut result = String::new();
            for arg in args {
                let t = eval_expr(arg, b)?;
                result.push_str(&str_val(&t)?);
            }
            Some(Term::Literal(Literal::new_simple_literal(result)))
        }
        Function::Lang => {
            let t = eval_expr(args.first()?, b)?;
            let lang = match &t {
                Term::Literal(l) => l.language().unwrap_or("").to_owned(),
                _ => String::new(),
            };
            Some(Term::Literal(Literal::new_simple_literal(lang)))
        }
        Function::Datatype => {
            let t = eval_expr(args.first()?, b)?;
            match &t {
                Term::Literal(l) => Some(Term::NamedNode(
                    sf_core::NamedNode::new(l.datatype().as_str()).ok()?,
                )),
                _ => None,
            }
        }
        Function::Abs => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.abs())
        }
        Function::Floor => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.floor())
        }
        Function::Ceil => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.ceil())
        }
        Function::Round => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.round())
        }
        _ => None,
    }
}

/// Stream the triples of a `CONSTRUCT` (or the `?s ?p ?o` dump), invoking `sink`
/// per well-formed triple. Ill-formed instantiations (e.g. a literal subject) are
/// skipped, per SPARQL CONSTRUCT semantics. Runs the driver-agnostic
/// [`crate::exec_core`] core over a live SQLite connection (ADR-0024).
pub fn construct(plan: &Plan, conn: &Connection, sink: impl FnMut(Triple)) -> Result<u64> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::construct(plan, &mut backend, sink))
}

/// Collect a CONSTRUCT's triples (test/diagnostic convenience; the streaming
/// [`construct`] is the bounded-memory API).
pub fn construct_triples(plan: &Plan, conn: &Connection) -> Result<Vec<Triple>> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::construct_triples(plan, &mut backend))
}

/// Stream the whole mapping as **quads** (ADR-0005 named-graph conformance),
/// invoking `sink` per well-formed quad. Distinct from the `?s ?p ?o` CONSTRUCT
/// dump: it walks the mapping IR ([`crate::dump`]) so each triple carries the
/// graph term from the applicable `rr:graphMap`(s), built through the *same*
/// `sf-core` term-gen path. Bounded-memory: one row in flight via
/// [`crate::exec_core`]. A triple whose subject/predicate/object column is NULL is
/// dropped (R2RML §11); a named-graph branch whose graph map produces no value
/// drops that quad (no silent default-graph fallback).
pub fn dump_quads_stream(
    maps: &[sf_core::ir::TriplesMap],
    conn: &Connection,
    dialect: sf_sql::Dialect,
    sink: impl FnMut(sf_core::Quad),
) -> Result<()> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::dump_quads_stream(
        maps,
        &mut backend,
        dialect,
        sink,
    ))
}

/// Collect the mapping-IR quad dump (conformance convenience; the streaming
/// [`dump_quads_stream`] is the bounded-memory API).
pub fn dump_quads(
    maps: &[sf_core::ir::TriplesMap],
    conn: &Connection,
    dialect: sf_sql::Dialect,
) -> Result<Vec<sf_core::Quad>> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::dump_quads(maps, &mut backend, dialect))
}

/// A SELECT solution: the projected variables (plan order) paired with each
/// row's bound terms (`None` = unbound).
pub struct Solutions {
    pub vars: Vec<String>,
    pub rows: Vec<Vec<Option<Term>>>,
}

/// Stream a SELECT's solutions, invoking `sink` per projected row (in projection
/// order, `None` = unbound) — one row in flight (bounded memory). The HTTP layer
/// drives this to serialise + flush each row without collecting (ADR-0010 §C).
pub fn select_each(
    plan: &Plan,
    conn: &Connection,
    sink: impl FnMut(&[Option<Term>]) -> Result<()>,
) -> Result<()> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::select_each(plan, &mut backend, sink))
}

/// Execute a SELECT, collecting solutions (bounded-memory streaming is the
/// [`crate::exec_core`] core; this collects for callers/tests).
pub fn select(plan: &Plan, conn: &Connection) -> Result<Solutions> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::select(plan, &mut backend))
}

/// Execute an ASK — true iff at least one solution exists.
pub fn ask(plan: &Plan, conn: &Connection) -> Result<bool> {
    let mut backend = sf_sql::backend::sqlite::SqliteBackend::new(conn);
    crate::exec_core::block_on(crate::exec_core::ask(plan, &mut backend))
}

/// Instantiate a CONSTRUCT-template triple against a solution; `None` if any
/// variable is unbound or the triple would be ill-formed. `pub(crate)` so the
/// PostgreSQL executor instantiates CONSTRUCT templates identically.
pub(crate) fn instantiate(
    tp: &spargebra::term::TriplePattern,
    bindings: &BTreeMap<String, Term>,
) -> Option<Triple> {
    use spargebra::term::{NamedNodePattern, TermPattern};
    let term = |p: &TermPattern| -> Option<Term> {
        match p {
            TermPattern::Variable(v) => bindings.get(v.as_str()).cloned(),
            TermPattern::NamedNode(n) => Some(Term::NamedNode(n.clone())),
            TermPattern::Literal(l) => Some(Term::Literal(l.clone())),
            TermPattern::BlankNode(b) => Some(Term::BlankNode(b.clone())),
            _ => None,
        }
    };
    let subject = term(&tp.subject)?;
    let predicate = match &tp.predicate {
        NamedNodePattern::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedNodePattern::Variable(v) => bindings.get(v.as_str()).cloned()?,
    };
    let object = term(&tp.object)?;
    Triple::from_terms(subject, predicate, object).ok()
}

/// Serialise triples as N-Triples 1.2 (ADR-0019 G1: triple-term graphs serialise
/// as N-Triples/Turtle, not JSON-LD). One triple per line; streamed.
pub fn write_ntriples(triples: &[Triple]) -> String {
    let mut out = String::new();
    for t in triples {
        out.push_str(&t.to_string());
        out.push_str(" .\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Rust-level GROUP BY (multi-branch inner, SPARQL §11)
// ---------------------------------------------------------------------------

/// Group a collected multiset of inner solutions by `rg.keys`, compute each
/// aggregate in `rg.aggs`, then apply the plan's ORDER BY + OFFSET/LIMIT to the
/// grouped rows (SPARQL §15: order, then slice). Returns the final result rows in
/// emit order.
///
/// Shared by the SQLite ([`rust_group_execute`]) and PostgreSQL
/// ([`crate::exec_pg`]) multi-branch GROUP BY paths (ADR-0007): the
/// grouping/aggregation semantics are backend-independent — only the collection of
/// the inner solutions (SQLite `Connection` vs live PostgreSQL cursor) differs.
pub(crate) fn rust_group_result_rows(
    plan: &Plan,
    rg: &RustGroup,
    inner_rows: Vec<BTreeMap<String, Term>>,
) -> Result<Vec<BTreeMap<String, Term>>> {
    // Group by the key variable values, preserving insertion order for stable output.
    // Use a Vec for ordering + a HashMap for O(1) group lookup.
    type GroupKey = Vec<Option<Term>>;
    type GroupRows = Vec<BTreeMap<String, Term>>;
    #[allow(clippy::type_complexity)]
    let mut groups: Vec<(GroupKey, GroupRows)> = Vec::new();
    let mut key_index: std::collections::HashMap<Vec<Option<Term>>, usize> =
        std::collections::HashMap::new();

    for row in inner_rows {
        let key: Vec<Option<Term>> = rg.keys.iter().map(|k| row.get(k).cloned()).collect();
        if let Some(&idx) = key_index.get(&key) {
            groups[idx].1.push(row);
        } else {
            let idx = groups.len();
            key_index.insert(key.clone(), idx);
            groups.push((key, vec![row]));
        }
    }

    // Implicit grouping (no key variables): always produce exactly one group,
    // even over an empty inner (COUNT(*) ⇒ 0, AVG/MIN/MAX ⇒ UNBOUND — §11).
    if rg.keys.is_empty() && groups.is_empty() {
        groups.push((vec![], vec![]));
    }

    // Materialise the result row (key vars + aggregates) for every group.
    let mut result_rows: Vec<BTreeMap<String, Term>> = Vec::with_capacity(groups.len());
    for (key_vals, group_rows) in &groups {
        let mut result = BTreeMap::new();
        for (k, val) in rg.keys.iter().zip(key_vals.iter()) {
            if let Some(t) = val {
                result.insert(k.clone(), t.clone());
            }
        }
        for agg_spec in &rg.aggs {
            if let Some(t) = rust_agg(agg_spec, group_rows)? {
                result.insert(agg_spec.out_var.clone(), t);
            }
        }
        result_rows.push(result);
    }

    // ORDER BY over the grouped rows (if requested), then OFFSET/LIMIT.
    if !plan.order.is_empty() {
        result_rows.sort_by(|a, b| order_cmp(&plan.order, a, b));
    }
    let take = plan.limit.unwrap_or(usize::MAX);
    Ok(result_rows
        .into_iter()
        .skip(plan.offset)
        .take(take)
        .collect())
}

/// Compute one aggregate over a group of solutions. Returns `None` for
/// UNBOUND (AVG/MIN/MAX over an empty multiset — SPARQL §11).
fn rust_agg(agg: &RustAgg, rows: &[BTreeMap<String, Term>]) -> Result<Option<Term>> {
    // Collect bound numeric values of the argument variable.
    let _bound_vals: Vec<&Term> = match &agg.arg_var {
        None => rows.iter().flat_map(|r| r.values()).collect(), // COUNT(*) — not used for numerics
        Some(var) => rows.iter().filter_map(|r| r.get(var)).collect(),
    };

    match agg.kind {
        AggKind::Count => {
            let count = match &agg.arg_var {
                None => rows.len(), // COUNT(*)
                Some(var) => {
                    if agg.distinct {
                        let mut seen: std::collections::HashSet<String> =
                            std::collections::HashSet::new();
                        rows.iter()
                            .filter_map(|r| r.get(var))
                            .filter(|t| seen.insert(t.to_string()))
                            .count()
                    } else {
                        rows.iter().filter(|r| r.contains_key(var.as_str())).count()
                    }
                }
            };
            Ok(Some(Term::Literal(Literal::new_typed_literal(
                count.to_string(),
                sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
            ))))
        }
        AggKind::Sum => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                // SUM over empty multiset ⇒ "0"^^xsd:integer (SPARQL §11).
                return Ok(Some(Term::Literal(Literal::new_typed_literal(
                    "0",
                    sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
                ))));
            }
            let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
            if nums.len() < vals.len() {
                return Ok(None); // non-numeric operand ⇒ UNBOUND (type error)
            }
            let sum: f64 = nums.iter().sum();
            if vals.iter().all(|t| is_xsd_integer(t)) {
                Ok(Some(integer_term(sum as i64)))
            } else {
                Ok(Some(decimal_term(sum)))
            }
        }
        AggKind::Avg => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                // AVG over an empty multiset ⇒ "0"^^xsd:integer (SPARQL §11, like SUM —
                // NOT UNBOUND; the spareval oracle confirms 0).
                return Ok(Some(integer_term(0)));
            }
            let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
            if nums.is_empty() {
                return Ok(None); // non-numeric operand ⇒ UNBOUND (type error, §11)
            }
            let avg = nums.iter().sum::<f64>() / nums.len() as f64;
            Ok(Some(decimal_term(avg)))
        }
        AggKind::Min | AggKind::Max => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                return Ok(None); // UNBOUND for empty multiset (§11)
            }
            let result = if agg.kind == AggKind::Min {
                vals.iter().min_by(|a, b| cmp_term(a, b))
            } else {
                vals.iter().max_by(|a, b| cmp_term(a, b))
            };
            Ok(result.map(|t| (*t).clone()))
        }
    }
}

/// Extract the `f64` numeric value of an RDF term (returns `None` for
/// non-numeric-typed literals and non-literals).
fn numeric_term(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) => numeric_value(l),
        _ => None,
    }
}

/// Whether an RDF term is an `xsd:integer`-typed literal.
fn is_xsd_integer(t: &Term) -> bool {
    match t {
        Term::Literal(l) => l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#integer",
        _ => false,
    }
}

/// Build an `xsd:integer` literal from an `i64`.
fn integer_term(n: i64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        n.to_string(),
        sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
    ))
}

/// Build an `xsd:decimal` literal from an `f64`.
fn decimal_term(n: f64) -> Term {
    // Use a compact decimal representation (avoid scientific notation).
    let s = if n.fract() == 0.0 {
        format!("{n:.1}")
    } else {
        format!("{n}")
    };
    Term::Literal(Literal::new_typed_literal(
        s,
        sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#decimal"),
    ))
}
