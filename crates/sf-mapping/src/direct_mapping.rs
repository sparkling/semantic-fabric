//! Direct Mapping (W3C *A Direct Mapping of Relational Data to RDF*) realised as
//! **auto-generated R2RML** (ADR-0003: "R2RML (+ Direct Mapping as auto-generated
//! R2RML)").
//!
//! The Direct Mapping of a relational schema is, by construction, an R2RML
//! mapping: per table one `rr:TriplesMap` whose subject is an `rr:template` over
//! the primary key (a blank node when the table has none), one predicate-object
//! map per column, one `rdf:type` to the table class, and one referencing object
//! map ([`crate::r2rml`] `RefObjectMap`) per foreign key (W3C Direct Mapping §2).
//! So Direct Mapping produces the *same* [`sf_core::ir::TriplesMap`] shape that
//! hand-written R2RML produces, and the virtualiser keeps a single rewrite target
//! (ADR-0003 R1) — there is no second mapping model.
//!
//! Schema introspection (the input) lives in `sf-sql` (the source/SQL layer,
//! ADR-0006); the caller (the conformance harness / executor) introspects and
//! passes [`sf_sql::TableSchema`] in — this module does no I/O. Column SQL types
//! drive the R2RML §10 natural datatype mapping, which is applied downstream in
//! term generation (`sf_core::datatype`, centralised per ADR-0015 / ADR-0003 R3),
//! so the column object maps here carry no explicit `rr:datatype`.

use sf_core::ir::{
    Join, LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, Segment, SubjectMap,
    Template, TermMap, TermSpec, TriplesMap,
};
use sf_core::{NamedNode, Result, Term};
use sf_sql::TableSchema;

/// Generate the Direct-Mapping IR for a relational schema (W3C Direct Mapping §2),
/// as auto-generated R2RML. `base_iri` is the document base (the test suite fixes
/// it at `http://example.com/base/`, ADR-0005). The produced [`TriplesMap`]s are
/// identical in shape to a parsed R2RML mapping (ADR-0003), so the virtualiser is
/// unaffected by which path produced them.
pub fn direct_mapping(tables: &[TableSchema], base_iri: &str) -> Result<Vec<TriplesMap>> {
    tables.iter().map(|t| table_map(t, base_iri)).collect()
}

fn table_map(table: &TableSchema, base: &str) -> Result<TriplesMap> {
    let class_iri = format!("{base}{}", encode(&table.name));
    let subject = subject_map(table, base, &class_iri)?;
    let mut poms = Vec::new();

    // One literal predicate-object map per column (W3C DM §2: the column property
    // is `<base/Table#Column>`; the object is the natural-typed value, with §10
    // applied downstream in term-gen).
    for col in &table.columns {
        let predicate = constant_iri(&format!(
            "{base}{}#{}",
            encode(&table.name),
            encode(&col.name)
        ));
        poms.push(PredicateObjectMap {
            predicates: vec![predicate],
            objects: vec![ObjectMap::Term(TermMap::Column(
                col.name.as_str().into(),
                TermSpec::plain_literal(),
            ))],
            graphs: vec![],
        });
    }

    // One referencing object map per foreign key: predicate `<…#ref-Col[;Col…]>`,
    // object the referenced row's subject via an equi-join (W3C DM §2).
    for fk in &table.foreign_keys {
        // Encode each FK column name, then join with a literal `;` separator (the
        // separator is structural and is not itself percent-encoded — W3C DM §2).
        let ref_name = fk
            .columns
            .iter()
            .map(|c| encode(c))
            .collect::<Vec<_>>()
            .join(";");
        let predicate = constant_iri(&format!("{base}{}#ref-{}", encode(&table.name), ref_name));
        let joins = fk
            .columns
            .iter()
            .zip(&fk.parent_columns)
            .map(|(c, p)| Join {
                child: c.clone(),
                parent: p.clone(),
            })
            .collect();
        poms.push(PredicateObjectMap {
            predicates: vec![predicate],
            objects: vec![ObjectMap::Ref(RefObjectMap {
                parent_triples_map: format!("{base}{}", encode(&fk.parent_table)),
                joins,
            })],
            graphs: vec![],
        });
    }

    Ok(TriplesMap {
        id: class_iri,
        source: LogicalSource::Table(table.name.clone()),
        subject,
        predicate_object_maps: poms,
    })
}

