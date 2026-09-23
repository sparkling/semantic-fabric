//! Raw-row term reconstruction and compact solution bindings.

// --- single-homed term-gen helpers (relocated from exec.rs, ADR-0024 M5) ------

/// `(alias, column) -> index` into a branch's fixed row schema — built ONCE per
/// branch ([`build_col_index`]), since the projection schema doesn't change row
/// to row, so [`RawRow`]'s per-row column lookups are an O(log n) binary search
/// instead of an O(n) `schema.iter().position(...)` scan per var, per row
/// (ADR-0024/M4 perf). A SORTED `Vec` + binary search, not a `HashMap`: a
/// branch's schema is typically a handful of columns, small enough that a
/// `HashMap`'s constant-factor overhead (table allocation, `SipHash` over the
/// `(usize, &str)` key) measurably LOST to the plain linear scan in a criterion
/// bench — a sorted `Vec` avoids both the allocation and the hashing while still
/// beating an O(n) scan once a branch's schema is large (e.g. a multi-table join).
/// Duplicate names resolve to the first projection position, matching the
/// historical linear lookup instead of unstable-sort-dependent tie selection.
pub(super) type ColIndex<'a> = Vec<((usize, &'a str), usize)>;

/// Raw test oracle: original position breaks ties, preserving first occurrence.
#[cfg(test)]
pub(super) fn build_col_index(schema: &[ColRef]) -> ColIndex<'_> {
    let mut index: ColIndex<'_> = schema
        .iter()
        .enumerate()
        .map(|(i, c)| ((c.alias, &*c.column), i))
        .collect();
    index.sort_unstable_by_key(|&(key, position)| (key, position));
    index
}

/// Borrow names and retain projection positions. Stable upper-bound insertion
/// pays comparisons, growth and shifts before they happen; no infallible sort
/// comparator is forced to continue after the request has stopped.
pub(super) fn build_col_index_controlled<'a>(
    schema: &'a [ColRef],
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<ColIndex<'a>> {
    use super::sql_error::map_sql_err;
    let mut out: sf_sql::source_work::SourceVec<((usize, &str), usize)> = Default::default();
    for (position, col) in schema.iter().enumerate() {
        work.charge(1).map_err(map_sql_err)?;
        let (mut low, mut high) = (0, out.as_slice().len());
        while low < high {
            work.charge(1).map_err(map_sql_err)?;
            let mid = low + (high - low) / 2;
            let ((alias, name), _) = out.as_slice()[mid];
            let mut order = alias.cmp(&col.alias);
            if order.is_eq() {
                work.charge(name.len().min(col.column.len()))
                    .map_err(map_sql_err)?;
                order = name.cmp(&col.column);
            }
            if order.is_gt() {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        out.insert(low, ((col.alias, &col.column), position), work)
            .map_err(map_sql_err)?;
    }
    work.checkpoint().map_err(map_sql_err)?;
    Ok(out.into_vec())
}

/// Look up `(alias, column)` in a [`ColIndex`] via binary search.
fn col_index_get(index: &ColIndex<'_>, alias: usize, column: &str) -> Option<usize> {
    let key = (alias, column);
    let position = index.partition_point(|&(candidate, _)| candidate < key);
    index
        .get(position)
        .filter(|&&(candidate, _)| candidate == key)
        .map(|entry| entry.1)
}

/// One projected result row's raw column values plus each value's resolved §10
/// type (declared type, else storage-class fallback), addressed by [`ColRef`] via
/// a precomputed [`ColIndex`]. `pub(crate)` so the PostgreSQL executor
/// ([`crate::exec_pg`]) drives the same single term-gen path (ADR-0003 R3) with
/// PG-extracted values.
pub(crate) struct RawRow<'a> {
    pub(crate) values: &'a [Option<String>],
    pub(crate) codes: &'a [Option<XsdTypeCode>],
    pub(crate) index: &'a ColIndex<'a>,
}

