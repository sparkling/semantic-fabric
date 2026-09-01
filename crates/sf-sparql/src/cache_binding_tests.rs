//! Adversarial tests for the immutable compiler/cache binding.

use rusqlite::Connection;
use sf_core::ir::{
    Join, LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, SubjectMap, Template,
    TermMap, TermSpec, TriplesMap,
};
use sf_core::{NamedNode, SourceId, SourceMapping};
use sf_sql::{Column, Dialect, ForeignKey, FunctionalDep, TableSchema};
use spargebra::SparqlParser;

use crate::cache::{self, CachedPlan};
use crate::{
    exec, parse_and_translate_with, translate_cached, translate_with, CompilerBinding,
    CompilerSchema, ConstraintAuthority, Error, Tbox,
};

fn binding(source_id: SourceId, dialect: Dialect) -> CompilerBinding {
    CompilerBinding::new(
        SourceMapping::new(source_id, Vec::new()),
        dialect,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(Vec::new()),
        8,
    )
}

#[test]
fn translate_cached_reuses_only_within_one_immutable_binding() {
    // The cache is consulted on the compile path (not dead infrastructure), but
    // a replacement binding — including a dialect change — owns a new scope and
    // cache, so a PostgreSQL plan can never poison SQLite.
    let query = SparqlParser::new()
        .parse_query("SELECT * WHERE { ?s ?p ?o }")
        .unwrap();
    let source_id = SourceId::new(0).unwrap();
    let sqlite = binding(source_id, Dialect::Sqlite);
    assert_eq!(sqlite.cache_len(), 0);
    let first = translate_cached(&query, &sqlite).unwrap();
    assert_eq!(first.dialect, Dialect::Sqlite);
    assert_eq!(sqlite.cache_len(), 1, "first compile populates the cache");
    let second = translate_cached(&query, &sqlite).unwrap();
    assert_eq!(second.dialect, Dialect::Sqlite);
    assert_eq!(sqlite.cache_len(), 1, "second call is a hit");

    let postgres = binding(source_id, Dialect::Postgres);
    assert_ne!(sqlite.scope(), postgres.scope());
    let pg_plan = translate_cached(&query, &postgres).unwrap();
    assert_eq!(pg_plan.dialect, Dialect::Postgres);
    assert_eq!(postgres.cache_len(), 1);
}

#[test]
fn deliberately_misscoped_cached_artifact_fails_closed() {
    let query = SparqlParser::new()
        .parse_query("SELECT * WHERE { ?s ?p ?o }")
        .unwrap();
    let source_id = SourceId::new(0).unwrap();
    let current = binding(source_id, Dialect::Sqlite);
    let other = binding(source_id, Dialect::Sqlite);
    let plan = translate_with(&query, &[], Dialect::Sqlite, &Tbox::default(), &[]).unwrap();
    let key = cache::plan_key(&query, current.scope());
    current
        .cache()
        .put(key, CachedPlan::new(other.scope(), plan));

    assert!(matches!(
        translate_cached(&query, &current),
        Err(Error::Mapping(message)) if message == "compiled-plan cache scope mismatch"
    ));
}

fn constraint_rich_schema() -> TableSchema {
    let mut table = TableSchema::new("items");
    table.columns = vec![
        Column {
            name: "id".to_owned(),
            sql_type: "integer".to_owned(),
            not_null: true,
            distinct_estimate: Some(2),
        },
        Column::new("a", "text", true),
        Column::new("b", "text", true),
    ];
    table.primary_key = vec!["id".to_owned()];
    table.unique = vec![vec!["a".to_owned()]];
    table.foreign_keys = vec![ForeignKey {
        columns: vec!["id".to_owned()],
        parent_table: "parents".to_owned(),
        parent_columns: vec!["id".to_owned()],
    }];
    table.functional_dependencies = vec![FunctionalDep {
        det: vec!["a".to_owned()],
        dep: vec!["b".to_owned()],
    }];
    table.row_estimate = Some(2);
    table
}

#[test]
fn compiler_binding_quarantines_unverified_integrity_constraints() {
    let source_id = SourceId::new(0).unwrap();
    let binding = CompilerBinding::new(
        SourceMapping::new(source_id, Vec::new()),
        Dialect::Sqlite,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(vec![constraint_rich_schema()]),
        8,
    );

    assert_eq!(
        binding.constraint_authority(),
        ConstraintAuthority::Unverified
    );
    assert_eq!(
        binding.scope().constraint_authority(),
        ConstraintAuthority::Unverified
    );
    let table = &binding.schema()[0];
    assert!(table.primary_key.is_empty());
    assert!(table.unique.is_empty());
    assert!(table.foreign_keys.is_empty());
    assert!(table.functional_dependencies.is_empty());
    assert!(table.columns.iter().all(|column| !column.not_null));

    assert_eq!(table.name, "items");
    assert_eq!(
        table
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.sql_type.as_str()))
            .collect::<Vec<_>>(),
        vec![("id", "integer"), ("a", "text"), ("b", "text")]
    );
    assert_eq!(table.columns[0].distinct_estimate, Some(2));
    assert_eq!(table.row_estimate, Some(2));
}

