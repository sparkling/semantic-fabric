#[path = "product_mock_gold/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;

use serde_json::Value;
use tokio_postgres::config::Host;
use tokio_postgres::{Client, Config, IsolationLevel, NoTls};

const LIVE_DATABASES: [(&str, &str); 11] = [
    ("CostingPricing", "costing_pricing"),
    ("FinanceCostAccounting", "finance_cost_accounting"),
    ("LogisticsAllocation", "logistics_allocation"),
    ("MaterialsBom", "materials_bom"),
    ("MerchandisingAssortment", "merchandising_assortment"),
    ("ProductDesign", "product_design"),
    ("ProductionManufacturing", "production_manufacturing"),
    ("QualityCompliance", "quality_compliance"),
    ("SamplingFit", "sampling_fit"),
    ("Style360", "style360"),
    ("SupplierSourcing", "supplier_sourcing"),
];

fn database_bindings(
    inventory: &support::RelationalInventory,
) -> Result<Vec<(&'static str, &'static str)>, &'static str> {
    if inventory.stores.len() != LIVE_DATABASES.len()
        || LIVE_DATABASES
            .iter()
            .any(|(store, _)| inventory.store(store).is_none())
        || inventory.stores.iter().any(|store| {
            !LIVE_DATABASES
                .iter()
                .any(|(expected, _)| *expected == store.name)
        })
    {
        return Err("sealed stores do not match explicit live database bindings");
    }
    Ok(LIVE_DATABASES.to_vec())
}

fn validated_base_config(value: &str) -> Result<Config, &'static str> {
    let config: Config = value
        .parse()
        .map_err(|_| "PostgreSQL connection configuration is invalid")?;
    let literal_loopback = match config.get_hosts() {
        [Host::Tcp(host)] => host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        _ => false,
    };
    let hostaddr_loopback = match config.get_hostaddrs() {
        [] => true,
        [address] => address.is_loopback(),
        _ => false,
    };
    if !literal_loopback || !hostaddr_loopback || config.get_ports() != [25432] {
        return Err("PostgreSQL endpoint is not the admitted loopback endpoint");
    }
    if config.get_user() != Some("product_design") || config.get_dbname() != Some("product_design")
    {
        return Err("PostgreSQL connection identity mismatch");
    }
    Ok(config)
}

fn required_path(name: &str) -> Result<PathBuf, &'static str> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or("required product-mock path variable is missing")
}

