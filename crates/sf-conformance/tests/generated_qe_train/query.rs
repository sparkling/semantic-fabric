use super::{Fixture, CASES_PER_FIXTURE, REPLAY_ENV, SUITE_SEED};

pub(super) struct GeneratedCase {
    pub(super) ordinal: usize,
    pub(super) shape: usize,
    pub(super) query: String,
    pub(super) projected: Vec<&'static str>,
    pub(super) ordered: bool,
}

impl GeneratedCase {
    pub(super) fn receipt(&self, fixture: &Fixture) -> String {
        format!(
            "replay={REPLAY_ENV}={} seed={SUITE_SEED:#018x} fixture={} \
             data={:#018x} shape={} variant={} query={}",
            self.ordinal,
            fixture.id,
            fixture.data_key,
            self.shape,
            self.ordinal % CASES_PER_FIXTURE / 10,
            self.query.split_whitespace().collect::<Vec<_>>().join(" ")
        )
    }
}

fn spec<const N: usize>(
    query: impl Into<String>,
    projected: [&'static str; N],
    ordered: bool,
) -> (String, Vec<&'static str>, bool) {
    (query.into(), projected.into(), ordered)
}

fn item_iri(fixture_id: usize, row: usize) -> String {
    format!("<http://example.com/generated/f{fixture_id}/item/{row}>")
}

pub(super) fn generated_case(fixture: &Fixture, local: usize) -> GeneratedCase {
    let ordinal = fixture.id * CASES_PER_FIXTURE + local;
    let shape = local % 10;
    let variant = local / 10;
    let row = variant % 5;
    let subject = item_iri(fixture.id, row + 1);
    let reverse = variant >= 5;
    let (query, projected, ordered) = match shape {
        0 if reverse => spec(
            format!(
                "SELECT ?value ?s WHERE {{ VALUES ?s {{ {subject} }} ?s qe:label ?value }}"
            ),
            ["value", "s"],
            false,
        ),
        0 => spec(
            format!(
                "SELECT ?s ?value WHERE {{ VALUES ?s {{ {subject} }} ?s qe:label ?value }}"
            ),
            ["s", "value"],
            false,
        ),
        1 => {
            let body = match variant {
                0 => "?s qe:bucket ?value".to_owned(),
                1 => "?s qe:bucket ?value FILTER (?value = \"red\")".to_owned(),
                2 => "?s qe:bucket ?value FILTER (?value = \"blue\")".to_owned(),
                3 => "?s qe:bucket ?value FILTER (?value = \"green\")".to_owned(),
                4 => "?s qe:bucket ?value FILTER (?value != \"orange\")".to_owned(),
                _ => format!("VALUES ?s {{ {subject} }} ?s qe:bucket ?value"),
            };
            spec(format!("SELECT ?value WHERE {{ {body} }}"), ["value"], false)
        }
        2 if reverse => spec(
            format!(
                "SELECT ?bucket ?label ?s WHERE {{ VALUES ?s {{ {subject} }} ?s qe:label ?label ; qe:bucket ?bucket }}"
            ),
            ["bucket", "label", "s"],
            false,
        ),
        2 => spec(
            format!(
                "SELECT ?s ?label ?bucket WHERE {{ VALUES ?s {{ {subject} }} ?s qe:label ?label ; qe:bucket ?bucket }}"
            ),
            ["s", "label", "bucket"],
            false,
        ),
        3 if reverse => spec(
            format!(
                "SELECT ?note ?s ?label WHERE {{ VALUES ?s {{ {subject} }} ?s qe:label ?label OPTIONAL {{ ?s qe:note ?note }} }}"
            ),
            ["note", "s", "label"],
            false,
        ),
        3 => spec(
            format!(
                "SELECT ?s ?label ?note WHERE {{ VALUES ?s {{ {subject} }} ?s qe:label ?label OPTIONAL {{ ?s qe:note ?note }} }}"
            ),
            ["s", "label", "note"],
            false,
        ),
        4 if reverse => spec(
            format!(
                "SELECT ?peer ?s WHERE {{ VALUES ?s {{ {subject} }} ?s qe:kind qe:Entity OPTIONAL {{ ?s qe:peer ?peer }} }}"
            ),
            ["peer", "s"],
            false,
        ),
        4 => spec(
            format!(
                "SELECT ?s ?peer WHERE {{ VALUES ?s {{ {subject} }} ?s qe:kind qe:Entity OPTIONAL {{ ?s qe:peer ?peer }} }}"
            ),
            ["s", "peer"],
            false,
        ),
        5 => {
            let threshold = fixture.data_key % 7 + variant as u64 * 4;
            spec(
                format!(
                    "SELECT ?s ?score WHERE {{ ?s qe:score ?score FILTER (?score >= {threshold}) }}"
                ),
                ["s", "score"],
                false,
            )
        }
        6 => {
            let bucket_index = (row + usize::from(reverse)) % fixture.buckets.len();
            let bucket = fixture.buckets[bucket_index];
            spec(
                format!(
                    "SELECT ?s WHERE {{ VALUES ?s {{ {subject} }} ?s qe:bucket \"{bucket}\" }}"
                ),
                ["s"],
                false,
            )
        }
        7 if reverse => spec(
            format!(
                "SELECT ?name ?group ?s WHERE {{ VALUES ?s {{ {subject} }} ?s qe:group ?group . ?group qe:groupName ?name }}"
            ),
            ["name", "group", "s"],
            false,
        ),
        7 => spec(
            format!(
                "SELECT ?s ?group ?name WHERE {{ VALUES ?s {{ {subject} }} ?s qe:group ?group . ?group qe:groupName ?name }}"
            ),
            ["s", "group", "name"],
            false,
        ),
        8 if reverse => spec(
            format!(
                "SELECT ?value ?s WHERE {{ VALUES ?s {{ {subject} }} {{ ?s qe:label ?value }} UNION {{ ?s qe:bucket ?value }} }}"
            ),
            ["value", "s"],
            false,
        ),
        8 => spec(
            format!(
                "SELECT ?s ?value WHERE {{ VALUES ?s {{ {subject} }} {{ ?s qe:label ?value }} UNION {{ ?s qe:bucket ?value }} }}"
            ),
            ["s", "value"],
            false,
        ),
        9 => {
            let limit = 1 + variant % 4;
            let offset = variant / 2;
            let direction = if reverse { "DESC" } else { "ASC" };
            spec(
                format!(
                    "SELECT ?label ?s WHERE {{ ?s qe:label ?label }} \
                     ORDER BY {direction}(?label) ASC(?s) LIMIT {limit} OFFSET {offset}"
                ),
                ["label", "s"],
                true,
            )
        }
        _ => unreachable!("shape is modulo ten"),
    };
    GeneratedCase {
        ordinal,
        shape,
        query: format!("PREFIX qe: <http://example.com/qe/> {query}"),
        projected,
        ordered,
    }
}
