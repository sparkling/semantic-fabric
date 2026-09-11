//! Preserve source-native comparison requirements across RDF-key deduplication.
use super::*;
use crate::iq::scan::LexicalMode;
use std::collections::BTreeSet;
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Consumer {
    Lexical(LexicalMode),
    Natural,
}
type Modes = std::collections::BTreeMap<Box<str>, Option<BTreeSet<Consumer>>>;

#[path = "distinct_scan_work.rs"]
mod work;
pub(super) use work::condition_columns;
pub(super) use work::native_keys_with_work;

/// Capture original consumer semantics before D1 replaces them with synthetic
/// raw Column recipes. IRI templates and explicit column literals preserve the
/// decoded lexical value. Literal conditions own identity/value roles separately;
/// natural literals and base-resolved IRIs cannot borrow this raw lexical proof.
pub(crate) fn lexical_keys(branch: &Branch, alias: usize) -> Vec<crate::iq::LexicalKey> {
    keys(branch, alias, true)
}

pub(crate) fn binding_lexical_keys(branch: &Branch, alias: usize) -> Vec<crate::iq::LexicalKey> {
    keys(branch, alias, false)
}

fn keys(branch: &Branch, alias: usize, with_conditions: bool) -> Vec<crate::iq::LexicalKey> {
    keys_with_work(
        branch,
        alias,
        with_conditions,
        crate::build::control::BuildWork::new(crate::CompilerWorkMode::Uncontrolled),
    )
    .expect("uncontrolled lexical-key inventory is infallible")
}

pub(super) fn keys_with_work(
    branch: &Branch,
    alias: usize,
    with_conditions: bool,
    work: crate::build::control::BuildWork<'_>,
) -> crate::Result<Vec<crate::iq::LexicalKey>> {
    work.checkpoint()?;
    let mut modes = Modes::new();
    for definition in branch.bindings.values() {
        work.charge(1)?;
        match definition {
            TermDef::Const(_) => (),
            TermDef::Derived {
                term_map,
                alias: owner,
            } => work::term(term_map, *owner, alias, &mut modes, work)?,
            TermDef::R2rmlBlank {
                term_map,
                alias: owner,
                graph,
            } => {
                work::term(term_map, *owner, alias, &mut modes, work)?;
                if let R2rmlGraphScope::Mapped {
                    term_map,
                    alias: owner,
                } = graph
                {
                    work::term(term_map, *owner, alias, &mut modes, work)?;
                }
            }
            _ => {
                for (owner, name) in super::resolve_work::columns(definition, work)? {
                    work.charge(1)?;
                    if owner == alias {
                        work::record(name, None, &mut modes, work)?;
                    }
                }
            }
        }
    }
    fn condition(
        cond: &SqlCond,
        alias: usize,
        modes: &mut Modes,
        work: crate::build::control::BuildWork<'_>,
    ) -> crate::Result<()> {
        let work = work.enter()?;
        match cond {
            SqlCond::IriCmp(cmp) => {
                for operand in [&cmp.left, &cmp.right] {
                    work.charge(1)?;
                    match operand {
                        crate::iq::iri_cmp::IriOperand::Column { column, base } => {
                            if column.alias == alias {
                                let mode = Consumer::Lexical(LexicalMode::Iri {
                                    base: base
                                        .as_deref()
                                        .map(|value| work.variable(value))
                                        .transpose()?,
                                });
                                work::record(&column.column, Some(&mode), modes, work)?;
                            }
                        }
                        crate::iq::iri_cmp::IriOperand::Template { parts, .. } => {
                            work.charge(parts.len())?;
                            for column in operand.columns() {
                                work.charge(1)?;
                                if column.alias == alias {
                                    work::record(
                                        &column.column,
                                        Some(&Consumer::Lexical(LexicalMode::Decoded)),
                                        modes,
                                        work,
                                    )?;
                                }
                            }
                        }
                        crate::iq::iri_cmp::IriOperand::Constant(_) => (),
                    }
                }
            }
            SqlCond::LiteralCmp(cmp) => {
                for operand in [&cmp.left, &cmp.right] {
                    work.charge(1)?;
                    if let crate::iq::literal_cmp::LiteralOperand::Column { column, spec } = operand
                    {
                        if column.alias == alias {
                            let mode = work::column_mode(spec, work)?;
                            work::record(&column.column, mode.as_ref(), modes, work)?;
                        }
                    }
                }
            }
            SqlCond::And(conds)
            | SqlCond::Or(conds)
            | SqlCond::Exists { conds, .. }
            | SqlCond::NotExists { conds, .. }
            | SqlCond::PathExists { conds, .. } => {
                for cond in conds {
                    condition(cond, alias, modes, work)?;
                }
            }
            SqlCond::Not(cond) => condition(cond, alias, modes, work)?,
            _ => (),
        }
        Ok(())
    }
    if with_conditions {
        for cond in branch.where_conds.iter().chain(
            branch
                .opts
                .iter()
                .flat_map(|opt| opt.on.iter().chain(&opt.extra)),
        ) {
            condition(cond, alias, &mut modes, work)?;
        }
    }
    work::finish(modes, work)
}

#[cfg(test)]
pub(super) fn native_keys(branch: &Branch, alias: usize) -> Vec<(Box<str>, bool)> {
    native_keys_with_work(
        branch,
        alias,
        crate::build::control::BuildWork::new(crate::CompilerWorkMode::Uncontrolled),
    )
    .expect("uncontrolled native-key inventory is infallible")
}

#[cfg(test)]
fn native_keys_raw(branch: &Branch, alias: usize) -> Vec<(Box<str>, bool)> {
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
            SqlCond::IsNull(_) | SqlCond::IsNotNull(_) | SqlCond::DecodedIsNotNull(_) => {}
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
            2,
            "authored datatype proof must survive independently of raw IRI consumption"
        );
        branch.bindings.remove("iri");
        assert_eq!(lexical_keys(&branch, 3).len(), 1);
        branch.bindings.insert("natural".into(), natural);
        let mixed = lexical_keys(&branch, 3);
        assert_eq!(mixed.len(), 2);
        assert!(mixed.iter().any(|key| key.mode == LexicalMode::Natural));
        assert!(mixed
            .iter()
            .any(|key| matches!(key.mode, LexicalMode::TypedLiteral { .. })));
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
