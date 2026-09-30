//! Term and graph-scope unification.
use super::*;

pub(super) fn unify_graph_scope(a: &R2rmlGraphScope, b: &R2rmlGraphScope) -> Unify {
    match (a, b) {
        (R2rmlGraphScope::Default, R2rmlGraphScope::Default) => Unify::Sat(Vec::new()),
        (
            R2rmlGraphScope::Mapped {
                term_map: ta,
                alias: aa,
            },
            R2rmlGraphScope::Mapped {
                term_map: tb,
                alias: ab,
            },
        ) => unify(&plain_term_def(ta, *aa), &plain_term_def(tb, *ab)),
        (R2rmlGraphScope::Default, R2rmlGraphScope::Mapped { term_map, alias })
        | (R2rmlGraphScope::Mapped { term_map, alias }, R2rmlGraphScope::Default) => unify(
            &TermDef::Const(Term::NamedNode(sf_core::NamedNode::new_unchecked(
                crate::graph_map::RR_DEFAULT_GRAPH,
            ))),
            &plain_term_def(term_map, *alias),
        ),
    }
}

pub(super) fn combine_unify(left: Unify, right: Unify) -> Unify {
    match (left, right) {
        (Unify::Empty, _) | (_, Unify::Empty) => Unify::Empty,
        (Unify::Sat(mut a), Unify::Sat(b)) => {
            a.extend(b);
            Unify::Sat(a)
        }
        (Unify::Unsupported(reason), _) | (_, Unify::Unsupported(reason)) => {
            Unify::Unsupported(reason)
        }
    }
}

/// Unify a constant against a derived term, retaining decoder identity where
/// raw-column equality cannot prove equality of the generated RDF terms.
pub(super) fn unify_const_derived(c: &Term, tm: &TermMap, alias: usize) -> Unify {
    if let Some(identity) = iri_cmp::static_constant(c, tm, alias) {
        return identity;
    }
    if let (Term::NamedNode(value), Some(column)) = (
        c,
        crate::iq::iri_cmp::needs_resolution(tm)
            .then(|| iri_cmp::operand(tm, alias))
            .flatten(),
    ) {
        return Unify::Sat(vec![iri_cmp::identity(
            column,
            crate::iq::iri_cmp::IriOperand::Constant(value.clone()),
        )]);
    }
    if let (Term::Literal(value), Some(column)) = (c, literal_cmp::operand(tm, alias)) {
        return Unify::Sat(vec![literal_cmp::identity(
            column,
            LiteralOperand::Constant(value.clone()),
        )]);
    }
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
                let prefix = leading_literal_prefix(t.segments());
                if !want.starts_with(&prefix) {
                    Unify::Empty
                } else {
                    Unify::Unsupported("constant vs multi-slot template".to_owned())
                }
            }
        },
    }
}

/// Unify two column/template term maps → raw-column equalities, or a disjointness
/// proof, or unsupported.
pub(super) fn unify_derived(t1: &TermMap, a1: usize, t2: &TermMap, a2: usize) -> Unify {
    if crate::iq::iri_cmp::needs_resolution(t1) || crate::iq::iri_cmp::needs_resolution(t2) {
        if let (Some(left), Some(right)) = (iri_cmp::operand(t1, a1), iri_cmp::operand(t2, a2)) {
            return Unify::Sat(vec![iri_cmp::identity(left, right)]);
        }
    }
    if let (Some(left), Some(right)) = (literal_cmp::operand(t1, a1), literal_cmp::operand(t2, a2))
    {
        // A natural column's datatype is learned from its live decoder, not
        // from an absent rr:datatype. Keep the complete identity until emission.
        return Unify::Sat(vec![literal_cmp::identity(left, right)]);
    }
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
        (TermMap::Template(x, spec1), TermMap::Template(y, spec2)) => {
            align_templates(x, spec1, a1, y, spec2, a2)
        }
        _ => Unify::Unsupported("column vs template unification".to_owned()),
    }
}
pub(super) fn term_map_type(tm: &TermMap) -> Option<TermType> {
    crate::iq::term_map_type(tm)
}

/// The effective `(datatype-IRI, language)` of a *literal* term map — a plain
/// literal normalises to `xsd:string`, a lang-tagged literal to `rdf:langString` —
/// i.e. the key under which two literals are `sameTerm` (SPARQL §17.4.1.7). `None`
/// for a non-literal term map (IRI / blank node) or a wrapped constant.
pub(super) fn literal_key(tm: &TermMap) -> Option<(&str, Option<&str>)> {
    let spec = match tm {
        TermMap::Column(_, s) | TermMap::Template(_, s) => s,
        TermMap::Constant(_) => return None,
    };
    literal_key_of_spec(spec)
}

/// [`literal_key`]'s spec-level core, reused directly by [`align_templates`]
/// (Run 5 W6 fix) — that caller already holds each side's `&TermSpec`, with
/// no `TermMap` to unwrap.
pub(super) fn literal_key_of_spec(spec: &TermSpec) -> Option<(&str, Option<&str>)> {
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
pub(super) fn const_lexical(c: &Term, want: Option<TermType>) -> Result<String, ()> {
    match (c, want) {
        (Term::NamedNode(n), Some(TermType::Iri) | None) => Ok(n.as_str().to_owned()),
        (Term::BlankNode(b), Some(TermType::BlankNode) | None) => Ok(b.as_str().to_owned()),
        (Term::Literal(l), Some(TermType::Literal) | None) => Ok(l.value().to_owned()),
        _ => Err(()),
    }
}
