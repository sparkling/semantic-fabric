use super::*;
use crate::iq::{scan::LexicalMode, LexicalKey, Scan, ScanSource};
use sf_core::ir::TermSpec;

#[test]
fn iri_dedup_never_falls_through_to_raw_distinct_for_an_unproven_sibling_key() {
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            ["u", "v"]
                .map(|name| sf_sql::backend::ResultColumn {
                    name: name.into(),
                    text_key: None,
                    sqlite_decode: Some(SqliteDecode {
                        declared: None,
                        padding: None,
                    }),
                })
                .to_vec(),
        )
        .unwrap();
    let scan = Scan {
        alias: 0,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 0,
                source: source.into(),
            }),
            columns: ["u", "v"]
                .map(|name| {
                    (
                        name.into(),
                        TermMap::Column(name.into(), TermSpec::plain_literal()),
                    )
                })
                .to_vec(),
            guards: vec![],
            distinct: true,
            native_keys: vec![],
            lexical_keys: vec![LexicalKey {
                column: "u".into(),
                mode: LexicalMode::Iri {
                    base: Some("http://ex/".into()),
                },
            }],
        },
    };
    let error = super::super::scan::scan_ref(&scan, Dialect::Sqlite, &catalog, &mut vec![], &mut 0)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exact keys for every scan consumer"),
        "{error}"
    );
}

#[test]
fn missing_or_native_decoder_never_authorizes_raw_iri_equality() {
    let comparison = IriComparison {
        left: IriOperand::Column {
            column: ColRef::new(0, "u"),
            base: Some("http://ex/".into()),
        },
        right: IriOperand::Constant(sf_core::NamedNode::new_unchecked("http://ex/AB")),
    };
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        assert!(matches!(
            render(
                &comparison,
                dialect,
                &ColumnCatalog::default(),
                &ActualColumns::default(),
                &mut vec![],
                &mut 0
            ),
            Err(Error::Unsupported(_))
        ));
    }
}
