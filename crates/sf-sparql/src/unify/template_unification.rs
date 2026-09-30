//! Template identity and disjointness proofs.
use super::*;

/// Two templates unify iff their segment kind-sequence matches (same fixed text
/// at each literal position, a column slot at each column position) AND their
/// declared term kind/literal-normalisation agrees (Run 5 W6 fix, below): a
/// same-SHAPE IRI template can never unify with a same-shape Literal or
/// BlankNode template, and two same-shape Literal templates additionally need
/// the same language/normalised-datatype — a shape match alone is NOT a term-
/// equality proof. Aligned columns become pairwise raw-column equalities; a
/// fixed-text mismatch proves disjointness; a kind/length mismatch falls to
/// [`template_eq_or_unsupported`] (Run 4 Wave B3: a `SqlCond::TemplateEq`
/// fallback for the term classes where that is sound, else the pre-existing
/// conservative Unsupported — never an unsound prune) UNLESS the ADR-0032 D6
/// leading-literal-prefix check below already proved disjointness.
pub(super) fn align_templates(
    x: &sf_core::ir::Template,
    spec1: &TermSpec,
    a1: usize,
    y: &sf_core::ir::Template,
    spec2: &TermSpec,
    a2: usize,
) -> Unify {
    if spec1.term_type == TermType::Iri
        && spec2.term_type == TermType::Iri
        && (spec1.base.is_some() || spec2.base.is_some())
    {
        return Unify::Sat(vec![iri_cmp::identity(
            iri_cmp::operand(&TermMap::Template(x.clone(), spec1.clone()), a1).unwrap(),
            iri_cmp::operand(&TermMap::Template(y.clone(), spec2.clone()), a2).unwrap(),
        )]);
    }
    let (sx, sy) = (x.segments(), y.segments());
    // ADR-0032 D6 lift: a conflict in the two templates' LEADING LITERAL text
    // (the fixed characters before either's first column reference) proves
    // disjointness REGARDLESS of overall shape/length — a column's value
    // never contains the raw delimiter/prefix characters the template's OWN
    // literal segments do (the same percent-encoding injectivity argument
    // `TemplateShape`'s prefix/suffix splitting, above, already relies on),
    // so two templates whose prefixes conflict before their first column can
    // never expand to the same value. Checked BEFORE the length/shape
    // conservatism below — that is precisely the case this lift targets: two
    // DIFFERENT quoted shapes' description maps (distinct subject templates,
    // typically distinct lengths) sharing the SAME 4-predicate ADR-0032 D1
    // vocabulary (`differential_star.rs`'s object-side-nesting-depth-2 test).
    // NEVER prunes without that proof — a length/shape MATCH with no prefix
    // conflict still falls through to the existing, unchanged logic below.
    if leading_literal_prefixes_conflict(sx, sy) {
        return Unify::Empty;
    }
    if sx.len() != sy.len() {
        return template_eq_or_unsupported(
            sx,
            spec1,
            a1,
            sy,
            spec2,
            a2,
            "template length mismatch",
        );
    }
    // Run 5 W6 fix: a matching segment SHAPE only ever proves the two
    // templates render the SAME TEXT for equal column values — it says
    // nothing about whether equal text means equal RDF TERMS. An IRI, a
    // Literal, and a BlankNode template can render identical text and still
    // be provably UNEQUAL terms (an IRI is never `sameTerm` as a Literal,
    // etc.), and two Literal templates need matching language/datatype too
    // (`"x"@en` != `"x"@es`, `"5"` != `"5"^^xsd:integer`). `unify_derived`
    // (this function's join-key caller) already proves exactly this BEFORE
    // ever calling `align_templates`, via its own `term_map_type`/
    // `literal_key` guards above — but `var_var_eq_beyond_column` (the
    // FILTER `?a = ?b` path) calls `align_templates` directly, bypassing
    // those guards entirely. So the same classification is mirrored here,
    // reusing `literal_key`'s own spec-level core (`literal_key_of_spec`) —
    // never a third normalisation.
    if spec1.term_type != spec2.term_type {
        return Unify::Empty;
    }
    if literal_key_of_spec(spec1) != literal_key_of_spec(spec2) {
        return Unify::Empty;
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
            _ => {
                return template_eq_or_unsupported(
                    sx,
                    spec1,
                    a1,
                    sy,
                    spec2,
                    a2,
                    "template shape mismatch",
                )
            }
        }
    }
    Unify::Sat(eqs)
}

