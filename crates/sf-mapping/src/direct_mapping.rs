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
