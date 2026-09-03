use std::env;

use sf_sql::introspect::introspect_postgres_public_observed_snapshot;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = env::var("SF_QUALIFICATION_DATABASE_URL")
        .map_err(|_| "SF_QUALIFICATION_DATABASE_URL is required")?;
    let (mut client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let version: String = client.query_one("SELECT version()", &[]).await?.get(0);
    let version_num: i32 = client
        .query_one("SELECT current_setting('server_version_num')::int4", &[])
        .await?
        .get(0);
    client
        .batch_execute(
            "SELECT set_config('search_path','pg_catalog,public,pg_temp',false); SET session_replication_role = origin;",
        )
        .await?;
    let snapshot = introspect_postgres_public_observed_snapshot(&mut client).await?;
    let (legacy, availability) = snapshot.into_parts();
    let identity = availability.identity();
    let json = format!(
        "{{\"serverVersion\":\"{}\",\"serverVersionNum\":{},\"guard\":\"pass\",\"legacyTableCount\":{},\"available\":{},\"structural\":{},\"types\":{},\"constraints\":{}}}",
        escape(&version), version_num, legacy.len(), identity.is_some(),
        identity.map_or_else(|| "null".into(), |v| format!("\"{:?}\"", v.structural())),
        identity.map_or_else(|| "null".into(), |v| format!("\"{:?}\"", v.types())),
        identity.map_or_else(|| "null".into(), |v| format!("\"{:?}\"", v.constraints())),
    );
    println!("{json}");
    Ok(())
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
