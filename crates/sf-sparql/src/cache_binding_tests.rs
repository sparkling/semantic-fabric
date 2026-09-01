//! Adversarial tests for the immutable compiler/cache binding.

use sf_core::{SourceId, SourceMapping};
use sf_sql::Dialect;
use spargebra::SparqlParser;

use crate::cache::{self, CachedPlan};
use crate::{translate_cached, translate_with, CompilerBinding, Error, Tbox};

fn binding(source_id: SourceId, dialect: Dialect) -> CompilerBinding {
    CompilerBinding::new(
        SourceMapping::new(source_id, Vec::new()),
        dialect,
        Tbox::default(),
        Vec::new(),
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