/// The subject map: a primary-key template IRI (`<base/Table/Col=val[;Col=val]>`),
/// or a per-row blank node when the table has no primary key (W3C DM §2). The
/// table class is attached via `rr:class`, yielding the `rdf:type` triple.
fn subject_map(table: &TableSchema, base: &str, class_iri: &str) -> Result<SubjectMap> {
    let class = NamedNode::new(class_iri)
        .map_err(|e| sf_core::Error::Mapping(format!("invalid DM class IRI {class_iri:?}: {e}")))?;
    let term = if table.primary_key.is_empty() {
        // No PK ⇒ a fresh blank node per row, keyed on all columns so each row
        // gets a distinct node (W3C DM §2: "a fresh blank node unique to the row").
        blank_node_template(table)
    } else {
        pk_template(table, base)?
    };
    Ok(SubjectMap {
        term,
        classes: vec![class],
        graphs: vec![],
    })
}

/// `<base/Table/Pk1={Pk1};Pk2={Pk2}>` as an IRI template (values percent-encoded
/// by the IRI term-gen path; the fixed name parts are encoded here).
fn pk_template(table: &TableSchema, base: &str) -> Result<TermMap> {
    let mut segs = Vec::new();
    segs.push(Segment::Literal(
        format!("{base}{}/", encode(&table.name)).into(),
    ));
    for (i, pk) in table.primary_key.iter().enumerate() {
        let prefix = if i == 0 {
            format!("{}=", encode(pk))
        } else {
            format!(";{}=", encode(pk))
        };
        segs.push(Segment::Literal(prefix.into()));
        segs.push(Segment::Column(pk.as_str().into()));
    }
    Ok(TermMap::Template(
        Template::from_segments(segs)?,
        TermSpec::iri(),
    ))
}

/// A per-row blank-node template (W3C DM §2: "a fresh blank node unique to the
/// row"). A no-PK table has no stable key over its columns — two identical rows
/// must still get *distinct* blank nodes — so the node is keyed on the source's
/// physical row identifier (SQLite's `rowid` pseudo-column; SQLite is this wave's
/// execution target, ADR-0005), table-qualified so rows of different no-PK tables
/// never share a label. The label is existential (graph-isomorphism ignores its
/// spelling), so the only requirements are per-row uniqueness and per-row
/// stability, which `{table}_{rowid}` satisfies.
fn blank_node_template(table: &TableSchema) -> TermMap {
    let segs = vec![
        Segment::Literal(format!("{}_", encode(&table.name)).into()),
        Segment::Column(ROWID.into()),
    ];
    // from_segments only fails on an empty list; this list is non-empty.
    TermMap::Template(
        Template::from_segments(segs).expect("non-empty segment list"),
        TermSpec::blank_node(),
    )
}

/// SQLite's per-row physical identifier pseudo-column (R2RML §5 sources are base
/// tables; the executor projects `t<alias>."rowid"`). Keyed on for no-PK rows.
const ROWID: &str = "rowid";

fn constant_iri(iri: &str) -> TermMap {
    TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked(iri)))
}

