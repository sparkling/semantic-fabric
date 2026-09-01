#[path = "product_mock_gold/support.rs"]
mod support;

use std::net::IpAddr;
use std::path::PathBuf;

use sf_core::Term;
use sf_sparql::{exec_pg, parse_and_translate_with, Tbox};
use sf_sql::introspect::introspect_postgres;
use sf_sql::{Column, Dialect, ForeignKey};
use tokio_postgres::config::Host;
use tokio_postgres::{Client, Config, IsolationLevel, NoTls};

const DIRECT_SQL: &str = "SELECT style_number, version FROM public.style \
    ORDER BY style_number ASC, version ASC LIMIT 10001";
const SPARQL: &str = r#"SELECT ?styleNumber ?version WHERE {
  ?style <https://hm.com/ns/semantic-product-mock/product-design/Style/field/StyleNumber> ?styleNumber ;
         <https://hm.com/ns/semantic-product-mock/product-design/Style/field/Version> ?version .
}
ORDER BY ?styleNumber ?version
LIMIT 10001"#;

fn validated_loopback_config(value: &str) -> Result<Config, &'static str> {
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
    if !literal_loopback || !hostaddr_loopback {
        return Err("PostgreSQL endpoint is not a literal loopback address");
    }
    if config.get_user() != Some("product_design") || config.get_dbname() != Some("product_design")
    {
        return Err("PostgreSQL connection identity mismatch");
    }
    Ok(config)
}

#[test]
fn live_endpoint_policy_accepts_only_literal_loopback_hosts() {
    assert!(validated_loopback_config(
        "host=127.0.0.1 port=25432 user=product_design dbname=product_design"
    )
    .is_ok());
    assert!(validated_loopback_config(
        "host=::1 port=25432 user=product_design dbname=product_design"
    )
    .is_ok());
    for refused in [
        "host=localhost user=product_design dbname=product_design",
        "host=192.0.2.1 user=product_design dbname=product_design",
        "host=127.0.0.1 hostaddr=192.0.2.1 user=product_design dbname=product_design",
        "host=127.0.0.1,127.0.0.1 user=product_design dbname=product_design",
        "host=127.0.0.1 user=postgres dbname=product_design",
        "host=127.0.0.1 user=product_design dbname=postgres",
    ] {
        assert!(validated_loopback_config(refused).is_err());
    }
}

fn required_path(name: &str) -> Result<PathBuf, &'static str> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or("required product-mock path variable is missing")
}

async fn run_live() -> Result<(), &'static str> {
    let gold_root = required_path(support::GOLD_ROOT_ENV)?;
    let source_root = required_path(support::SOURCE_ROOT_ENV)?;
    let url = std::env::var(support::PG_URL_ENV)
        .map_err(|_| "SF_PRODUCT_MOCK_PG_URL is required and must be UTF-8")?;
    let gold = support::load_external(&gold_root, &source_root)?;
    let config = validated_loopback_config(&url)?;
    let (mut client, connection) = config
        .connect(NoTls)
        .await
        .map_err(|_| "PostgreSQL connection failed")?;
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    let result = run_transaction(&mut client, &gold).await;
    drop(client);
    driver.abort();
    result
}

async fn run_transaction(
    client: &mut Client,
    gold: &support::GoldVertical,
) -> Result<(), &'static str> {
    let transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(|_| "read-only snapshot could not be started")?;
    let result = run_snapshot(transaction.client(), gold).await;
    let rollback = transaction
        .rollback()
        .await
        .map_err(|_| "read-only snapshot rollback failed");
    rollback?;
    result
}

async fn run_snapshot(client: &Client, gold: &support::GoldVertical) -> Result<(), &'static str> {
    assert_posture(client).await?;
    let catalog_style = read_style_schema(client).await?;
    if catalog_style != gold.style {
        return Err("live Style catalog contract mismatch");
    }
    let production_schema = introspect_postgres(client, "style")
        .await
        .map_err(|_| "production Style introspection failed")?;
    assert_production_schema(&production_schema, &gold.style)?;

    let count: i64 = client
        .query_one("SELECT count(*)::bigint FROM public.style", &[])
        .await
        .map_err(|_| "Style row count query failed")?
        .get(0);
    if !(1..=10_000).contains(&count) {
        return Err("Style row count is outside the admitted bound");
    }
    let direct: Vec<(String, i32)> = client
        .query(DIRECT_SQL, &[])
        .await
        .map_err(|_| "direct Style query failed")?
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    if direct.len() != count as usize {
        return Err("direct Style result count mismatch");
    }

    let maps =
        sf_mapping::parse_r2rml(&gold.r2rml).map_err(|_| "sealed Style R2RML did not parse")?;
    let plan = parse_and_translate_with(
        SPARQL,
        &maps,
        Dialect::Postgres,
        &Tbox::default(),
        &[production_schema],
    )
    .map_err(|_| "production Style translation failed")?;
    let solutions = exec_pg::select_pg(&plan, client)
        .await
        .map_err(|_| "production Style execution failed")?;
    if solutions.vars != ["styleNumber", "version"] || solutions.rows.len() != direct.len() {
        return Err("production Style result shape mismatch");
    }
    for (row, (style_number, version)) in solutions.rows.iter().zip(&direct) {
        let [style_term, version_term] = row.as_slice() else {
            return Err("production Style row width mismatch");
        };
        if literal(style_term, "http://www.w3.org/2001/XMLSchema#string")? != style_number
            || literal(version_term, "http://www.w3.org/2001/XMLSchema#integer")?
                != version.to_string()
        {
            return Err("production and direct Style rows differ");
        }
    }
    Ok(())
}

