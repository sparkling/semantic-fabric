use rusqlite::{params_from_iter, Connection};
use sf_core::ir::LogicalSource;
use sf_core::{Literal, NamedNode, Term};
use sf_core::{SourceId, SourceMapping};
use sf_sparql::{CompilerBinding, Epoch, Tbox};
use sf_sql::Dialect;

pub const MAPPING: &str = include_str!("../fixtures/query-successor/semantic-product-v1.r2rml.ttl");
pub const ENUMERATE: &str =
    include_str!("../fixtures/query-successor/c2-successor-enumerate.sparql");
pub const ROOT: &str = include_str!("../fixtures/query-successor/c2-successor-root.sparql");

pub const ROOT_VARS: [&str; 32] = [
    "subject",
    "class",
    "styleNumber",
    "season",
    "styleStatus",
    "styleVersion",
    "bomVersion",
    "poRef",
    "factoryRef",
    "orderQuantity",
    "incoterm",
    "exFactoryDate",
    "sampleRound",
    "sampleStatus",
    "fitSignedOff",
    "ppSignedOff",
    "costVersion",
    "costTotal",
    "runRef",
    "plannedQuantity",
    "runState",
    "shippedQuantity",
    "confirmedAt",
    "inspectionRef",
    "inspectionResult",
    "shipmentRef",
    "shipmentQuantity",
    "shipmentStatus",
    "plannedUnits",
    "plannedRetail",
    "accrualRef",
    "accrualReversed",
];

pub fn string(value: &str) -> Term {
    Literal::new_simple_literal(value).into()
}

pub fn typed(value: &str, datatype: &str) -> Term {
    Literal::new_typed_literal(
        value,
        NamedNode::new(format!("http://www.w3.org/2001/XMLSchema#{datatype}")).unwrap(),
    )
    .into()
}

fn iri(path: &str) -> Term {
    NamedNode::new(format!("https://hm.com/ns/semantic-product-mock/{path}"))
        .unwrap()
        .into()
}

fn expected_row(
    class: &str,
    key: &str,
    style: Option<&str>,
    fields: Vec<(usize, Term)>,
) -> Vec<Option<Term>> {
    let mut row = vec![None; ROOT_VARS.len()];
    row[0] = Some(iri(&format!("resource/{class}/{key}")));
    row[1] = Some(iri(class));
    row[2] = style.map(string);
    for (column, term) in fields {
        assert!(row[column].is_none());
        row[column] = Some(term);
    }
    row
}

/// Independent oracle from the pinned mapping and fixture input values, not
/// reconstructed from query output. Every omitted UNION position stays unbound.
pub fn expected_root(style: &str) -> Vec<Vec<Option<Term>>> {
    if style == "STYLE-099" {
        return vec![];
    }
    let po = format!("PO-{style}");
    let run = format!("RUN-{style}");
    let id = format!("Id-{style}");
    let inspection = format!("InspectionRef-{style}");
    let shipment = format!("ShipmentRef-{style}");
    let entry = format!("EntryRef-{style}");
    let s = Some(style);
    vec![
        expected_row("product-design/Style", style, s, vec![
            (3, iri("resource/product-design/SeasonReference/SS26")),
            (4, iri("resource/product-design/StyleStatus/Accepted")), (5, typed("1", "integer"))]),
        expected_row("materials-bom/BillOfMaterials", &id, s, vec![(6, typed("1", "integer"))]),
        expected_row("supplier-sourcing/PurchaseOrder", &po, s, vec![
            (7, string(&po)), (8, string("factory")), (9, typed("1", "decimal")),
            (10, string("FOB")), (11, typed("2026-01-01", "date"))]),
        expected_row("sampling-fit/SampleRequest", &id, s, vec![
            (7, string(&po)), (12, typed("1", "integer")),
            (13, iri("resource/sampling-fit/SampleStatus/Accepted")),
            (14, typed("true", "boolean")), (15, typed("true", "boolean"))]),
        expected_row("costing-pricing/CostSheet", &id, s, vec![
            (7, string(&po)), (16, typed("1", "integer")), (17, typed("1.5", "decimal"))]),
        expected_row("production-planning-manufacturing-execution/ProductionRun", &run, s, vec![
            (7, string(&po)), (18, string(&run)), (19, typed("1", "decimal")),
            (20, iri("resource/production-planning-manufacturing-execution/ProductionRunState/Accepted")),
            (21, typed("1", "decimal")), (22, typed("2026-01-01T00:00:00Z", "dateTime"))]),
        expected_row("quality-compliance/Inspection", &inspection, None, vec![
            (7, string(&po)), (18, string(&run)), (23, string(&inspection)),
            (24, iri("resource/quality-compliance/InspectionResult/Accepted"))]),
        expected_row("logistics-allocation/Shipment", &shipment, s, vec![
            (7, string(&po)), (25, string(&shipment)), (26, typed("1", "decimal")),
            (27, iri("resource/logistics-allocation/ShipmentStatus/Accepted"))]),
        expected_row("merchandising-assortment-planning/LinePlan", style, s, vec![
            (28, typed("1", "decimal")), (29, typed("1.5", "decimal"))]),
        expected_row("finance-cost-accounting/LandedCostAccrual", &entry, s, vec![
            (7, string(&po)), (30, string(&entry)), (31, typed("true", "boolean"))]),
    ]
}

