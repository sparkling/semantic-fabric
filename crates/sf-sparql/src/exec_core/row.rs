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
pub(super) type ColIndex<'a> = Vec<((usize, &'a str), usize)>;

/// Build a [`ColIndex`] over a branch's projection schema (see its doc comment).
pub(super) fn build_col_index(schema: &[ColRef]) -> ColIndex<'_> {
    let mut index: ColIndex<'_> = schema
        .iter()
        .enumerate()
        .map(|(i, c)| ((c.alias, &*c.column), i))
        .collect();
    index.sort_unstable_by_key(|&(key, _)| key);
    index
}

/// Look up `(alias, column)` in a [`ColIndex`] via binary search.
fn col_index_get(index: &ColIndex<'_>, alias: usize, column: &str) -> Option<usize> {
    index
        .binary_search_by_key(&(alias, column), |&(key, _)| key)
        .ok()
        .map(|pos| index[pos].1)
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
fn build_term(def: &TermDef, raw: &RawRow<'_>) -> Result<Option<Term>> {
    match def {
        TermDef::Const(t) => Ok(Some(t.clone())),
        TermDef::Derived { term_map, alias } => derived_term(term_map, *alias, raw),
        TermDef::R2rmlBlank {
            term_map,
            alias,
            graph,
        } => {
            let Some(base) = derived_term(term_map, *alias, raw)? else {
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
                    let Some(graph_term) = derived_term(term_map, *alias, raw)? else {
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
                super::push_hex(&mut label, graph.as_str().as_bytes());
                label.push('_');
            }
            super::push_hex(&mut label, blank.as_str().as_bytes());
            Ok(Some(Term::BlankNode(sf_core::BlankNode::new_unchecked(
                label,
            ))))
        }
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
                build_term(subject, raw)?,
                build_term(predicate, raw)?,
                build_term(object, raw)?,
            ) else {
                return Ok(None);
            };
            Ok(Triple::from_terms(s, p, o).ok().map(Term::from))
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
pub(super) fn natural_literal(value: &str, code: XsdTypeCode) -> Result<Term> {
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
/// representation"). Does NOT touch [`Branch::bindings`] itself, which stays
/// a `BTreeMap<String, TermDef>` — its alphabetical iteration order is
/// load-bearing elsewhere (`iq::lower`'s positional `c{i}` alias assignment).
pub(crate) type InternedBindings<'a> = Vec<(Arc<str>, &'a TermDef)>;

pub(super) fn intern_bindings(branch: &Branch) -> InternedBindings<'_> {
    branch
        .bindings
        .iter()
        .map(|(var, def)| (Arc::from(var.as_str()), def))
        .collect()
}

/// Reconstruct all bound variables of one raw row from `interned` — a
/// branch's [`intern_bindings`] output, built ONCE per branch (see its doc
/// comment). `pub(crate)` so the PostgreSQL executor reuses the identical
/// reconstruction (ADR-0003 R3).
pub(crate) fn reconstruct(interned: &InternedBindings<'_>, raw: &RawRow<'_>) -> Result<Bindings> {
    let mut out = Bindings::new();
    for (var, def) in interned {
        if let Some(term) = build_term(def, raw)? {
            // `push`, not `insert`: `interned` comes from a `BTreeMap` (unique
            // keys), so `var` can never already be bound in `out`.
            out.push(var.clone(), term);
        }
    }
    Ok(out)
}
use std::sync::Arc;

use sf_core::datatype::{self, XsdTypeCode};
use sf_core::ir::{TermMap, TermType};
use sf_core::{Literal, Row, Term, Triple};

use crate::graph_map::RR_DEFAULT_GRAPH;
use crate::iq::{AggKind, Branch, ColRef, R2rmlGraphScope, TermDef};
use crate::{Error, Result};

#[cfg(test)]
mod graph_scope_tests {
    use super::*;
    use sf_core::ir::TermSpec;

    fn blank(graph: R2rmlGraphScope, graph_value: &str) -> Term {
        let schema = vec![ColRef::new(0, "id"), ColRef::new(0, "graph")];
        let index = build_col_index(&schema);
        let values = vec![Some("shared".to_owned()), Some(graph_value.to_owned())];
        let codes = vec![None, None];
        let raw = RawRow {
            values: &values,
            codes: &codes,
            index: &index,
        };
        build_term(
            &TermDef::R2rmlBlank {
                term_map: TermMap::Column("id".into(), TermSpec::blank_node()),
                alias: 0,
                graph,
            },
            &raw,
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn generated_labels_are_injective_over_effective_graph_and_identifier() {
        let default = blank(R2rmlGraphScope::Default, "unused");
        let named = blank(
            R2rmlGraphScope::Mapped {
                term_map: TermMap::Constant(Term::NamedNode(sf_core::NamedNode::new_unchecked(
                    "http://ex/g1",
                ))),
                alias: 0,
            },
            "unused",
        );
        let dynamic_named = blank(
            R2rmlGraphScope::Mapped {
                term_map: TermMap::Column("graph".into(), TermSpec::iri()),
                alias: 0,
            },
            "http://ex/g1",
        );
        let dynamic_default = blank(
            R2rmlGraphScope::Mapped {
                term_map: TermMap::Column("graph".into(), TermSpec::iri()),
                alias: 0,
            },
            RR_DEFAULT_GRAPH,
        );

        assert_eq!(named, dynamic_named);
        assert_eq!(default, dynamic_default);
        assert_ne!(default, named);
        assert!(matches!(default, Term::BlankNode(ref b) if b.as_str().starts_with("sfr1d_")));
        assert!(matches!(named, Term::BlankNode(ref b) if b.as_str().starts_with("sfr1n_")));
    }
}