/// Run 4 Wave B3 — the fallback [`align_templates`] falls to on a genuine
/// shape mismatch (`why` names which: different segment count, or same count
/// but a literal/column kind mismatch at some position): `Sat` with ONE
/// [`SqlCond::TemplateEq`] rendering EACH template as a SQL string
/// concatenation and comparing them with `=`, but ONLY when BOTH sides are a
/// term class where that rendered-string equality IS RDF term equality (see
/// [`lexical_eq_is_term_eq`]) AND the two sides agree on which class (an IRI
/// can never equal a literal regardless of lexical form, so a class mismatch
/// stays the ordinary Unsupported, not a silently-always-false condition —
/// `unify_derived`'s `term_map_type` check already rules this out for the
/// join-key caller, but `var_var_eq_beyond_column` calls this directly
/// without that pre-check, so it is re-verified here). The original
/// Unsupported(`why`) otherwise: a typed-literal / language-tagged / blank-
/// node class, where the RENDERED STRING alone does not determine term
/// equality (see `lexical_eq_is_term_eq`'s doc comment) — a sound-over-
/// complete 501, never a wrong answer.
pub(super) fn template_eq_or_unsupported(
    sx: &[Segment],
    spec1: &TermSpec,
    a1: usize,
    sy: &[Segment],
    spec2: &TermSpec,
    a2: usize,
    why: &str,
) -> Unify {
    if spec1.term_type == spec2.term_type
        && lexical_eq_is_term_eq(spec1)
        && lexical_eq_is_term_eq(spec2)
    {
        // Run 4 B-repair FIX 2: `term_type` is checked equal just above, so
        // `spec1`/`spec2` are either BOTH `Iri` or BOTH `Literal` (the only two
        // classes `lexical_eq_is_term_eq` admits) — one flag correctly covers
        // both sides. Only an IRI template percent-encodes at expansion.
        let encode_iri = spec1.term_type == TermType::Iri;
        Unify::Sat(vec![SqlCond::TemplateEq(
            sx.to_vec(),
            a1,
            sy.to_vec(),
            a2,
            encode_iri,
        )])
    } else {
        Unify::Unsupported(why.to_owned())
    }
}

/// Whether a template-bound term map's [`TermSpec`] is a term class where SQL
/// string/lexical equality on the FULLY RENDERED template text is EXACTLY RDF
/// term equality — the restriction [`template_eq_or_unsupported`]'s
/// `SqlCond::TemplateEq` fallback needs (Run 4 Wave B3):
///
/// * **IRIs** — RDF term equality for two `NamedNode`s IS lexical equality of
///   their IRI string; no other axis exists.
/// * **Plain literals** — no language tag, and no datatype (or the
///   `xsd:string` datatype, its normalised-equal form — mirrors
///   [`literal_key`]'s own "no datatype ⇒ xsd:string" default): term equality
///   is again exactly lexical equality (SPARQL §17.4.1.7 simple/`xsd:string`
///   literal equality IS lexical-form equality — no value space beyond the
///   string itself).
///
/// Excluded (stays the pre-existing Unsupported): every OTHER typed literal —
/// a datatype's VALUE space can equate two DIFFERENT lexical forms
/// (`"01"^^xsd:integer` =_value= `"1"^^xsd:integer` but they are different
/// STRINGS, so a SQL string `=` would wrongly refuse a pair the SPARQL oracle
/// accepts) — a language-tagged literal (equality ALSO requires the tag to
/// match, a second axis this string-only comparison does not check), and
/// blank nodes (not attempted this wave; RDF term equality for a blank node
/// is scoped to one query solution, a narrower question this lexical
/// shortcut does not address).
pub(super) fn lexical_eq_is_term_eq(spec: &TermSpec) -> bool {
    match spec.term_type {
        TermType::Iri => true,
        TermType::Literal => {
            spec.language.is_none()
                && spec
                    .datatype
                    .as_ref()
                    .is_none_or(|dt| dt.as_str() == "http://www.w3.org/2001/XMLSchema#string")
        }
        TermType::BlankNode => false,
    }
}

