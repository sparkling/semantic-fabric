use super::*;
use spargebra::SparqlParser;

fn parse(q: &str) -> Query {
    SparqlParser::new().parse_query(q).unwrap()
}

fn scope(dialect: Dialect, epoch: Epoch) -> CompileScope {
    CompileScope::new(
        CompileBindingId::mint(),
        dialect,
        epoch,
        ConstraintAuthority::Unverified,
        ColumnTypeAuthority::Unverified,
    )
}

#[test]
fn same_query_same_key() {
    let scope = scope(Dialect::Sqlite, Epoch(3));
    let a = plan_key(&parse("SELECT * WHERE { ?s ?p ?o }"), scope);
    let b = plan_key(&parse("SELECT * WHERE { ?s ?p ?o }"), scope);
    assert_eq!(a, b);
}

#[test]
fn schema_selecting_constant_changes_key() {
    let scope = scope(Dialect::Sqlite, Epoch(0));
    let a = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/a> ?y }"), scope);
    let b = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/b> ?y }"), scope);
    assert_ne!(a, b);
}

#[test]
fn hash_collision_does_not_serve_the_wrong_plan() {
    let cache: PlanCache<u32> = PlanCache::new(8);
    let scope = scope(Dialect::Sqlite, Epoch(0));
    let mut ka = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/a> ?y }"), scope);
    let mut kb = plan_key(&parse("SELECT ?x WHERE { ?x <http://ex/b> ?y }"), scope);
    ka.structural_hash = 42;
    kb.structural_hash = 42;
    assert_ne!(ka, kb);
    cache.put(ka.clone(), 1);
    assert_eq!(cache.get(&ka), Some(1));
    assert_eq!(cache.get(&kb), None);
}

#[test]
fn epoch_bump_invalidates() {
    let q = parse("SELECT * WHERE { ?s ?p ?o }");
    let binding = CompileBindingId::mint();
    assert_ne!(
        plan_key(
            &q,
            CompileScope::new(
                binding,
                Dialect::Sqlite,
                Epoch(1),
                ConstraintAuthority::Unverified,
                ColumnTypeAuthority::Unverified,
            ),
        ),
        plan_key(
            &q,
            CompileScope::new(
                binding,
                Dialect::Sqlite,
                Epoch(2),
                ConstraintAuthority::Unverified,
                ColumnTypeAuthority::Unverified,
            ),
        )
    );
}

#[test]
#[should_panic(expected = "compile epoch exhausted")]
fn epoch_exhaustion_fails_instead_of_wrapping() {
    let mut epoch = Epoch(u64::MAX);
    epoch.bump();
}

#[test]
fn dialect_and_binding_identity_are_part_of_the_key() {
    let q = parse("SELECT * WHERE { ?s ?p ?o }");
    let binding = CompileBindingId::mint();
    let sqlite = CompileScope::new(
        binding,
        Dialect::Sqlite,
        Epoch(0),
        ConstraintAuthority::Unverified,
        ColumnTypeAuthority::Unverified,
    );
    let postgres = CompileScope::new(
        binding,
        Dialect::Postgres,
        Epoch(0),
        ConstraintAuthority::Unverified,
        ColumnTypeAuthority::Unverified,
    );
    let other_binding = scope(Dialect::Sqlite, Epoch(0));

    assert_ne!(plan_key(&q, sqlite), plan_key(&q, postgres));
    assert_ne!(plan_key(&q, sqlite), plan_key(&q, other_binding));
}

#[test]
fn cache_round_trips_and_is_bounded() {
    let cache: PlanCache<u32> = PlanCache::new(2);
    let scope = scope(Dialect::Sqlite, Epoch(0));
    let k1 = plan_key(&parse("SELECT * WHERE { ?a ?b ?c }"), scope);
    cache.put(k1.clone(), 10);
    assert_eq!(cache.get(&k1), Some(10));
    cache.put(plan_key(&parse("SELECT * WHERE { ?d ?e ?f }"), scope), 20);
    cache.put(plan_key(&parse("SELECT * WHERE { ?g ?h ?i }"), scope), 30);
    assert!(cache.len() <= 2);
}

fn synth_key(scope: CompileScope, id: usize) -> PlanKey {
    PlanKey {
        scope,
        structural_hash: id as u64,
        canonical: format!("synthetic-plan-{id}"),
    }
}

#[test]
fn hot_working_set_survives_cold_churn_past_capacity() {
    const CAPACITY: usize = 64;
    const HOT: usize = 32;
    const COLD: usize = 128;
    const ITERS: usize = 3000;

    let cache: PlanCache<u32> = PlanCache::new(CAPACITY);
    let scope = scope(Dialect::Sqlite, Epoch(0));
    let mut hits = 0u32;
    let mut accesses = 0u32;
    for i in 0..ITERS {
        let key = if i % 3 != 0 {
            synth_key(scope, i % HOT)
        } else {
            synth_key(scope, HOT + (i / 3) % COLD)
        };
        accesses += 1;
        if cache.get(&key).is_some() {
            hits += 1;
        } else {
            cache.put(key, i as u32);
        }
    }
    let hit_rate = f64::from(hits) / f64::from(accesses);
    eprintln!(
        "PlanCache hot/cold hit rate over {ITERS} accesses ({HOT} hot + {COLD} cold keys, \
         capacity {CAPACITY}): {hits}/{accesses} = {hit_rate:.3}"
    );
    assert!(
        hit_rate > 0.5,
        "hot working set should survive cold churn past capacity, got hit_rate={hit_rate:.3}"
    );
}