fn literal<'a>(term: &'a Option<Term>, datatype: &str) -> Result<&'a str, &'static str> {
    match term {
        Some(Term::Literal(value)) if value.datatype().as_str() == datatype => Ok(value.value()),
        _ => Err("production Style result term mismatch"),
    }
}

async fn assert_posture(client: &Client) -> Result<(), &'static str> {
    let row = client
        .query_one(
            "SELECT current_setting('server_version_num'), current_database(), current_user, \
             current_schema(), current_setting('transaction_isolation'), \
             current_setting('transaction_read_only')",
            &[],
        )
        .await
        .map_err(|_| "PostgreSQL posture query failed")?;
    let actual: [&str; 6] = [
        row.get(0),
        row.get(1),
        row.get(2),
        row.get(3),
        row.get(4),
        row.get(5),
    ];
    if actual
        != [
            "160009",
            "product_design",
            "product_design",
            "public",
            "repeatable read",
            "on",
        ]
    {
        return Err("PostgreSQL posture mismatch");
    }
    Ok(())
}

async fn read_style_schema(client: &Client) -> Result<support::StyleSchema, &'static str> {
    let columns = client
        .query(
            "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'style' ORDER BY ordinal_position",
            &[],
        )
        .await
        .map_err(|_| "Style column catalog query failed")?
        .into_iter()
        .map(|row| {
            let nullable: String = row.get(2);
            let nullable = match nullable.as_str() {
                "YES" => Ok(true),
                "NO" => Ok(false),
                _ => Err("Style nullability catalog value is invalid"),
            }?;
            Ok(support::StyleColumn {
                name: row.get(0),
                store_type: row.get(1),
                nullable,
            })
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    let primary_key = client
        .query(
            "SELECT attribute.attname FROM pg_catalog.pg_constraint AS con \
             JOIN pg_catalog.pg_class AS relation ON relation.oid = con.conrelid \
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
             JOIN LATERAL generate_subscripts(con.conkey, 1) AS key_ordinal(key_index) ON true \
             JOIN pg_catalog.pg_attribute AS attribute ON attribute.attrelid = relation.oid \
               AND attribute.attnum = con.conkey[key_ordinal.key_index] \
             WHERE namespace.nspname = 'public' AND relation.relname = 'style' \
               AND con.contype = 'p' ORDER BY key_ordinal.key_index",
            &[],
        )
        .await
        .map_err(|_| "Style primary-key catalog query failed")?
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    let foreign_keys = client
        .query(
            "SELECT con.conname, child_attribute.attname, parent.relname, \
                    parent_attribute.attname \
             FROM pg_catalog.pg_constraint AS con \
             JOIN pg_catalog.pg_class AS child ON child.oid = con.conrelid \
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = child.relnamespace \
             JOIN pg_catalog.pg_class AS parent ON parent.oid = con.confrelid \
             JOIN LATERAL generate_subscripts(con.conkey, 1) AS key_ordinal(key_index) ON true \
             JOIN pg_catalog.pg_attribute AS child_attribute ON child_attribute.attrelid = child.oid \
               AND child_attribute.attnum = con.conkey[key_ordinal.key_index] \
             JOIN pg_catalog.pg_attribute AS parent_attribute ON parent_attribute.attrelid = parent.oid \
               AND parent_attribute.attnum = con.confkey[key_ordinal.key_index] \
             WHERE namespace.nspname = 'public' AND child.relname = 'style' \
               AND con.contype = 'f' ORDER BY con.conname, key_ordinal.key_index",
            &[],
        )
        .await
        .map_err(|_| "Style foreign-key catalog query failed")?
        .into_iter()
        .map(|row| support::StyleForeignKey {
            name: row.get(0),
            child_columns: vec![row.get(1)],
            parent_table: row.get(2),
            parent_columns: vec![row.get(3)],
        })
        .collect();
    Ok(support::StyleSchema {
        columns,
        primary_key,
        foreign_keys,
    })
}

fn assert_production_schema(
    actual: &sf_sql::TableSchema,
    expected: &support::StyleSchema,
) -> Result<(), &'static str> {
    let columns: Vec<_> = actual
        .columns
        .iter()
        .map(|column| Column::new(&column.name, &column.sql_type, column.not_null))
        .collect();
    let expected_columns: Vec<_> = expected
        .columns
        .iter()
        .map(|column| Column::new(&column.name, &column.store_type, !column.nullable))
        .collect();
    let foreign_keys: Vec<_> = expected
        .foreign_keys
        .iter()
        .map(|key| ForeignKey {
            columns: key.child_columns.clone(),
            parent_table: key.parent_table.clone(),
            parent_columns: key.parent_columns.clone(),
        })
        .collect();
    if actual.name != "style"
        || columns != expected_columns
        || actual.primary_key != expected.primary_key
        || actual.foreign_keys != foreign_keys
    {
        return Err("production Style schema contract mismatch");
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires exact external gold/source roots and loopback PostgreSQL 16.9"]
async fn exact_live_postgres_style_differential_uses_one_read_only_snapshot() {
    run_live()
        .await
        .expect("sealed product-mock live differential must pass");
}