fn predicate_object(predicate: &str, column: &str) -> PredicateObjectMap {
    PredicateObjectMap {
        predicates: vec![TermMap::Constant(
            NamedNode::new_unchecked(predicate).into(),
        )],
        objects: vec![ObjectMap::Term(TermMap::Column(
            column.to_owned().into(),
            TermSpec::plain_literal(),
        ))],
        graphs: Vec::new(),
    }
}

fn stale_key_mapping(source_id: SourceId) -> SourceMapping {
    SourceMapping::new(
        source_id,
        vec![TriplesMap {
            id: "items".to_owned(),
            source: LogicalSource::Table("items".to_owned()),
            subject: SubjectMap {
                term: TermMap::Template(
                    Template::parse("http://example.test/item/{id}").unwrap(),
                    TermSpec::iri(),
                ),
                classes: Vec::new(),
                graphs: Vec::new(),
            },
            predicate_object_maps: vec![
                predicate_object("http://example.test/a", "a"),
                predicate_object("http://example.test/b", "b"),
            ],
        }],
    )
}

fn assert_stale_key_cannot_change_cached_query_results(schema: TableSchema) {
    let source_id = SourceId::new(0).unwrap();
    let mapping = stale_key_mapping(source_id);
    let query = "SELECT ?a ?b WHERE { \
        ?s <http://example.test/a> ?a ; <http://example.test/b> ?b \
    }";
    let poisoned = parse_and_translate_with(
        query,
        mapping.triples_maps(),
        Dialect::Sqlite,
        &Tbox::default(),
        std::slice::from_ref(&schema),
    )
    .unwrap();
    let binding = CompilerBinding::new(
        mapping,
        Dialect::Sqlite,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(vec![schema]),
        8,
    );
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items(id INTEGER, a TEXT, b TEXT);\
         INSERT INTO items VALUES (1, 'a1', 'b1'), (1, 'a2', 'b2');",
    )
    .unwrap();

    assert_eq!(
        exec::select(&poisoned, &conn).unwrap().rows.len(),
        2,
        "the raw stale constraint must poison this fixture, or the safe assertion is vacuous"
    );

    for _ in 0..2 {
        let plan = binding.compile(query).unwrap();
        let solutions = exec::select(&plan, &conn).unwrap();
        assert_eq!(
            solutions.rows.len(),
            4,
            "the two property relations join on RDF subject; stale PK metadata must not collapse them"
        );
    }
    assert_eq!(binding.cache_len(), 1, "the second compile is a cache hit");
}

#[test]
fn stale_primary_key_cannot_change_cached_query_results() {
    let mut schema = constraint_rich_schema();
    schema.unique.clear();
    schema.foreign_keys.clear();
    schema.functional_dependencies.clear();
    assert_stale_key_cannot_change_cached_query_results(schema);
}

#[test]
fn stale_unique_key_cannot_change_cached_query_results() {
    let mut schema = constraint_rich_schema();
    schema.primary_key.clear();
    schema.unique = vec![vec!["id".to_owned()]];
    schema.foreign_keys.clear();
    schema.functional_dependencies.clear();
    assert_stale_key_cannot_change_cached_query_results(schema);
}

fn stale_fk_mapping(source_id: SourceId) -> SourceMapping {
    let parent = TriplesMap {
        id: "parents".to_owned(),
        source: LogicalSource::Table("parents".to_owned()),
        subject: SubjectMap {
            term: TermMap::Template(
                Template::parse("http://example.test/parent/{id}").unwrap(),
                TermSpec::iri(),
            ),
            classes: Vec::new(),
            graphs: Vec::new(),
        },
        predicate_object_maps: Vec::new(),
    };
    let child = TriplesMap {
        id: "children".to_owned(),
        source: LogicalSource::Table("children".to_owned()),
        subject: SubjectMap {
            term: TermMap::Template(
                Template::parse("http://example.test/child/{id}").unwrap(),
                TermSpec::iri(),
            ),
            classes: Vec::new(),
            graphs: Vec::new(),
        },
        predicate_object_maps: vec![PredicateObjectMap {
            predicates: vec![TermMap::Constant(
                NamedNode::new_unchecked("http://example.test/parent").into(),
            )],
            objects: vec![ObjectMap::Ref(RefObjectMap {
                parent_triples_map: "parents".to_owned(),
                joins: vec![Join {
                    child: "parent_id".to_owned(),
                    parent: "id".to_owned(),
                }],
            })],
            graphs: Vec::new(),
        }],
    };
    SourceMapping::new(source_id, vec![child, parent])
}

