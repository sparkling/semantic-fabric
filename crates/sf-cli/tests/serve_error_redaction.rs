//! Black-box CLI receipts: startup source secrets never reach stdout or stderr.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SECRET: &str = "sf_secret_NEVER_EXPOSE_c913";
const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [ rr:predicate ex:value ; rr:objectMap [ rr:column "value" ] ] .
"#;

fn mapping_path() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sf_cli_redaction_{}_{unique}.ttl",
        std::process::id()
    ))
}

#[test]
fn invalid_pg_and_mysql_specs_are_redacted_on_every_cli_stream() {
    let path = mapping_path();
    fs::write(&path, MAPPING_TTL).expect("write mapping fixture");
    let specs = [
        format!("pg:host=localhost password='{SECRET}"),
        format!("mysql://user:{SECRET}@localhost:not-a-port/db"),
    ];

    for spec in specs {
        let output = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"))
            .args([
                "serve",
                "--source",
                &spec,
                "--mapping",
                path.to_str().expect("UTF-8 temp path"),
            ])
            .output()
            .expect("run semantic-fabric");
        assert!(!output.status.success(), "invalid source must fail");
        for (name, bytes) in [("stdout", output.stdout), ("stderr", output.stderr)] {
            assert!(
                !bytes
                    .windows(SECRET.len())
                    .any(|window| window == SECRET.as_bytes()),
                "secret escaped byte-for-byte on {name}: {}",
                String::from_utf8_lossy(&bytes)
            );
            assert!(bytes.len() < 512, "unbounded {name} error surface");
            if name == "stderr" {
                let text = String::from_utf8(bytes).expect("stderr UTF-8");
                assert!(text.contains("startup-source"), "stderr={text:?}");
                assert!(text.contains("correlation sf-"), "stderr={text:?}");
                assert!(!text.to_ascii_lowercase().contains("password"));
            }
        }
    }

    fs::remove_file(path).expect("remove mapping fixture");
}