/// Compiler-only, unverified observation. This fixture has no ontology store,
/// serving admission, external source identity, grant or independent owner.
pub fn fixture() -> (CompilerBinding, Connection) {
    let maps = sf_mapping::parse_r2rml(MAPPING).unwrap();
    assert_eq!(maps.len(), 10);
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("ATTACH DATABASE ':memory:' AS nordlys_semantic")
        .unwrap();
    for map in &maps {
        let LogicalSource::Query(sql) = &map.source else {
            panic!("exact query mapping")
        };
        let (projection, table) = sql
            .strip_prefix("SELECT ")
            .unwrap()
            .split_once(" FROM ")
            .unwrap();
        assert!(table.starts_with("nordlys_semantic."));
        assert!(table
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.".contains(&b)));
        let columns: Vec<_> = projection
            .split(',')
            .map(|s| s.trim().trim_matches('"'))
            .collect();
        assert!(columns
            .iter()
            .all(|c| c.bytes().all(|b| b.is_ascii_alphanumeric())));
        let definition = columns
            .iter()
            .map(|c| format!("\"{c}\" TEXT"))
            .collect::<Vec<_>>()
            .join(",");
        conn.execute_batch(&format!("CREATE TABLE {table}({definition})"))
            .unwrap();
        let placeholders = vec!["?"; columns.len()].join(",");
        let insert = format!("INSERT INTO {table} VALUES ({placeholders})");
        let count = if table.ends_with(".style_v1") { 70 } else { 2 };
        for index in 1..=count {
            let style = format!("STYLE-{index:03}");
            let values: Vec<_> = columns.iter().map(|column| value(column, &style)).collect();
            conn.execute(&insert, params_from_iter(values)).unwrap();
        }
        if count == 70 {
            let values: Vec<_> = columns
                .iter()
                .map(|column| value(column, "STYLE-070"))
                .collect();
            conn.execute(&insert, params_from_iter(values)).unwrap();
        }
    }
    let binding = CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps),
        Dialect::Sqlite,
        Tbox::default(),
        Vec::new(),
        Epoch::default(),
        64,
    );
    (binding, conn)
}

fn value(column: &str, style: &str) -> String {
    match column {
        "StyleNumber" | "StyleReference" | "StyleRef" => style.into(),
        "PoRef" | "LandedCostPoRef" => format!("PO-{style}"),
        "RunRef" | "ProductionRunRef" => format!("RUN-{style}"),
        "Version" | "AggregateVersion" | "Round" | "Quantity" | "PlannedQuantity"
        | "ShippedQuantity" | "PlannedUnits" => "1".into(),
        "Total" | "PlannedRetail" => "1.5".into(),
        "IsFitSignedOff" | "IsPpSignedOff" | "Reversed" => "true".into(),
        "ExFactoryDate" => "2026-01-01".into(),
        "ExFactoryConfirmedAt" => "2026-01-01T00:00:00Z".into(),
        "Season" => "SS26".into(),
        "Status" | "State" | "Result" => "Accepted".into(),
        "Incoterm" => "FOB".into(),
        "FactoryRef" => "factory".into(),
        "Id" | "InspectionRef" | "ShipmentRef" | "EntryRef" => format!("{column}-{style}"),
        unknown => panic!("unexpected pinned fixture column {unknown}"),
    }
}

pub fn render(template: &str, placeholder: &str, value: &str, empty_allowed: bool) -> String {
    assert!(value.len() <= 64 && (empty_allowed || !value.is_empty()));
    assert!(value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)));
    let query = template.replace(placeholder, value);
    assert!(!query.contains("{{"));
    query
}
