use super::*;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

fn digest(policy: &ValidationPolicy, shape: [u8; 32], select: [u8; 32]) -> [u8; 32] {
    digest_policy(policy, shape, select).unwrap()
}

#[test]
fn policy_v2_has_an_exact_known_answer() {
    let actual = digest(
        &POLICY,
        REVIEWED_SHAPE_SET_DIGEST,
        crate::batched_datatype::SELECT_DIGEST,
    );
    let actual = actual
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        actual,
        "c69b76dc85e8039f7476a3e626fe0fba5602a2335000a95f98aa698afb863c51"
    );
}

#[test]
fn every_policy_field_partitions_the_receipt_identity() {
    let baseline = digest(
        &POLICY,
        REVIEWED_SHAPE_SET_DIGEST,
        crate::batched_datatype::SELECT_DIGEST,
    );
    let mut cases = Vec::new();

    let mut changed = POLICY;
    changed.graph_limits.max_utf8_bytes += 1;
    cases.push((
        "max_utf8_bytes",
        digest(
            &changed,
            REVIEWED_SHAPE_SET_DIGEST,
            crate::batched_datatype::SELECT_DIGEST,
        ),
    ));
    changed = POLICY;
    changed.graph_limits.max_parsed_triples += 1;
    cases.push((
        "max_parsed_triples",
        digest(
            &changed,
            REVIEWED_SHAPE_SET_DIGEST,
            crate::batched_datatype::SELECT_DIGEST,
        ),
    ));
    changed = POLICY;
    changed.validation_limits.max_work_units += 1;
    cases.push((
        "max_work_units",
        digest(
            &changed,
            REVIEWED_SHAPE_SET_DIGEST,
            crate::batched_datatype::SELECT_DIGEST,
        ),
    ));
    changed = POLICY;
    changed.validation_limits.max_result_cardinality += 1;
    cases.push((
        "max_result_cardinality",
        digest(
            &changed,
            REVIEWED_SHAPE_SET_DIGEST,
            crate::batched_datatype::SELECT_DIGEST,
        ),
    ));
    changed = POLICY;
    changed.preflight_revision = b"mutated";
    cases.push((
        "preflight_revision",
        digest(
            &changed,
            REVIEWED_SHAPE_SET_DIGEST,
            crate::batched_datatype::SELECT_DIGEST,
        ),
    ));
    changed = POLICY;
    changed.blank_datatype_focus = BlankDatatypeFocusPolicy::AllowGlobal;
    cases.push((
        "blank_datatype_focus",
        digest(
            &changed,
            REVIEWED_SHAPE_SET_DIGEST,
            crate::batched_datatype::SELECT_DIGEST,
        ),
    ));
    changed = POLICY;
    changed.execution_topology = b"mutated";
    cases.push((
        "execution_topology",
        digest(
            &changed,
            REVIEWED_SHAPE_SET_DIGEST,
            crate::batched_datatype::SELECT_DIGEST,
        ),
    ));
    changed = POLICY;
    changed.feature_profile = b"mutated";
    cases.push((
        "feature_profile",
        digest(
            &changed,
            REVIEWED_SHAPE_SET_DIGEST,
            crate::batched_datatype::SELECT_DIGEST,
        ),
    ));

    macro_rules! engine_case {
        ($name:literal, $field:ident) => {{
            let mut changed = POLICY;
            changed.engines.$field = b"mutated";
            cases.push((
                $name,
                digest(
                    &changed,
                    REVIEWED_SHAPE_SET_DIGEST,
                    crate::batched_datatype::SELECT_DIGEST,
                ),
            ));
        }};
    }
    engine_case!("shacl", shacl);
    engine_case!("rudof_rdf", rudof_rdf);
    engine_case!("sparql_service", sparql_service);
    engine_case!("oxigraph", oxigraph);
    engine_case!("spareval", spareval);
    engine_case!("spargebra", spargebra);
    engine_case!("oxrdf", oxrdf);
    engine_case!("oxttl", oxttl);
    engine_case!("oxrdfio", oxrdfio);

    cases.push((
        "shape_digest",
        digest(&POLICY, [0; 32], crate::batched_datatype::SELECT_DIGEST),
    ));
    cases.push((
        "select_digest",
        digest(&POLICY, REVIEWED_SHAPE_SET_DIGEST, [0; 32]),
    ));

    for (field, changed_digest) in cases {
        assert_ne!(baseline, changed_digest, "field {field} was not bound");
    }
}

#[test]
fn configured_dependency_identities_match_package_scoped_cargo_tree() {
    let configured = [
        ("shacl", POLICY.engines.shacl, "0.3.14", "default,sparql"),
        (
            "rudof_rdf",
            POLICY.engines.rudof_rdf,
            "0.3.14",
            "default,sparql",
        ),
        (
            "sparql_service",
            POLICY.engines.sparql_service,
            "0.3.14",
            "default,sparql",
        ),
        (
            "oxigraph",
            POLICY.engines.oxigraph,
            "0.5.9",
            "http-client,http-client-rustls-native,oxhttp,rdf-12",
        ),
        (
            "spareval",
            POLICY.engines.spareval,
            "0.2.6",
            "calendar-ext,default,sep-0002,sep-0006,sparql-12",
        ),
        (
            "spargebra",
            POLICY.engines.spargebra,
            "0.4.6",
            "default,sep-0002,sep-0006,sparql-12",
        ),
        (
            "oxrdf",
            POLICY.engines.oxrdf,
            "0.3.3",
            "default,oxsdatatypes,rdf-12,rdfc-10",
        ),
        ("oxttl", POLICY.engines.oxttl, "0.2.3", "default,rdf-12"),
        ("oxrdfio", POLICY.engines.oxrdfio, "0.2.5", "default,rdf-12"),
    ];
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("Cargo.toml");
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "tree",
            "-p",
            "sf-validation",
            "--locked",
            "--offline",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}\t{f}",
        ])
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .expect("cargo tree must be executable");
    assert!(
        output.status.success(),
        "package-scoped cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("cargo tree output must be UTF-8");
    let observed = stdout
        .lines()
        .map(|line| line.strip_suffix(" (*)").unwrap_or(line))
        .filter(|line| {
            configured
                .iter()
                .any(|(name, _, _, _)| line.starts_with(&format!("{name} v")))
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let expected = configured
        .iter()
        .map(|(name, identity, version, features)| {
            assert_eq!(*identity, format!("{name}/{version}").as_bytes());
            format!("{name} v{version}\t{features}")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(observed, expected);

    let expected_profile = configured
        .iter()
        .map(|(name, _, _, features)| format!("{name}={}", features.replace(',', "+")))
        .collect::<Vec<_>>()
        .join(";");
    assert_eq!(POLICY.feature_profile, expected_profile.as_bytes());
}