async fn run_live() -> Result<(), String> {
    let gold_root = required_path(support::GOLD_ROOT_ENV).map_err(str::to_owned)?;
    let source_root = required_path(support::SOURCE_ROOT_ENV).map_err(str::to_owned)?;
    let url = std::env::var(support::PG_URL_ENV)
        .map_err(|_| "SF_PRODUCT_MOCK_PG_URL is required and must be UTF-8".to_owned())?;
    let gold = support::load_external(&gold_root, &source_root).map_err(str::to_owned)?;
    let bindings = database_bindings(&gold.inventory).map_err(str::to_owned)?;
    let base = validated_base_config(&url).map_err(str::to_owned)?;

    let mut failures = Vec::new();
    for (store_name, database) in bindings {
        let expected = gold
            .inventory
            .store(store_name)
            .ok_or_else(|| format!("sealed store {store_name} disappeared"))?;
        match inspect_database(&base, store_name, database).await {
            Ok(actual) => {
                if let Err(difference) = compare_store(expected, &actual) {
                    failures.push(difference);
                }
            }
            Err(error) => failures.push(format!("{store_name}/{database}: {error}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "live product-mock schema differs from sealed 112-table/598-column inventory:\n{}",
            failures.join("\n")
        ))
    }
}

async fn inspect_database(
    base: &Config,
    store_name: &str,
    database: &str,
) -> Result<support::StoreSchema, String> {
    let mut config = base.clone();
    config.dbname(database);
    let (mut client, connection) = config
        .connect(NoTls)
        .await
        .map_err(|_| "PostgreSQL connection failed".to_owned())?;
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    let result = inspect_transaction(&mut client, store_name, database).await;
    drop(client);
    driver.abort();
    result
}

async fn inspect_transaction(
    client: &mut Client,
    store_name: &str,
    database: &str,
) -> Result<support::StoreSchema, String> {
    let transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(|_| "read-only snapshot could not be started".to_owned())?;
    let result = async {
        assert_posture(transaction.client(), database).await?;
        read_store(transaction.client(), store_name).await
    }
    .await;
    transaction
        .rollback()
        .await
        .map_err(|_| "read-only snapshot rollback failed".to_owned())?;
    result
}

async fn assert_posture(client: &Client, database: &str) -> Result<(), String> {
    let row = client
        .query_one(
            "SELECT current_setting('server_version_num'), current_database(), current_user, \
             current_schema(), current_setting('transaction_isolation'), \
             current_setting('transaction_read_only')",
            &[],
        )
        .await
        .map_err(|_| "PostgreSQL posture query failed".to_owned())?;
    let actual: [&str; 6] = [
        row.get(0),
        row.get(1),
        row.get(2),
        row.get(3),
        row.get(4),
        row.get(5),
    ];
    let expected = [
        "160009",
        database,
        "product_design",
        "public",
        "repeatable read",
        "on",
    ];
    if actual != expected {
        return Err("live product-mock PostgreSQL 16.9 posture mismatch".to_owned());
    }
    Ok(())
}

async fn read_store(client: &Client, store_name: &str) -> Result<support::StoreSchema, String> {
    let table_rows = client
        .query(
            "SELECT relation.relname FROM pg_catalog.pg_class AS relation \
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
             WHERE namespace.nspname = 'public' AND relation.relkind IN ('r', 'p') \
             ORDER BY relation.relname COLLATE \"C\"",
            &[],
        )
        .await
        .map_err(|_| "table catalog query failed".to_owned())?;
    let mut relations: BTreeMap<String, Vec<support::RelationalColumn>> = table_rows
        .into_iter()
        .map(|row| (row.get(0), Vec::new()))
        .collect();
    let column_rows = client
        .query(
            "SELECT relation.relname, attribute.attname, \
                    pg_catalog.format_type(attribute.atttypid, attribute.atttypmod), \
                    NOT attribute.attnotnull \
             FROM pg_catalog.pg_class AS relation \
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
             JOIN pg_catalog.pg_attribute AS attribute ON attribute.attrelid = relation.oid \
             WHERE namespace.nspname = 'public' AND relation.relkind IN ('r', 'p') \
               AND attribute.attnum > 0 AND NOT attribute.attisdropped \
             ORDER BY relation.relname COLLATE \"C\", attribute.attnum",
            &[],
        )
        .await
        .map_err(|_| "column catalog query failed".to_owned())?;
    for row in column_rows {
        let table: String = row.get(0);
        let columns = relations
            .get_mut(&table)
            .ok_or_else(|| "column catalog returned an unknown table".to_owned())?;
        columns.push(support::RelationalColumn {
            name: row.get(1),
            store_type: row.get(2),
            nullable: row.get(3),
        });
    }
    Ok(support::StoreSchema {
        name: store_name.to_owned(),
        relations: relations
            .into_iter()
            .map(|(name, columns)| support::RelationSchema { name, columns })
            .collect(),
    })
}

fn compare_store(
    expected: &support::StoreSchema,
    actual: &support::StoreSchema,
) -> Result<(), String> {
    if expected.name != actual.name {
        return Err(format!("{}: live store identity mismatch", expected.name));
    }
    let expected_tables: BTreeMap<_, _> = expected
        .relations
        .iter()
        .map(|relation| (relation.name.as_str(), relation))
        .collect();
    let actual_tables: BTreeMap<_, _> = actual
        .relations
        .iter()
        .map(|relation| (relation.name.as_str(), relation))
        .collect();
    let missing: Vec<_> = expected_tables
        .iter()
        .filter(|(name, _)| !actual_tables.contains_key(**name))
        .collect();
    let extra: Vec<_> = actual_tables
        .iter()
        .filter(|(name, _)| !expected_tables.contains_key(**name))
        .collect();
    let mut changed_tables = 0;
    let mut changed_columns = 0;
    for (name, expected_relation) in &expected_tables {
        if let Some(actual_relation) = actual_tables.get(*name) {
            let changed = compare_relation(expected_relation, actual_relation);
            if changed > 0 {
                changed_tables += 1;
                changed_columns += changed;
            }
        }
    }
    if missing.is_empty() && extra.is_empty() && changed_tables == 0 {
        Ok(())
    } else {
        let missing_columns: usize = missing
            .iter()
            .map(|(_, relation)| relation.columns.len())
            .sum();
        let unexpected_columns: usize = extra
            .iter()
            .map(|(_, relation)| relation.columns.len())
            .sum();
        Err(format!(
            "{}: missing tables={}, missing columns={}, unexpected tables={}, \
             unexpected columns={}, changed tables={}, changed columns={}",
            expected.name,
            missing.len(),
            missing_columns,
            extra.len(),
            unexpected_columns,
            changed_tables,
            changed_columns
        ))
    }
}

fn compare_relation(expected: &support::RelationSchema, actual: &support::RelationSchema) -> usize {
    let mut changed = expected.columns.len().abs_diff(actual.columns.len());
    for (expected_column, actual_column) in expected.columns.iter().zip(&actual.columns) {
        let expected_type = normalized_type(&expected_column.store_type);
        let actual_type = normalized_type(&actual_column.store_type);
        if expected_column.name != actual_column.name
            || expected_type != actual_type
            || expected_column.nullable != actual_column.nullable
        {
            changed += 1;
        }
    }
    changed
}

fn normalized_type(value: &str) -> String {
    if let Some(length) = value
        .strip_prefix("varchar(")
        .and_then(|value| value.strip_suffix(')'))
    {
        format!("character varying({length})")
    } else if value == "timestamptz" {
        "timestamp with time zone".to_owned()
    } else {
        value.to_owned()
    }
}

#[test]
fn sealed_stores_have_explicit_database_bindings() {
    let gold = support::admit_synthetic(&support::SyntheticFixture::valid())
        .expect("synthetic inventory is valid");
    assert_eq!(gold.inventory.table_count(), 112);
    assert_eq!(gold.inventory.column_count(), 598);
    let bindings = database_bindings(&gold.inventory).expect("all stores are explicitly bound");
    assert_eq!(bindings.len(), 11);
    assert_eq!(bindings[5], ("ProductDesign", "product_design"));
}

#[test]
fn resealed_inventory_cannot_omit_a_claimed_table() {
    let mut fixture = support::SyntheticFixture::valid();
    let mut coverage: Value = serde_json::from_slice(
        fixture
            .artifacts
            .get(support::COVERAGE_PATH)
            .expect("coverage exists"),
    )
    .expect("synthetic coverage parses");
    coverage["relationalSchema"]["stores"][0]["relations"]
        .as_array_mut()
        .expect("synthetic relations array")
        .pop();
    fixture.artifacts.insert(
        support::COVERAGE_PATH.to_owned(),
        serde_json::to_vec(&coverage).expect("synthetic coverage serializes"),
    );
    fixture.reseal_artifact(support::COVERAGE_PATH);

    assert_eq!(
        support::admit_synthetic(&fixture),
        Err("relational inventory table count mismatch")
    );
}

#[test]
fn live_endpoint_policy_is_exact_and_loopback_only() {
    assert!(validated_base_config(
        "host=127.0.0.1 port=25432 user=product_design dbname=product_design"
    )
    .is_ok());
    for refused in [
        "host=localhost port=25432 user=product_design dbname=product_design",
        "host=127.0.0.1 port=5432 user=product_design dbname=product_design",
        "host=127.0.0.1 port=25432 user=postgres dbname=product_design",
        "host=127.0.0.1 port=25432 user=product_design dbname=postgres",
    ] {
        assert!(validated_base_config(refused).is_err());
    }
}

#[test]
fn comparison_reports_unclassified_tables_instead_of_filtering_them() {
    let expected = store_with_relations(&[relation("style", "varchar(64)")]);
    let actual = store_with_relations(&[
        relation("style", "character varying(64)"),
        relation("unclassified_table", "text"),
    ]);
    assert_eq!(
        compare_store(&expected, &actual),
        Err(
            "ProductDesign: missing tables=0, missing columns=0, unexpected tables=1, \
             unexpected columns=1, changed tables=0, changed columns=0"
                .to_owned()
        )
    );
}

#[test]
fn comparison_reports_column_contract_drift() {
    let expected = store_with_relations(&[relation("style", "text")]);
    let mut actual = store_with_relations(&[relation("style", "text")]);
    actual.relations[0].columns[0].nullable = true;
    let error = compare_store(&expected, &actual).expect_err("nullability drift must fail");
    assert_eq!(
        error,
        "ProductDesign: missing tables=0, missing columns=0, unexpected tables=0, \
         unexpected columns=0, changed tables=1, changed columns=1"
    );
}

#[test]
fn empty_style360_reports_three_tables_and_twenty_one_columns_missing() {
    let expected = support::StoreSchema {
        name: "Style360".to_owned(),
        relations: (0..3)
            .map(|table| support::RelationSchema {
                name: format!("table_{table}"),
                columns: (0..7)
                    .map(|column| support::RelationalColumn {
                        name: format!("column_{column}"),
                        store_type: "text".to_owned(),
                        nullable: false,
                    })
                    .collect(),
            })
            .collect(),
    };
    let actual = support::StoreSchema {
        name: "Style360".to_owned(),
        relations: Vec::new(),
    };
    assert_eq!(
        compare_store(&expected, &actual),
        Err(
            "Style360: missing tables=3, missing columns=21, unexpected tables=0, \
             unexpected columns=0, changed tables=0, changed columns=0"
                .to_owned()
        )
    );
}

fn store_with_relations(relations: &[support::RelationSchema]) -> support::StoreSchema {
    support::StoreSchema {
        name: "ProductDesign".to_owned(),
        relations: relations.to_vec(),
    }
}

fn relation(name: &str, store_type: &str) -> support::RelationSchema {
    support::RelationSchema {
        name: name.to_owned(),
        columns: vec![support::RelationalColumn {
            name: "id".to_owned(),
            store_type: store_type.to_owned(),
            nullable: false,
        }],
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires sealed external roots and all 11 loopback product-mock PostgreSQL 16.9 databases"]
async fn exact_live_postgres_schema_inventory_uses_one_read_only_snapshot_per_database() {
    run_live()
        .await
        .expect("sealed product-mock schema differential must pass");
}