/// ADR-0032 D6 flat/tree 501-parity: whether two term definitions for the SAME
/// (join-correlated) variable are provably disjoint via the leading-literal-prefix
/// check ALONE — the narrow proof `unfold::merge` reuses to prune a path-carrying
/// join as empty BEFORE its unconditional "no join onto any path branch" 501 would
/// otherwise fire (the tree path has no such preemptive check, so it reaches
/// [`align_templates`] during ordinary unification and proves the SAME join empty).
/// Only the template/template, conflicting-prefix case answers `true`; every other
/// shape (a `Const`, a `Column`, a `Coalesce`/`Concat`/`Agg`/`ComposedTriple`, or two
/// templates with no prefix conflict) answers `false` — deliberately NOT the full
/// [`unify`], whose `Sat`/`Unsupported` verdicts make no sense to act on before the
/// path restriction they would otherwise bypass.
pub(crate) fn templates_provably_disjoint(a: &TermDef, b: &TermDef) -> bool {
    let (Some(tx), Some(ty)) = (term_def_template(a), term_def_template(b)) else {
        return false;
    };
    leading_literal_prefixes_conflict(tx.segments(), ty.segments())
}

pub(super) fn term_def_template(def: &TermDef) -> Option<&sf_core::ir::Template> {
    match def {
        TermDef::Derived {
            term_map: TermMap::Template(template, spec),
            ..
        }
        | TermDef::R2rmlBlank {
            term_map: TermMap::Template(template, spec),
            ..
        } if spec.base.is_none() => Some(template),
        _ => None,
    }
}

/// Whether `sx`/`sy`'s leading literal text — see [`leading_literal_prefix`]
/// — differs at some position BOTH have fixed text for (a shorter prefix
/// simply has nothing to conflict with beyond its own length, which is fine:
/// e.g. `"http://ex/a/"` vs `"http://ex/ab"` DO conflict at the 12th
/// character, but `"http://ex/a"` (ending exactly there) vs `"http://ex/a/"`
/// do NOT — the shorter one could still be a strict prefix of a longer
/// literal run the longer template's OWN later segments complete).
pub(super) fn leading_literal_prefixes_conflict(sx: &[Segment], sy: &[Segment]) -> bool {
    let px = leading_literal_prefix(sx);
    let py = leading_literal_prefix(sy);
    px.bytes().zip(py.bytes()).any(|(a, b)| a != b)
}

/// The concatenated text of every `Segment::Literal` from the start of
/// `segs` up to (not including) the first `Segment::Column` — the portion of
/// a template whose exact characters are fixed by the template itself,
/// independent of any row's column values.
pub(super) fn leading_literal_prefix(segs: &[Segment]) -> String {
    let mut s = String::new();
    for seg in segs {
        match seg {
            Segment::Literal(l) => s.push_str(l),
            Segment::Column(_) => break,
        }
    }
    s
}

pub(super) enum TemplateShape {
    AllLiteral(String),
    SingleSlot {
        prefix: String,
        column: Box<str>,
        suffix: String,
    },
    MultiSlot,
}

pub(super) fn split_template(t: &sf_core::ir::Template) -> TemplateShape {
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