impl RawRow<'_> {
    /// The resolved §10 XSD type of `column` under `alias`, if any.
    fn code_for(&self, alias: usize, column: &str) -> Option<XsdTypeCode> {
        col_index_get(self.index, alias, column).and_then(|i| self.codes[i])
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
        col_index_get(self.raw.index, self.alias, column)
            .and_then(|i| self.raw.values[i].as_deref())
    }
}

/// Materialise a term definition into an `oxrdf` term, or `None` if a referenced
/// column is NULL/absent (R2RML §11: no value ⇒ no term ⇒ unbound).
fn build_term(def: &TermDef, raw: &RawRow<'_>, work: TermWork<'_>) -> Result<Option<Term>> {
    work.charge(1)?;
    match def {
        TermDef::Const(t) => Ok(Some(t.clone())),
        TermDef::Derived { term_map, alias } => derived_term(term_map, *alias, raw, work),
        TermDef::R2rmlBlank {
            term_map,
            alias,
            graph,
        } => {
            let Some(base) = derived_term(term_map, *alias, raw, work)? else {
                return Ok(None);
            };
            let Term::BlankNode(blank) = base else {
                return Err(Error::Core(
                    "R2rmlBlank IQ recipe generated a non-blank RDF term".to_owned(),
                ));
            };
            let graph_iri = match graph {
                R2rmlGraphScope::Default => None,
                R2rmlGraphScope::Mapped { term_map, alias } => {
                    let Some(graph_term) = derived_term(term_map, *alias, raw, work)? else {
                        return Ok(None);
                    };
                    let Term::NamedNode(graph) = graph_term else {
                        return Ok(None);
                    };
                    (graph.as_str() != RR_DEFAULT_GRAPH).then_some(graph)
                }
            };
            let mut label = if graph_iri.is_some() {
                String::from("sfr1n_")
            } else {
                String::from("sfr1d_")
            };
            if let Some(graph) = graph_iri {
                // Hex labelling doubles each source byte; charge that growth
                // before the label is built, plus the separator.
                work.product(graph.as_str().len(), HEX_LABEL_WIDTH)?;
                work.charge(1)?;
                super::push_hex(&mut label, graph.as_str().as_bytes());
                label.push('_');
            }
            work.product(blank.as_str().len(), HEX_LABEL_WIDTH)?;
            super::push_hex(&mut label, blank.as_str().as_bytes());
            Ok(Some(Term::BlankNode(sf_core::BlankNode::new_unchecked(
                label,
            ))))
        }
        // R2 COALESCE: the preserved (left) side wins when bound; otherwise the
        // optional (right) value (ADR-0007). `None` from `left` = its source
        // columns were NULL (the optional did not match), so fall back to `right`.
        TermDef::Coalesce(l, r) => match build_term(l, raw, work)? {
            Some(t) => Ok(Some(t)),
            None => build_term(r, raw, work),
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
                let Some(Term::Literal(l)) = build_term(p, raw, work)? else {
                    return Ok(None);
                };
                let lang = l.language();
                if lang.is_none() && l.datatype() != sf_core::vocab::xsd::STRING {
                    return Ok(None); // a non-string typed literal ⇒ type error
                }
                work.charge(l.value().len())?;
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
                // A NULL SQL aggregate value on the single-branch SQL-pushdown path. ADR-0025
                // C.7: SUM/AVG/COUNT over an EMPTY group ⇒ "0"^^xsd:integer (SPARQL §11); only
                // MIN/MAX of an empty multiset are UNBOUND. This is sound HERE specifically
                // because ADR-0025 C.6 routes any NULLABLE-operand aggregate to `rust_group` —
                // so on this SQL path the operand is MANDATORY (bound in every row), hence a
                // NULL aggregate value means 0 rows (empty group), never "non-empty but all
                // operands unbound" (which must be UNBOUND and is handled correctly by
                // `rust_agg` C.4/C.5). Pre-C.6 this branch conflated the two for AVG.
                return match kind {
                    AggKind::Sum | AggKind::Count | AggKind::Avg => {
                        work.charge(1 + natural_lexical_growth_allowance(XsdTypeCode::Integer))?;
                        Ok(Some(natural_literal("0", XsdTypeCode::Integer)?))
                    }
                    AggKind::Min | AggKind::Max => Ok(None),
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
            work.charge(value.len() + natural_lexical_growth_allowance(code))?;
            Ok(Some(natural_literal(value, code)?))
        }
        // ADR-0032 D2 — the ONLY route by which this engine ever produces a native
        // `Term::Triple`: recursively realize the three components, then compose via
        // `Triple::from_terms`, which is fallible and enforces RDF 1.2 §3.1 position
        // legality (subject IRI/bnode, predicate IRI) for free. A failed composition
        // (illegal shape) OR an unbound component ⇒ unbound (`None`) — never an error,
        // matching SPARQL's usual "error in construction ⇒ unbound" discipline at
        // projection. Deliberately bypasses `sf_core::term::generate` (`GenTerm` has
        // no triple arm by design, ADR-0006 zero-alloc — see the module-level note on
        // `TermDef::ComposedTriple`).
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            let (Some(s), Some(p), Some(o)) = (
                build_term(subject, raw, work)?,
                build_term(predicate, raw, work)?,
                build_term(object, raw, work)?,
            ) else {
                return Ok(None);
            };
            Ok(Triple::from_terms(s, p, o).ok().map(Term::from))
        }
    }
}

