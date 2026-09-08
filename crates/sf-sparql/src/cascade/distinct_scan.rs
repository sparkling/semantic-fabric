//! Preserve source-native comparison requirements across RDF-key deduplication.
use super::*;
use std::collections::BTreeSet;

/// Capture original consumer semantics before D1 replaces them with synthetic
/// raw Column recipes. Every consumer must be an IRI template: literal identity
/// versus value comparison needs separate authority, even for explicit datatypes.
pub(super) fn lexical_keys(branch: &Branch, alias: usize) -> Vec<crate::iq::LexicalKey> {
    fn term(
        map: &TermMap,
        owner: usize,
        alias: usize,
        modes: &mut std::collections::BTreeMap<Box<str>, bool>,
    ) {
        if owner != alias {
            return;
        }
        let lexical = match map {
            TermMap::Template(_, spec) => spec.term_type == sf_core::ir::TermType::Iri,
            TermMap::Column(_, _) => false,
            TermMap::Constant(_) => return,
        };
        let mut record = |column: &str| {
            modes
                .entry(column.into())
                .and_modify(|value| *value &= lexical)
                .or_insert(lexical);
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
                    modes.insert(column.column, false);
                }
            }
        }
    }
    modes
        .into_iter()
        .filter_map(|(column, lexical)| lexical.then_some(crate::iq::LexicalKey { column }))
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
        assert!(
            lexical_keys(&branch, 3).is_empty(),
            "mixed IRI/literal consumers need per-comparison roles"
        );
        branch.bindings.remove("iri");
        assert!(
            lexical_keys(&branch, 3).is_empty(),
            "literal identity remains separate"
        );
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
        assert!(
            lexical_keys(&branch, 3).is_empty(),
            "base resolution is not raw lexical identity"
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