fn stale_fk_schema() -> Vec<TableSchema> {
    let mut child = TableSchema::new("children");
    child.columns = vec![
        Column::new("id", "integer", true),
        Column::new("parent_id", "integer", true),
    ];
    child.primary_key = vec!["id".to_owned()];
    child.foreign_keys = vec![ForeignKey {
        columns: vec!["parent_id".to_owned()],
        parent_table: "parents".to_owned(),
        parent_columns: vec!["id".to_owned()],
    }];
    let mut parent = TableSchema::new("parents");
    parent.columns = vec![Column::new("id", "integer", true)];
    parent.primary_key = vec!["id".to_owned()];
    vec![child, parent]
}

#[test]
fn stale_foreign_key_cannot_invent_a_reference_triple() {
    let source_id = SourceId::new(0).unwrap();
    let mapping = stale_fk_mapping(source_id);
    let schema = stale_fk_schema();
    let query = "SELECT ?parent WHERE { \
        ?child <http://example.test/parent> ?parent \
    }";
    let poisoned = parse_and_translate_with(
        query,
        mapping.triples_maps(),
        Dialect::Sqlite,
        &Tbox::default(),
        &schema,
    )
    .unwrap();
    let binding = CompilerBinding::new(
        mapping,
        Dialect::Sqlite,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(schema),
        8,
    );
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE children(id INTEGER PRIMARY KEY, parent_id INTEGER);\
         CREATE TABLE parents(id INTEGER PRIMARY KEY);\
         INSERT INTO children VALUES (1, 99);",
    )
    .unwrap();

    assert_eq!(
        exec::select(&poisoned, &conn).unwrap().rows.len(),
        1,
        "the stale FK/PK pair must poison this fixture, or the safe assertion is vacuous"
    );
    for _ in 0..2 {
        assert_eq!(
            exec::select(&binding.compile(query).unwrap(), &conn)
                .unwrap()
                .rows
                .len(),
            0,
            "an orphan row cannot generate a referencing-object triple"
        );
    }
    assert_eq!(binding.cache_len(), 1);
}

fn stale_fd_mapping(source_id: SourceId) -> SourceMapping {
    SourceMapping::new(
        source_id,
        vec![TriplesMap {
            id: "items".to_owned(),
            source: LogicalSource::Table("items".to_owned()),
            subject: SubjectMap {
                term: TermMap::Template(
                    Template::parse("http://example.test/group/{det}").unwrap(),
                    TermSpec::iri(),
                ),
                classes: Vec::new(),
                graphs: Vec::new(),
            },
            predicate_object_maps: vec![
                predicate_object("http://example.test/a", "id"),
                predicate_object("http://example.test/b", "id"),
            ],
        }],
    )
}

fn stale_fd_schema() -> TableSchema {
    let mut table = TableSchema::new("items");
    table.columns = vec![
        Column::new("id", "integer", true),
        Column::new("det", "text", true),
    ];
    table.primary_key = vec!["id".to_owned()];
    table.functional_dependencies = vec![FunctionalDep {
        det: vec!["det".to_owned()],
        dep: vec!["id".to_owned()],
    }];
    table
}

#[test]
fn stale_functional_dependency_cannot_collapse_distinct_self_join() {
    let source_id = SourceId::new(0).unwrap();
    let mapping = stale_fd_mapping(source_id);
    let schema = stale_fd_schema();
    let query = "SELECT DISTINCT ?a ?b WHERE { \
        ?s <http://example.test/a> ?a ; <http://example.test/b> ?b \
    }";
    let poisoned = parse_and_translate_with(
        query,
        mapping.triples_maps(),
        Dialect::Sqlite,
        &Tbox::default(),
        std::slice::from_ref(&schema),
    )
    .unwrap();
    let binding = CompilerBinding::new(
        mapping,
        Dialect::Sqlite,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(vec![schema]),
        8,
    );
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items(id INTEGER PRIMARY KEY, det TEXT NOT NULL);\
         INSERT INTO items VALUES (1, 'same'), (2, 'same');",
    )
    .unwrap();

    assert_eq!(
        exec::select(&poisoned, &conn).unwrap().rows.len(),
        2,
        "the stale FD must poison this fixture, or the safe assertion is vacuous"
    );
    for _ in 0..2 {
        assert_eq!(
            exec::select(&binding.compile(query).unwrap(), &conn)
                .unwrap()
                .rows
                .len(),
            4,
            "without FD authority the two RDF property relations form four distinct pairs"
        );
    }
    assert_eq!(binding.cache_len(), 1);
}
