//! The real authenticated CLI retains source-local processor bases across reload.
use super::*;

const TOKEN: &str = "test-only-processor-base-token-0123456789";

fn json_body(response: &str) -> serde_json::Value {
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    if !headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return serde_json::from_str(body).unwrap();
    }
    let mut decoded = Vec::new();
    let mut rest = body.as_bytes();
    loop {
        let end = rest.windows(2).position(|part| part == b"\r\n").unwrap();
        let size = usize::from_str_radix(std::str::from_utf8(&rest[..end]).unwrap(), 16).unwrap();
        rest = &rest[end + 2..];
        if size == 0 {
            assert_eq!(rest, b"\r\n");
            break;
        }
        decoded.extend_from_slice(&rest[..size]);
        assert_eq!(&rest[size..size + 2], b"\r\n");
        rest = &rest[size + 2..];
    }
    serde_json::from_slice(&decoded).unwrap()
}

fn iri_mapping(predicate: &str, column: &str) -> String {
    format!(
        "@base <http://document.example/ignored-for-output/> .\n{}",
        mapping(predicate).replace(
            "rr:column \"value\"",
            &format!("rr:column \"{column}\"; rr:termType rr:IRI")
        )
    )
}

fn wait_values(address: SocketAddr, server: &mut Server, expected: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last = None;
    loop {
        if let Some(response) = request_with_token(address, Some(TOKEN)) {
            if response.starts_with("HTTP/1.1 200") {
                let json = json_body(&response);
                let mut values: Vec<_> = json["results"]["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| {
                        assert_eq!(row["value"]["type"], "uri");
                        row["value"]["value"].as_str().unwrap()
                    })
                    .collect();
                values.sort();
                if values == expected {
                    return;
                }
            }
            last = Some(response);
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "processor-base server exited"
        );
        assert!(
            Instant::now() < deadline,
            "processor-base query failed to converge: {last:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn cli_uses_independent_output_bases_and_retains_them_after_reload() {
    let mut fixture = Fixture::new();
    let first_db = fixture.path("first.db");
    let second_db = fixture.path("second.db");
    let first_map = fixture.path("first.ttl");
    let second_map = fixture.path("second.ttl");
    let ontology = fixture.path("ontology.ttl");
    let config = fixture.path("serve.toml");
    for (db, base) in [
        (&first_db, "http://data.example/left/"),
        (&second_db, "http://data.example/right/"),
    ] {
        database(db);
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute("UPDATE items SET value='../old'", []).unwrap();
        conn.execute("INSERT INTO items VALUES(?)", [format!("{base}../old")])
            .unwrap();
    }
    for (map, predicate) in [(&first_map, "left"), (&second_map, "right")] {
        std::fs::write(
            map,
            iri_mapping(&format!("http://example.test/{predicate}"), "value"),
        )
        .unwrap();
    }
    std::fs::write(&ontology, "<http://example.test/left> a <http://www.w3.org/2002/07/owl#ObjectProperty> .\n<http://example.test/right> a <http://www.w3.org/2002/07/owl#ObjectProperty> .").unwrap();
    let value = toml::toml! {
        [source]
        source = (format!("sqlite:{}", first_db.display()))
        source_2 = (format!("sqlite:{}", second_db.display()))
        [mappings]
        mapping = (first_map.to_str().unwrap())
        mapping_2 = (second_map.to_str().unwrap())
        mapping_base = "http://wrong.example/file/"
        mapping_base_2 = "http://wrong.example/file2/"
        [graphs]
        ontology = (ontology.to_str().unwrap())
    };
    std::fs::write(&config, toml::to_string(&value).unwrap()).unwrap();
    let address = available_address();
    let mut server = Server(
        Command::new(BINARY)
            .env_clear()
            .env("BASE_TEST_TOKEN", TOKEN)
            .env("SEMANTIC_FABRIC_MAPPING_BASE", "http://wrong.example/env/")
            .env(
                "SEMANTIC_FABRIC_MAPPING_BASE_2",
                "http://data.example/right/",
            )
            .args(["serve", "--config"])
            .arg(&config)
            .args([
                "--mapping-base",
                "http://data.example/left/",
                "--auth-token-env",
                "BASE_TEST_TOKEN",
                "--log-level",
                "error",
                "--reload-interval-secs",
                "1",
                "--bind",
                &address.to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    wait_values(
        address,
        &mut server,
        &[
            "http://data.example/left/../old",
            "http://data.example/right/../old",
        ],
    );
    assert!(request_with_token(address, None)
        .unwrap()
        .starts_with("HTTP/1.1 401"));
    for (db, map, predicate) in [
        (&first_db, &first_map, "left"),
        (&second_db, &second_map, "right"),
    ] {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute_batch(
            "ALTER TABLE items ADD COLUMN fresh TEXT; UPDATE items SET fresh='?new';",
        )
        .unwrap();
        std::fs::write(
            map,
            iri_mapping(&format!("http://example.test/{predicate}"), "fresh"),
        )
        .unwrap();
    }
    wait_values(
        address,
        &mut server,
        &[
            "http://data.example/left/?new",
            "http://data.example/right/?new",
        ],
    );
}

#[test]
fn invalid_processor_bases_fail_before_mapping_or_source_io_without_echo() {
    for second in [false, true] {
        let mut cmd = Command::new(BINARY);
        cmd.env_clear().args([
            "serve",
            "--source",
            "sqlite:/must-not-open.db",
            "--mapping",
            "/must-not-read.ttl",
            "--ontology",
            "/must-not-read-ontology.ttl",
        ]);
        if second {
            cmd.args([
                "--source-2",
                "sqlite:/also-must-not-open.db",
                "--mapping-2",
                "/also-must-not-read.ttl",
            ]);
        }
        cmd.args([
            if second {
                "--mapping-base-2"
            } else {
                "--mapping-base"
            },
            "SECRET invalid base",
        ]);
        let output = cmd.output().unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        let event: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
        assert_eq!(event["event"], "startup.failed");
        assert_eq!(event["failure"], "startup-configuration");
        assert!(!stderr.contains("SECRET"), "{stderr}");
        assert!(!stderr.contains("must-not"), "{stderr}");
    }
}
