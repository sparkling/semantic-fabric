//! Preserve source-native comparison requirements across RDF-key deduplication.
use super::*;
use crate::iq::scan::LexicalMode;
use std::collections::BTreeSet;
type Modes = std::collections::BTreeMap<Box<str>, Option<BTreeSet<LexicalMode>>>;

/// Capture original consumer semantics before D1 replaces them with synthetic
/// raw Column recipes. IRI templates and explicit column literals preserve the
/// decoded lexical value. Literal conditions own identity/value roles separately;
/// natural literals and base-resolved IRIs cannot borrow this raw lexical proof.
pub(crate) fn lexical_keys(branch: &Branch, alias: usize) -> Vec<crate::iq::LexicalKey> {
    fn term(map: &TermMap, owner: usize, alias: usize, modes: &mut Modes) {
        if owner != alias {
            return;
        }
        let mode = match map {
            TermMap::Template(_, spec) => {
                (spec.term_type == sf_core::ir::TermType::Iri).then_some(LexicalMode::Decoded)
            }
            TermMap::Column(_, spec) if spec.term_type == sf_core::ir::TermType::Iri => {
                Some(LexicalMode::Iri {
                    base: spec.base.clone(),
                })
            }
            TermMap::Column(_, spec) if spec.term_type == sf_core::ir::TermType::BlankNode => {
                Some(LexicalMode::Decoded)
            }
            TermMap::Column(_, spec) => (spec.term_type == sf_core::ir::TermType::Literal
                && (spec.datatype.is_some() || spec.language.is_some()))
            .then_some(LexicalMode::Decoded),
            TermMap::Constant(_) => return,
        };
        let mut record = |column: &str| {
            modes
                .entry(column.into())
                .and_modify(|value| match (value.as_mut(), mode.as_ref()) {
                    (Some(modes), Some(mode)) => {
                        modes.insert(mode.clone());
                    }
                    _ => *value = None,
                })
                .or_insert_with(|| mode.clone().map(|mode| BTreeSet::from([mode])));
        };
        match map {
            TermMap::Column(column, _) => record(column),
            TermMap::Template(template, _) => {
                for segment in template.segments() {
                    if let sf_core::ir::Segment::Column(column) = segment {
                        record(column);
                    }
                }
            }
            TermMap::Constant(_) => {}
        }
    }
    let mut modes = std::collections::BTreeMap::new();
    for def in branch.bindings.values() {
        match def {
            TermDef::Const(_) => {}
            TermDef::Derived {
                term_map,
                alias: owner,
            } => term(term_map, *owner, alias, &mut modes),
            TermDef::R2rmlBlank {
                term_map,
                alias: owner,
                graph,
            } => {
                term(term_map, *owner, alias, &mut modes);
                if let R2rmlGraphScope::Mapped {
                    term_map,
                    alias: owner,
                } = graph
                {
                    term(term_map, *owner, alias, &mut modes);
                }
            }
            _ => {
                for column in def
                    .columns()
                    .into_iter()
                    .filter(|column| column.alias == alias)
                {
                    modes.insert(column.column, None);
                }
            }
        }
    }
    fn condition(cond: &SqlCond, alias: usize, modes: &mut Modes) {
        match cond {
            SqlCond::IriCmp(cmp) => {
                for operand in [&cmp.left, &cmp.right] {
                    if let crate::iq::iri_cmp::IriOperand::Column { column, base } = operand {
                        let mut spec = sf_core::ir::TermSpec::iri();
                        spec.base = base.clone();
                        term(
                            &TermMap::Column(column.column.clone(), spec),
                            column.alias,
                            alias,
                            modes,
                        );
                    }
                    if let crate::iq::iri_cmp::IriOperand::Template { .. } = operand {
                        for column in operand.columns().filter(|c| c.alias == alias) {
                            modes
                                .entry(column.column.clone())
                                .and_modify(|value| {
                                    if let Some(modes) = value {
                                        modes.insert(LexicalMode::Decoded);
                                    }
                                })
                                .or_insert_with(|| Some(BTreeSet::from([LexicalMode::Decoded])));
                        }
                    }
                }
            }
            SqlCond::LiteralCmp(cmp) => {
                for operand in [&cmp.left, &cmp.right] {
                    if let crate::iq::literal_cmp::LiteralOperand::Column { column, spec } = operand
                    {
                        term(
                            &TermMap::Column(column.column.clone(), spec.clone()),
                            column.alias,
                            alias,
                            modes,
                        );
                    }
                }
            }
            SqlCond::And(cs)
            | SqlCond::Or(cs)
            | SqlCond::Exists { conds: cs, .. }
            | SqlCond::NotExists { conds: cs, .. }
            | SqlCond::PathExists { conds: cs, .. } => {
                for c in cs {
                    condition(c, alias, modes);
                }
            }
            SqlCond::Not(c) => condition(c, alias, modes),
            _ => {}
        }
    }
    for cond in branch
        .where_conds
        .iter()
        .chain(branch.opts.iter().flat_map(|o| o.on.iter().chain(&o.extra)))
    {
        condition(cond, alias, &mut modes);
    }
    modes
        .into_iter()
        .flat_map(|(column, modes)| {
            modes
                .into_iter()
                .flatten()
                .map(move |mode| crate::iq::LexicalKey {
                    column: column.clone(),
                    mode,
                })
        })
        .collect()
}