/// Percent-encode a table/column name for the fixed (non-value) part of a DM IRI.
/// W3C DM uses RFC 3987 IRI encoding: the *iunreserved* set passes through, so
/// ASCII specials are `%XX`-escaped but non-ASCII Unicode (ucschar — e.g. CJK
/// table/column names) is emitted verbatim, yielding an IRI rather than a URI.
fn encode(name: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii() {
            let b = ch as u8;
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                out.push(ch);
            } else {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::ir::TermType;
    use sf_core::term::generate_into;
    use sf_sql::{Column, ForeignKey};

    const BASE: &str = "http://example.com/base/";

    fn single_pk_table() -> TableSchema {
        let mut t = TableSchema::new("employees");
        t.columns = vec![
            Column::new("id", "integer", true),
            Column::new("name", "text", false),
        ];
        t.primary_key = vec!["id".to_owned()];
        t
    }

    // --- subject map: single-column primary key -----------------------------

    #[test]
    fn single_pk_table_produces_iri_template_subject() {
        let t = single_pk_table();
        let tm = table_map(&t, BASE).expect("table_map succeeds");

        assert_eq!(tm.id, "http://example.com/base/employees");
        match &tm.subject.term {
            TermMap::Template(template, spec) => {
                assert_eq!(spec.term_type, TermType::Iri);
                assert_eq!(
                    template.segments(),
                    &[
                        Segment::Literal("http://example.com/base/employees/".into()),
                        Segment::Literal("id=".into()),
                        Segment::Column("id".into()),
                    ]
                );
            }
            other => panic!("expected a Template subject term, got {other:?}"),
        }
    }

    #[test]
    fn single_pk_table_attaches_table_class() {
        let t = single_pk_table();
        let tm = table_map(&t, BASE).expect("table_map succeeds");

        assert_eq!(tm.subject.classes.len(), 1);
        assert_eq!(
            tm.subject.classes[0].as_str(),
            "http://example.com/base/employees"
        );
    }

    #[test]
    fn single_pk_table_produces_one_pom_per_column() {
        let t = single_pk_table();
        let tm = table_map(&t, BASE).expect("table_map succeeds");

        // Two columns (id, name), no FKs ⇒ exactly two predicate-object maps.
        assert_eq!(tm.predicate_object_maps.len(), 2);

        let name_pom = &tm.predicate_object_maps[1];
        match &name_pom.predicates[0] {
            TermMap::Constant(Term::NamedNode(n)) => {
                assert_eq!(n.as_str(), "http://example.com/base/employees#name");
            }
            other => panic!("expected a constant IRI predicate, got {other:?}"),
        }
        match &name_pom.objects[0] {
            ObjectMap::Term(TermMap::Column(col, spec)) => {
                assert_eq!(&**col, "name");
                assert_eq!(spec.term_type, TermType::Literal);
                assert!(spec.datatype.is_none());
                assert!(spec.language.is_none());
            }
            other => panic!("expected a Column literal object, got {other:?}"),
        }
    }

    // --- subject map: composite primary key ----------------------------------

    #[test]
    fn composite_pk_table_joins_segments_with_semicolon() {
        let mut t = TableSchema::new("order_items");
        t.columns = vec![
            Column::new("order_id", "integer", true),
            Column::new("line_no", "integer", true),
        ];
        t.primary_key = vec!["order_id".to_owned(), "line_no".to_owned()];

        let tm = table_map(&t, BASE).expect("table_map succeeds");
        match &tm.subject.term {
            TermMap::Template(template, _) => {
                assert_eq!(
                    template.segments(),
                    &[
                        Segment::Literal("http://example.com/base/order_items/".into()),
                        Segment::Literal("order_id=".into()),
                        Segment::Column("order_id".into()),
                        Segment::Literal(";line_no=".into()),
                        Segment::Column("line_no".into()),
                    ]
                );
            }
            other => panic!("expected a Template subject term, got {other:?}"),
        }
    }

    // --- subject map: no primary key ⇒ blank node -----------------------------

    #[test]
    fn no_pk_table_uses_a_rowid_keyed_blank_node() {
        let mut t = TableSchema::new("log");
        t.columns = vec![Column::new("message", "text", false)];
        // primary_key left empty.

        let tm = table_map(&t, BASE).expect("table_map succeeds");
        match &tm.subject.term {
            TermMap::Template(template, spec) => {
                assert_eq!(spec.term_type, TermType::BlankNode);
                assert_eq!(
                    template.segments(),
                    &[
                        Segment::Literal("log_".into()),
                        Segment::Column("rowid".into()),
                    ]
                );
            }
            other => panic!("expected a Template blank-node subject term, got {other:?}"),
        }
    }

    #[test]
    fn no_pk_table_still_gets_the_table_class() {
        let mut t = TableSchema::new("log");
        t.columns = vec![Column::new("message", "text", false)];

        let tm = table_map(&t, BASE).expect("table_map succeeds");
        assert_eq!(
            tm.subject.classes[0].as_str(),
            "http://example.com/base/log"
        );
    }

    // --- foreign keys ----------------------------------------------------------

    #[test]
    fn single_column_fk_generates_referencing_object_map() {
        let mut t = single_pk_table();
        t.foreign_keys = vec![ForeignKey {
            columns: vec!["dept_id".to_owned()],
            parent_table: "departments".to_owned(),
            parent_columns: vec!["id".to_owned()],
        }];

        let tm = table_map(&t, BASE).expect("table_map succeeds");
        // 2 column POMs (id, name) + 1 FK POM.
        assert_eq!(tm.predicate_object_maps.len(), 3);

        let fk_pom = &tm.predicate_object_maps[2];
        match &fk_pom.predicates[0] {
            TermMap::Constant(Term::NamedNode(n)) => {
                assert_eq!(n.as_str(), "http://example.com/base/employees#ref-dept_id");
            }
            other => panic!("expected a constant IRI predicate, got {other:?}"),
        }
        match &fk_pom.objects[0] {
            ObjectMap::Ref(rom) => {
                assert_eq!(
                    rom.parent_triples_map,
                    "http://example.com/base/departments"
                );
                assert_eq!(rom.joins.len(), 1);
                assert_eq!(rom.joins[0].child, "dept_id");
                assert_eq!(rom.joins[0].parent, "id");
            }
            other => panic!("expected a referencing object map, got {other:?}"),
        }
    }

    #[test]
    fn composite_fk_joins_column_names_with_semicolon_in_predicate() {
        let mut t = single_pk_table();
        t.foreign_keys = vec![ForeignKey {
            columns: vec!["a".to_owned(), "b".to_owned()],
            parent_table: "parent".to_owned(),
            parent_columns: vec!["pa".to_owned(), "pb".to_owned()],
        }];

        let tm = table_map(&t, BASE).expect("table_map succeeds");
        let fk_pom = tm.predicate_object_maps.last().expect("fk pom present");
        match &fk_pom.predicates[0] {
            TermMap::Constant(Term::NamedNode(n)) => {
                assert_eq!(n.as_str(), "http://example.com/base/employees#ref-a;b");
            }
            other => panic!("expected a constant IRI predicate, got {other:?}"),
        }
        match &fk_pom.objects[0] {
            ObjectMap::Ref(rom) => {
                assert_eq!(rom.joins[0].child, "a");
                assert_eq!(rom.joins[0].parent, "pa");
                assert_eq!(rom.joins[1].child, "b");
                assert_eq!(rom.joins[1].parent, "pb");
            }
            other => panic!("expected a referencing object map, got {other:?}"),
        }
    }

    // --- NULL column handling ---------------------------------------------------
    //
    // direct_mapping.rs itself carries no NULL-specific logic: it emits a plain
    // `TermMap::Column` object map per column, and NULL-omission (W3C DM §2 / R2RML
    // §11: no column value ⇒ no triple) is enforced once, generically, by
    // `sf_core::term::generate_into` for *every* `rr:column` object map — Direct
    // Mapping gets it "for free" rather than re-implementing it. These tests pin
    // that the object map this module builds actually drives that shared path
    // correctly in both directions (see final report note).

    #[test]
    fn null_column_value_yields_no_term_via_the_generated_object_map() {
        let t = single_pk_table();
        let tm = table_map(&t, BASE).expect("table_map succeeds");
        let name_pom = &tm.predicate_object_maps[1]; // the `name` column POM
        let ObjectMap::Term(term_map) = &name_pom.objects[0] else {
            panic!("expected a Term object map");
        };

        let row: &[(&str, Option<&str>)] = &[("id", Some("1")), ("name", None)];
        let mut buf = String::new();
        let generated = generate_into(term_map, row, &mut buf).expect("generation succeeds");
        assert!(generated.is_none(), "NULL column must yield no term/triple");
    }

    #[test]
    fn non_null_column_value_yields_a_literal_via_the_generated_object_map() {
        let t = single_pk_table();
        let tm = table_map(&t, BASE).expect("table_map succeeds");
        let name_pom = &tm.predicate_object_maps[1];
        let ObjectMap::Term(term_map) = &name_pom.objects[0] else {
            panic!("expected a Term object map");
        };

        let row: &[(&str, Option<&str>)] = &[("id", Some("1")), ("name", Some("Ada"))];
        let mut buf = String::new();
        let generated = generate_into(term_map, row, &mut buf).expect("generation succeeds");
        match generated {
            Some(sf_core::term::GenTerm::Literal(lit)) => assert_eq!(lit.value(), "Ada"),
            other => panic!("expected a literal term, got {other:?}"),
        }
    }

    // --- direct_mapping(): multi-table driver -----------------------------------

    #[test]
    fn direct_mapping_produces_one_triples_map_per_table() {
        let tables = vec![single_pk_table(), TableSchema::new("log")];
        let maps = direct_mapping(&tables, BASE).expect("direct_mapping succeeds");
        assert_eq!(maps.len(), 2);
        assert_eq!(maps[0].id, "http://example.com/base/employees");
        assert_eq!(maps[1].id, "http://example.com/base/log");
    }

    #[test]
    fn invalid_base_iri_surfaces_as_a_mapping_error() {
        let t = single_pk_table();
        // A space is not valid in an IRI, and `base` is used verbatim (only table/
        // column *names* are percent-encoded), so this must fail NamedNode parsing.
        let err = table_map(&t, "not a valid base ").unwrap_err();
        match err {
            sf_core::Error::Mapping(msg) => {
                assert!(
                    msg.contains("invalid DM class IRI"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected a Mapping error, got {other:?}"),
        }
    }

    // --- name encoding (fixed IRI parts: table/column names, not row values) ---

    #[test]
    fn encode_passes_unreserved_ascii_through_untouched() {
        assert_eq!(encode("Table-Name_1.2~3"), "Table-Name_1.2~3");
    }

    #[test]
    fn encode_percent_encodes_ascii_specials_with_uppercase_hex() {
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(encode("a#b"), "a%23b");
        assert_eq!(encode("a/b"), "a%2Fb");
    }

    #[test]
    fn encode_passes_non_ascii_through_verbatim() {
        // W3C DM uses RFC 3987 IRI encoding: ucschar (e.g. CJK) is not %-escaped,
        // unlike a strict RFC 3986 URI encoder.
        assert_eq!(encode("café"), "café");
        assert_eq!(encode("表"), "表");
    }

    #[test]
    fn fk_predicate_name_encodes_special_characters_in_column_names() {
        let mut t = single_pk_table();
        t.foreign_keys = vec![ForeignKey {
            columns: vec!["dept id".to_owned()],
            parent_table: "departments".to_owned(),
            parent_columns: vec!["id".to_owned()],
        }];
        let tm = table_map(&t, BASE).expect("table_map succeeds");
        let fk_pom = tm.predicate_object_maps.last().expect("fk pom present");
        match &fk_pom.predicates[0] {
            TermMap::Constant(Term::NamedNode(n)) => {
                assert_eq!(
                    n.as_str(),
                    "http://example.com/base/employees#ref-dept%20id"
                );
            }
            other => panic!("expected a constant IRI predicate, got {other:?}"),
        }
    }
}
