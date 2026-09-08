//! Preserve source-native comparison requirements across RDF-key deduplication.
use super::*;
use std::collections::BTreeSet;

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
