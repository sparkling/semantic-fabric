//! The cross-repo `M ⋈ T` SHACL gate over the vendored mapping-output validation
//! shapes (ADR-0005 / ADR-0019): rudof `shacl` in Native mode validates a small
//! `M ⋈ T` closure (a source-mapping graph joined with a minimal generated model
//! `T`) and must flag a dangling mapping target.

use std::path::PathBuf;

use sf_conformance::mapping_conforms_to_t;

fn shapes() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/m-join-t/meta-shapes.ttl");
    std::fs::read_to_string(p).expect("vendored meta-shapes present")
}

/// A conforming closure: every `rr:class` / `rr:predicate` target is declared in T.
const CONFORMING: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex:  <http://ex/> .
ex:tm rr:class ex:Person .
ex:pom rr:predicate ex:worksFor .
ex:Person a owl:Class .
ex:worksFor a owl:ObjectProperty .
"#;

/// A drifting closure: `ex:Ghost` is a mapping target class but is not declared in T.
const DANGLING: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex:  <http://ex/> .
ex:tm rr:class ex:Ghost .
ex:pom rr:predicate ex:worksFor .
ex:worksFor a owl:ObjectProperty .
"#;

#[test]
fn m_join_t_gate_passes_a_conforming_closure() {
    let out = mapping_conforms_to_t(CONFORMING, &shapes()).expect("Native validation runs");
    assert!(out.conforms, "conforming M⋈T closure must pass the gate: {out:?}");
    assert_eq!(out.violations, 0);
}

#[test]
fn m_join_t_gate_flags_a_dangling_target_class() {
    let out = mapping_conforms_to_t(DANGLING, &shapes()).expect("Native validation runs");
    assert!(
        !out.conforms,
        "a dangling rr:class target is an M-vs-T drift and must violate the gate: {out:?}"
    );
    assert!(out.violations >= 1);
}
