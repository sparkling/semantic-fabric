use super::{
    parse_manifest, render_manifest, seal_for, verify_seal, Case, ExpectedStatus, Manifest,
    ManifestSeal, StandardReference, Surface,
};

fn case(id: &str, status: ExpectedStatus, cause: &str) -> Case {
    Case {
        id: id.to_owned(),
        scenario: id.to_owned(),
        expected_status: status,
        http_status: match status {
            ExpectedStatus::Supported => 200,
            ExpectedStatus::Unsupported => 501,
            ExpectedStatus::Rejected => 400,
        },
        response_media_type: if status == ExpectedStatus::Supported {
            "application/sparql-results+json".to_owned()
        } else {
            "application/problem+json".to_owned()
        },
        cause: cause.to_owned(),
        result_sha256: "0".repeat(64),
    }
}

fn manifest() -> Manifest {
    Manifest {
        profile_id: "test-query-surface-v1".to_owned(),
        surface: Surface::SparqlQuery,
        backend: "sqlite".to_owned(),
        standard: StandardReference::for_surface(Surface::SparqlQuery),
        cases: vec![
            case("query-ask", ExpectedStatus::Supported, "ask-boolean"),
            case(
                "query-service",
                ExpectedStatus::Unsupported,
                "unsupported-query",
            ),
        ],
    }
}

fn reseal(manifest: &Manifest) -> String {
    render_manifest(manifest)
}

#[test]
fn canonical_manifest_round_trips() {
    let expected = manifest();
    let text = render_manifest(&expected);

    assert_eq!(parse_manifest(&text), Ok(expected));
}

#[test]
fn fixed_seal_rejects_missing_and_extra_cases_even_after_internal_reseal() {
    let original = manifest();
    let original_text = reseal(&original);
    let seal = seal_for(&original_text, &original);
    let mut missing = original.clone();
    missing.cases.remove(0);
    let mut extra = original.clone();
    extra.cases.push(case(
        "query-third",
        ExpectedStatus::Supported,
        "select-bindings",
    ));
    let missing_text = reseal(&missing);
    let missing = parse_manifest(&missing_text).expect("internally resealed deletion is valid");
    let extra_text = reseal(&extra);
    let extra = parse_manifest(&extra_text).expect("internally resealed insertion is valid");

    assert_eq!(
        verify_seal(&missing_text, &missing, &seal),
        Err("supported-surface manifest does not match its fixed seal".to_owned())
    );
    assert_eq!(
        verify_seal(&extra_text, &extra, &seal),
        Err("supported-surface manifest does not match its fixed seal".to_owned())
    );
}

#[test]
fn parser_rejects_reordered_cases_with_a_recomputed_internal_digest() {
    let mut reordered = manifest();
    reordered.cases.reverse();

    assert_eq!(
        parse_manifest(&reseal(&reordered)),
        Err("case records are not strictly ordered".to_owned())
    );
}

#[test]
fn fixed_seal_rejects_count_neutral_status_and_cause_reclassification() {
    let original = manifest();
    let original_text = reseal(&original);
    let seal = seal_for(&original_text, &original);
    let mut mutated = original.clone();
    mutated.cases[0].expected_status = ExpectedStatus::Rejected;
    mutated.cases[0].http_status = 400;
    mutated.cases[0].response_media_type = "application/problem+json".to_owned();
    mutated.cases[0].cause = "invalid-request".to_owned();
    let mutated_text = reseal(&mutated);
    let mutated = parse_manifest(&mutated_text).expect("count-neutral reclassification is valid");

    assert_eq!(
        verify_seal(&mutated_text, &mutated, &seal),
        Err("supported-surface manifest does not match its fixed seal".to_owned())
    );
}

#[test]
fn parser_rejects_invalid_typed_outcome_combinations() {
    let mut invalid = manifest();
    invalid.cases[0].expected_status = ExpectedStatus::Rejected;

    assert_eq!(
        parse_manifest(&reseal(&invalid)),
        Err("case query-ask has an invalid status/code/media/cause combination".to_owned())
    );
}

#[test]
fn parser_rejects_unknown_metadata_and_noncanonical_text() {
    let text = render_manifest(&manifest());
    let unknown = text.replacen(
        "meta\tbackend\tsqlite\n",
        "meta\tbackend\tsqlite\nmeta\tunknown\tvalue\n",
        1,
    );
    let no_newline = text.trim_end().to_owned();

    assert!(parse_manifest(&unknown)
        .unwrap_err()
        .contains("unknown manifest metadata key"));
    assert_eq!(
        parse_manifest(&no_newline),
        Err("supported-surface manifest must end with a newline".to_owned())
    );
}

#[test]
fn seal_is_bound_to_profile_surface_count_and_complete_bytes() {
    let profile = manifest();
    let text = render_manifest(&profile);
    let seal = seal_for(&text, &profile);
    let wrong_surface = ManifestSeal {
        surface: Surface::SparqlProtocol,
        ..seal.clone()
    };

    assert_eq!(verify_seal(&text, &profile, &seal), Ok(()));
    assert_eq!(
        verify_seal(&text, &profile, &wrong_surface),
        Err("supported-surface manifest does not match its fixed seal".to_owned())
    );
}