/// Build a derived term, applying the R2RML §10 natural datatype mapping
/// (ADR-0015) to column literals without an override, including an explicit
/// datatype equal to the resolved source datatype (R2RML §11.2). Templates,
/// IRIs, blank nodes, different datatypes and language retain their own path.
fn derived_term(
    term_map: &TermMap,
    alias: usize,
    raw: &RawRow<'_>,
    work: TermWork<'_>,
) -> Result<Option<Term>> {
    if let TermMap::Column(col, spec) = term_map {
        if spec.uses_natural_type(raw.code_for(alias, col)) {
            let row = AliasRow { raw, alias };
            let Some(value) = row.value(col) else {
                return Ok(None);
            };
            let code = raw.code_for(alias, col).unwrap_or(XsdTypeCode::String);
            work.charge(value.len() + natural_lexical_growth_allowance(code))?;
            return Ok(Some(natural_literal(value, code)?));
        }
    }
    let row = AliasRow { raw, alias };
    sf_core::term::generate_controlled(term_map, &row, work).map_err(map_core_err)
}

/// Produce the RDF literal for a value under its §10 natural XSD type, in the
/// XSD-canonical lexical form (ADR-0015 chokepoint, `sf_core::datatype`).
/// `HexBinary` values arrive already uppercase-hex-encoded from blob extraction.
pub(super) fn natural_literal(value: &str, code: XsdTypeCode) -> Result<Term> {
    let literal = match code {
        XsdTypeCode::String => Literal::new_simple_literal(value),
        XsdTypeCode::HexBinary => Literal::new_typed_literal(value, code.iri()),
        _ => {
            let mut buf = String::new();
            datatype::natural_lexical(value, code, &mut buf)
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

/// One reconstructed SPARQL solution row's bound-variable -> term mapping
/// (Run 4 Wave C1, replacing the former `BTreeMap<String, Term>`): a small
/// linear-scan `Vec`, not a tree. `sf-bench`'s `constant_memory` peak-heap
/// profiling (see [`TERM_GEN_BATCH_SIZE`]'s doc comment) found `BTreeMap`'s
/// per-node allocation overhead — not the term data itself — dominated peak
/// heap in the buffered-batch window, because a typical branch binds only a
/// handful (1-3) of variables per row: far below where a tree's O(log n)
/// lookup would ever beat a linear scan (the same reasoning [`ColIndex`]
/// documents for a branch's column schema). Var names are `Arc<str>`, not
/// `String`: every row [`reconstruct`] builds for one branch's stream shares
/// that branch's SAME interned handles ([`intern_bindings`]), so a per-row
/// insert clones an `Arc` (refcount bump) instead of allocating a fresh
/// `String`.
///
/// Preserves INSERTION order, NOT the old `BTreeMap`'s alphabetical-by-key
/// order. [`Bindings::get`]/[`contains_key`](Bindings::contains_key) (keyed
/// lookup) are unaffected by this, but a site that needs a canonical,
/// order-independent view of the WHOLE row — hashing it or structurally
/// comparing it, as opposed to looking up one named variable — must go
/// through [`canonical_pairs`] first, or two equal solutions whose vars
/// happened to get bound/inserted in a different sequence would compare
/// unequal. The `derive`d [`PartialEq`] below is therefore ALSO
/// insertion-order sensitive (structural, element-by-element) — fine for the
/// one place this file compares `Bindings` values directly
/// (`order_sort_key_tests`, where both sides are clones of the same original
/// rows, never rebuilt), but not a substitute for [`canonical_pairs`]
/// anywhere a value could have been built along a different path.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Bindings(Vec<(Arc<str>, Term)>);

impl Bindings {
    pub(super) fn new() -> Self {
        Bindings(Vec::new())
    }

    /// The term bound to `var`, if any.
    pub(super) fn get(&self, var: &str) -> Option<&Term> {
        self.0.iter().find(|(k, _)| &**k == var).map(|(_, v)| v)
    }

    pub(super) fn contains_key(&self, var: &str) -> bool {
        self.0.iter().any(|(k, _)| &**k == var)
    }

    /// `BTreeMap::insert`'s replace-on-existing-key semantics: overwrite
    /// `var`'s slot if already bound, else append a new one.
    pub(super) fn insert(&mut self, var: Arc<str>, term: Term) {
        match self.0.iter_mut().find(|(k, _)| *k == var) {
            Some(slot) => slot.1 = term,
            None => self.0.push((var, term)),
        }
    }

    /// Append `(var, term)` WITHOUT checking for an existing key — sound only
    /// when the caller already guarantees `var` is not yet bound. Prefer
    /// [`Bindings::insert`] anywhere that isn't true; [`reconstruct`] is the
    /// one caller that can (its `interned` source is unique-by-construction,
    /// see [`intern_bindings`]).
    fn push(&mut self, var: Arc<str>, term: Term) {
        self.0.push((var, term));
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&str, &Term)> {
        self.0.iter().map(|(k, v)| (&**k, v))
    }

    /// Exact textual payload bytes retained by this solution. Fixed container
    /// overhead is bounded separately by the admitted row window.
    pub(super) fn retained_payload_bytes(&self) -> Option<u64> {
        self.0.iter().try_fold(0_u64, |total, (name, term)| {
            total
                .checked_add(u64::try_from(name.len()).ok()?)?
                .checked_add(term_payload_bytes(term)?)
        })
    }
}

pub(super) fn term_payload_bytes(term: &Term) -> Option<u64> {
    fn length(value: &str) -> Option<u64> {
        u64::try_from(value.len()).ok()
    }
    match term {
        Term::NamedNode(node) => length(node.as_str()),
        Term::BlankNode(node) => length(node.as_str()),
        Term::Literal(literal) => length(literal.value())?
            .checked_add(length(literal.datatype().as_str())?)?
            .checked_add(length(literal.language().unwrap_or(""))?),
        Term::Triple(triple) => {
            let subject = match &triple.subject {
                sf_core::NamedOrBlankNode::NamedNode(node) => length(node.as_str())?,
                sf_core::NamedOrBlankNode::BlankNode(node) => length(node.as_str())?,
            };
            subject
                .checked_add(length(triple.predicate.as_str())?)?
                .checked_add(term_payload_bytes(&triple.object)?)
        }
    }
}

/// [`Bindings`]'s pairs in CANONICAL (var-name-sorted) order — see
/// [`Bindings`]'s doc comment for why any whole-row hash/structural-equality
/// site needs this instead of raw [`Bindings::iter`] order. The two sites
/// that hash a FULL solution row rather than looking up one named variable:
/// `run_branches`' ADR-0034 D1 term-dedup key, and `rust_agg`'s
/// `COUNT(DISTINCT *)` key.
pub(super) fn canonical_pairs(b: &Bindings) -> Vec<(&str, &Term)> {
    let mut pairs: Vec<(&str, &Term)> = b.iter().collect();
    pairs.sort_unstable_by_key(|&(k, _)| k);
    pairs
}

/// [`Branch::bindings`]'s variable names, pre-interned as [`Arc<str>`] and
/// paired with their [`TermDef`] — built ONCE per branch (`run_branches`,
/// mirroring [`build_col_index`]'s "once per branch, not per row" idiom, see
/// its doc comment). Every row [`reconstruct`] builds for this branch then
/// clones an already-allocated `Arc` (a refcount bump) into its [`Bindings`]
/// instead of allocating a fresh `String` per variable per row (Run 4 Wave
/// C1 — the ADR-0006 correction note's "leaner per-row binding
/// representation"). Does NOT touch [`crate::iq::Branch::bindings`], which stays
/// a `BTreeMap<String, TermDef>` — its alphabetical iteration order is
/// load-bearing elsewhere (`iq::lower`'s positional `c{i}` alias assignment).
pub(crate) type InternedBindings<'a> = Vec<(Arc<str>, &'a TermDef)>;

#[cfg(test)]
pub(super) fn intern_bindings(branch: &crate::iq::Branch) -> InternedBindings<'_> {
    branch
        .bindings
        .iter()
        .map(|(var, def)| (Arc::from(var.as_str()), def))
        .collect()
}

#[cfg(test)]
pub(super) fn intern_bindings_controlled<'a>(
    branch: &'a crate::iq::Branch,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<InternedBindings<'a>> {
    intern_binding_view(&crate::emit::BindingView::Direct(&branch.bindings), work)
}

pub(super) fn intern_binding_view<'a>(
    bindings: &crate::emit::BindingView<'a>,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<InternedBindings<'a>> {
    use super::sql_error::map_sql_err;
    let mut out = work.vector(bindings.len()).map_err(map_sql_err)?;
    for (name, definition) in bindings.iter() {
        work.charge(1).map_err(map_sql_err)?;
        work.charge(name.len()).map_err(map_sql_err)?;
        work.charge(2 * std::mem::size_of::<usize>())
            .map_err(map_sql_err)?;
        out.push((Arc::from(name), definition));
    }
    work.checkpoint().map_err(map_sql_err)?;
    Ok(out)
}

/// Reconstruct all bound variables of one raw row from `interned` — a
/// branch's [`intern_bindings`] output, built ONCE per branch (see its doc
/// comment).
///
/// The uncontrolled reference shape, kept as
/// [`reconstruct_controlled`]'s oracle in `batch_reconstruct_tests`. Production
/// execution — including the PostgreSQL executor, which reaches this path
/// through the backend-generic controlled driver (ADR-0003 R3) — goes through
/// [`reconstruct_controlled`] so a request's budget and cancellation govern
/// per-row term generation.
#[cfg_attr(not(test), expect(dead_code, reason = "uncontrolled test oracle"))]
pub(crate) fn reconstruct(interned: &InternedBindings<'_>, raw: &RawRow<'_>) -> Result<Bindings> {
    reconstruct_controlled(interned, raw, TermWork::uncontrolled())
}

/// [`reconstruct`] under the governing request's term-generation budget.
///
/// The per-row half of source governance: the branch's column index and
/// interned binding names are charged once per branch at setup
/// ([`build_col_index_controlled`], [`intern_binding_view`]), and this charges
/// what every row then costs — one unit per candidate binding plus each term's
/// own generated width, through [`build_term`]. The leading `checkpoint` is the
/// in-batch stop point: a request cancelled or past its deadline stops at the
/// next row, not only at the next batch boundary.
///
/// Charging is per row and order-independent, so a batch reconstructed in
/// parallel chunks accrues exactly the total a sequential pass would
/// (`QueryBudget::consume` is a CAS loop over atomics, and
/// [`super::batch::reconstruct_batch`] preserves row order either way).
pub(crate) fn reconstruct_controlled(
    interned: &InternedBindings<'_>,
    raw: &RawRow<'_>,
    work: TermWork<'_>,
) -> Result<Bindings> {
    work.checkpoint()?;
    let mut out = Bindings::new();
    for (var, def) in interned {
        work.charge(1)?;
        if let Some(term) = build_term(def, raw, work)? {
            // `push`, not `insert`: `interned` comes from a `BTreeMap` (unique
            // keys), so `var` can never already be bound in `out`.
            out.push(var.clone(), term);
        }
    }
    Ok(out)
}
use std::sync::Arc;

use sf_core::datatype::{self, XsdTypeCode};
use sf_core::ir::TermMap;
use sf_core::term_work::TermWork;
use sf_core::{Literal, Row, Term, Triple};

use crate::graph_map::RR_DEFAULT_GRAPH;

/// Hex labelling writes two characters per source byte ([`super::push_hex`]);
/// the prepaid bound on an `R2rmlBlank` label's growth.
const HEX_LABEL_WIDTH: usize = 2;

/// A prepaid upper bound on the width [`natural_literal`] actually BUILDS for a
/// value of `code`, which is NOT its raw input width: §10 canonicalization
/// (`sf_core::datatype::canonical_lexical`) can EXPAND a value, so charging
/// `value.len()` alone would undercharge the literal that gets allocated.
/// `sf_core::datatype`'s own tests show it: `"0"` becomes `"false"` (1 -> 5) and
/// `"100"` becomes `"1.0E2"` (3 -> 5). This mirrors the decode-side discipline
/// in `sf_sql::backend::pg::decode`, which charges a fixed worst case per SQL
/// type (`Type::BOOL => 5`) rather than the raw byte width.
///
/// Returned as an ADDEND charged on top of `value.len()`, not a replacement:
/// codes that pass through or shrink (String verbatim, HexBinary already
/// uppercase-encoded, Integer/Decimal which only strip zeros and signs) stay
/// bounded by the input and add nothing, while a code that can grow adds the
/// most its canonical form can exceed a one-byte input by.
///
/// - `Boolean`: `"false"` is the widest canonical form (5).
/// - `Double`: canonical E-notation with a mandatory fractional digit; the
///   widest `{:E}` rendering of any finite `f64` plus the `.0` pad is 26
///   (measured at `-2.2250738585072014E-308`); `NaN`/`INF`/`-INF` are shorter.
/// - `Date`/`Time`/`DateTime`: `oxsdatatypes` canonical forms carry an optional
///   timezone and an expanded year, so allow the widest such rendering (32).
pub(super) const fn natural_lexical_growth_allowance(code: XsdTypeCode) -> usize {
    match code {
        // Verbatim or already-encoded: output is exactly the input.
        XsdTypeCode::String | XsdTypeCode::HexBinary => 0,
        // `cast_display::<Integer>` only strips a leading `+`/`-0` and leading
        // zeros, so the output never exceeds the input.
        XsdTypeCode::Integer => 0,
        // `decimal::write_canonical` trims leading integral zeros and trailing
        // fraction zeros, but SUPPLIES a leading `0` when the integral part is
        // empty: `".5"` becomes `"0.5"` and `"-.5"` becomes `"-0.5"`, one byte
        // wider than the input. That single inserted digit is the only way this
        // path can grow (measured across leading-dot, signed and zero forms).
        XsdTypeCode::Decimal => 1,
        XsdTypeCode::Boolean => 5,
        XsdTypeCode::Double => 26,
        // `cast_display` re-renders the parsed value; measured across expanded
        // years, timezones and fractional seconds these never exceed their
        // input, so this is a deliberately generous bound rather than a tight
        // one — it may overcharge slightly, and must never undercharge.
        XsdTypeCode::Date | XsdTypeCode::Time | XsdTypeCode::DateTime => 32,
    }
}

/// Map a `sf-core` term-generation failure, preserving a governing control's
/// terminal cause as this crate's own control error instead of flattening it
/// into an opaque core-error string.
fn map_core_err(error: sf_core::Error) -> Error {
    match error {
        sf_core::Error::Control(cause) => Error::QueryControl(cause),
        other => Error::Core(other.to_string()),
    }
}
use crate::iq::{AggKind, ColRef, R2rmlGraphScope, TermDef};
use crate::{Error, Result};

#[cfg(test)]
#[path = "row/tests.rs"]
mod tests;