pub(super) fn native_keys(branch: &Branch, alias: usize) -> Vec<(Box<str>, bool)> {
    fn visit(
        cond: &SqlCond,
        alias: usize,
        native: &mut BTreeSet<Box<str>>,
        rdf: &mut BTreeSet<Box<str>>,
    ) {
        match cond {
            SqlCond::NativeColEq(..) | SqlCond::NativeCmp(..) => {
                collect_cond_cols(cond, &mut |c| {
                    if c.alias == alias {
                        native.insert(c.column.clone());
                    }
                })
            }
            SqlCond::And(cs)
            | SqlCond::Or(cs)
            | SqlCond::Exists { conds: cs, .. }
            | SqlCond::NotExists { conds: cs, .. }
            | SqlCond::PathExists { conds: cs, .. } => {
                for c in cs {
                    visit(c, alias, native, rdf);
                }
            }
            SqlCond::Not(c) => visit(c, alias, native, rdf),
            // Existence guards do not make the value an RDF identity key.
            SqlCond::IsNull(_) | SqlCond::IsNotNull(_) => {}
            _ => collect_cond_cols(cond, &mut |c| {
                if c.alias == alias {
                    rdf.insert(c.column.clone());
                }
            }),
        }
    }
    let mut native = BTreeSet::new();
    let mut rdf = BTreeSet::new();
    for def in branch.bindings.values() {
        for c in def.columns() {
            if c.alias == alias {
                rdf.insert(c.column);
            }
        }
    }
    for cond in branch
        .where_conds
        .iter()
        .chain(branch.opts.iter().flat_map(|o| o.on.iter().chain(&o.extra)))
    {
        visit(cond, alias, &mut native, &mut rdf);
    }
    native
        .into_iter()
        .map(|c| {
            let both = rdf.contains(&c);
            (c, both)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_key_proof_keeps_identity_and_value_comparison_consumers_separate() {
        use sf_core::ir::{Template, TermSpec};
        let iri = TermDef::Derived {
            term_map: TermMap::Template(
                Template::parse("http://ex/{key}").unwrap(),
                TermSpec::iri(),
            ),
            alias: 3,
        };
        let natural = TermDef::Derived {
            term_map: TermMap::Column("key".into(), TermSpec::plain_literal()),
            alias: 3,
        };
        let typed = TermDef::Derived {
            term_map: TermMap::Column(
                "key".into(),
                TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(
                    "http://www.w3.org/2001/XMLSchema#integer",
                )),
            ),
            alias: 3,
        };
        let mut branch = Branch::empty();
        branch.bindings.insert("iri".into(), iri.clone());
        let keys = lexical_keys(&branch, 3);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].column.as_ref(), "key");
        assert!(lexical_keys(&branch, 4).is_empty());
        branch.bindings.insert("typed".into(), typed);
        assert_eq!(
            lexical_keys(&branch, 3).len(),
            1,
            "explicit literal identity is now condition-owned"
        );
        branch.bindings.remove("iri");
        assert_eq!(lexical_keys(&branch, 3).len(), 1);
        branch.bindings.insert("natural".into(), natural);
        assert!(
            lexical_keys(&branch, 3).is_empty(),
            "one natural consumer revokes raw lexical identity"
        );
        branch.bindings.clear();
        branch.bindings.insert("iri".into(), iri);
        branch.bindings.insert(
            "column-iri".into(),
            TermDef::Derived {
                term_map: TermMap::Column("key".into(), TermSpec::iri().with_base("http://ex/")),
                alias: 3,
            },
        );
        assert_eq!(
            lexical_keys(&branch, 3).len(),
            2,
            "base resolution and template decoding retain separate identity keys"
        );
    }

    #[test]
    fn native_markers_survive_nested_alias_rewrites() {
        let mut cond = SqlCond::And(vec![
            SqlCond::NativeColEq(ColRef::new(1, "a"), ColRef::new(2, "b")),
            SqlCond::Not(Box::new(SqlCond::NativeCmp(
                ColRef::new(1, "a"),
                CmpOp::Eq,
                "bound".into(),
            ))),
        ]);
        rewrite_cond_alias(&mut cond, &|c| {
            if c.alias == 1 {
                c.alias = 7;
            }
        });
        let SqlCond::And(parts) = cond else {
            unreachable!()
        };
        assert!(matches!(&parts[0], SqlCond::NativeColEq(a,b) if a.alias == 7 && b.alias == 2));
        assert!(
            matches!(&parts[1], SqlCond::Not(inner) if matches!(inner.as_ref(), SqlCond::NativeCmp(a,_,v) if a.alias == 7 && v == "bound"))
        );
    }

    #[test]
    fn same_terms_elimination_never_orphans_native_join_aliases() {
        let mut b = Branch::empty();
        b.core = (0..3)
            .map(|alias| Scan {
                alias,
                source: LogicalSource::Table(if alias == 2 { "other" } else { "items" }.into())
                    .into(),
            })
            .collect();
        b.bindings.insert(
            "s".into(),
            TermDef::Derived {
                term_map: TermMap::Column("id".into(), sf_core::ir::TermSpec::plain_literal()),
                alias: 0,
            },
        );
        b.where_conds = vec![
            SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id")),
            SqlCond::NativeColEq(ColRef::new(1, "id"), ColRef::new(2, "id")),
        ];
        sameterm::same_terms_elimination(
            &mut b,
            &CascadeCtx {
                project: None,
                distinct: true,
            },
        );
        for c in &b.where_conds {
            collect_cond_cols(c, &mut |r| {
                assert!(
                    b.core.iter().any(|s| s.alias == r.alias),
                    "dangling alias {}",
                    r.alias
                )
            });
        }
        assert!(b
            .where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::NativeColEq(..))));
    }
}
