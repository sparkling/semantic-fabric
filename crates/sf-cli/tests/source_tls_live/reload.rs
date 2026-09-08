//! Required live proof that native reload retains authenticated TLS source routing.
use super::*;
#[path = "reload_lineage.rs"]
mod proof;

pub(super) fn assert_reloads(
    fixture: &Fixture,
    first: &Database,
    second: Option<&Database>,
    expected: &[&str],
) {
    let (mut command, address) = command(fixture, first, second);
    let mut server = Server(
        command
            .args(["--reload-interval-secs", "1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let query = if second.is_some() { UNION } else { SINGLE };
    let poll = |server: &mut Server, expected_status: u16, expected: &[&str]| {
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            if let Some((status, body)) = request(address, query, Some(&fixture.token)) {
                if status == expected_status {
                    if status != 200 {
                        break;
                    }
                    let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let bindings = document["results"]["bindings"].as_array().unwrap();
                    let mut values = bindings
                        .iter()
                        .map(|b| b["value"]["value"].as_str().unwrap())
                        .collect::<Vec<_>>();
                    values.sort_unstable();
                    let mut expected = expected.to_vec();
                    expected.sort_unstable();
                    if values == expected {
                        break;
                    }
                }
            }
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "TLS reload child exited"
            );
            assert!(Instant::now() < deadline, "TLS reload did not converge");
            thread::sleep(Duration::from_millis(25));
        }
    };
    poll(
        &mut server,
        200,
        if second.is_some() {
            &["same", "same"]
        } else {
            &["same"]
        },
    );
    let profile = if second.is_some() {
        "bounded-federated-union-lineage-v1"
    } else {
        "constant-mapping-source-v1"
    };
    let initial = proof::capture(address, query, &fixture.token, profile, &[proof::MAP]);
    if second.is_none() {
        proof::check_graph(
            address,
            &fixture.token,
            "constant-mapping-source-graph-v1",
            &[proof::MAP],
            &initial,
        );
    }
    let path = fixture.root.join("first.ttl");
    let mapping = std::fs::read_to_string(&path).unwrap();
    let second_path = fixture.root.join("second.ttl");
    let second_mapping = second.map(|_| std::fs::read_to_string(&second_path).unwrap());
    let replacement = mapping.replace("rr:column \"value\"", "rr:column \"refreshed\"");
    assert_ne!(replacement, mapping);
    std::fs::write(&path, replacement).unwrap();
    if let Some(mapping) = &second_mapping {
        std::fs::write(
            &second_path,
            mapping.replace("rr:column \"value\"", "rr:column \"refreshed\""),
        )
        .unwrap();
    }
    poll(&mut server, 200, expected);
    let mut current = proof::capture(address, query, &fixture.token, profile, &[proof::MAP]);
    proof::changed(&initial, &current);
    if second.is_none() {
        proof::check_graph(
            address,
            &fixture.token,
            "constant-mapping-source-graph-v1",
            &[proof::MAP],
            &current,
        );
    }
    assert_eq!(request(address, query, None).unwrap().0, 401);
    first.assert_encrypted_sessions();
    if let Some(second) = second {
        second.assert_encrypted_sessions();
    }
    let mut current_profile = profile;
    let mut current_maps = vec![proof::MAP];
    if second.is_none() {
        let multiple = ["a", "b"]
            .map(|id| mapping.replace("<#items>", &format!("<urn:reload:{id}>")))
            .join("\n")
            .replace(
                "rr:column \"value\"",
                "rr:column \"refreshed\" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string>",
            );
        fixture.write("first.ttl", &multiple);
        let until = Instant::now() + Duration::from_secs(40);
        loop {
            let (status, body) =
                request_format(address, query, Some(&fixture.token), lineage::FORMAT).unwrap();
            if status == 200 {
                let first = std::str::from_utf8(&body)
                    .unwrap()
                    .split('\u{1e}')
                    .nth(1)
                    .unwrap();
                let header: serde_json::Value = serde_json::from_str(first).unwrap();
                if header["profile"] == "bounded-mapping-source-v1" {
                    break;
                }
            }
            assert!(Instant::now() < until, "multi-map lineage reload");
            thread::sleep(Duration::from_millis(25));
        }
        current_profile = "bounded-mapping-source-v1";
        current_maps = vec!["urn:reload:a", "urn:reload:b"];
        let next = proof::capture(
            address,
            query,
            &fixture.token,
            current_profile,
            &current_maps,
        );
        proof::changed(&current, &next);
        assert_eq!(next.rows, current.rows);
        current = next;
        proof::check_graph(
            address,
            &fixture.token,
            "bounded-mapping-source-graph-v1",
            &current_maps,
            &current,
        );
        fixture.write(
            "first.ttl",
            &multiple.replace("rr:column \"refreshed\"", "rr:column \"value\""),
        );
        let next = proof::await_values(
            address,
            &fixture.token,
            current_profile,
            &current_maps,
            &["same"],
        );
        assert_eq!(next.rows, initial.rows);
        proof::changed(&current, &next);
        current = next;
        proof::check_graph(
            address,
            &fixture.token,
            "bounded-mapping-source-graph-v1",
            &current_maps,
            &current,
        );
    }
    proof::invalidate_while_pinned(
        fixture,
        first,
        address,
        query,
        current_profile,
        &current_maps,
        &current,
    );
    poll(&mut server, 503, &[]);
    std::fs::write(path, mapping).unwrap();
    if let Some(mapping) = second_mapping {
        std::fs::write(second_path, mapping).unwrap();
    }
    poll(
        &mut server,
        200,
        if second.is_some() {
            &["same", "same"]
        } else {
            &["same"]
        },
    );
    let repaired = proof::capture(address, query, &fixture.token, profile, &[proof::MAP]);
    assert_eq!(repaired.rows, initial.rows);
    assert_eq!(repaired.documents, initial.documents);
    assert_ne!(repaired.header["snapshot"], initial.header["snapshot"]);
    assert_ne!(
        repaired.header["logicalPlan"],
        initial.header["logicalPlan"]
    );
    assert_ne!(repaired.sources, initial.sources);
    assert_eq!(repaired.header["policy"], initial.header["policy"]);
    assert_eq!(proof::health(address, "/readyz"), Some(200));
    if second.is_none() {
        proof::check_graph(
            address,
            &fixture.token,
            "constant-mapping-source-graph-v1",
            &[proof::MAP],
            &repaired,
        );
    }
}

pub(super) fn assert_join_reloads(fixture: &Fixture, postgres: &Database, mysql: &Database) {
    let (mut command, address) = command(fixture, postgres, Some(mysql));
    command.args(["--reload-interval-secs", "1"]);
    let _server = stop_matrix::start(command, address);
    let profile = "bounded-federated-join-lineage-v1";
    let initial = [join::JOIN, join::REVERSED]
        .map(|query| proof::capture(address, query, &fixture.token, profile, &[proof::MAP]));
    assert_eq!(initial[0].rows.len(), 12);
    assert_eq!(initial[0].rows, initial[1].rows);
    let original = ["first.ttl", "second.ttl"]
        .map(|file| std::fs::read_to_string(fixture.root.join(file)).unwrap());
    for (file, mapping) in ["first.ttl", "second.ttl"].into_iter().zip(&original) {
        // Preserve the nonempty exact join key; change both observable subjects
        // and authored mapping-document identity, not just a generation counter.
        let replacement = mapping.replace(
            "http://example.test/item/",
            "http://example.test/reloaded-item/",
        );
        assert_ne!(replacement, *mapping);
        fixture.write(file, &replacement);
    }
    let expected: Vec<_> = initial[0]
        .rows
        .iter()
        .map(|row| {
            row.replace(
                "http://example.test/item/",
                "http://example.test/reloaded-item/",
            )
        })
        .collect();
    let poll = |expected: &[String]| {
        let until = Instant::now() + Duration::from_secs(40);
        loop {
            if let Some((200, body)) = request(address, join::JOIN, Some(&fixture.token)) {
                if stop_matrix::bag(&body).1 == expected {
                    break;
                }
            }
            assert!(
                Instant::now() < until,
                "nonempty join reload did not converge"
            );
            thread::sleep(Duration::from_millis(25));
        }
    };
    poll(&expected);
    let current = [join::JOIN, join::REVERSED]
        .map(|query| proof::capture(address, query, &fixture.token, profile, &[proof::MAP]));
    for (before, after) in initial.iter().zip(&current) {
        proof::changed(before, after);
        assert_eq!(after.rows, expected);
    }
    proof::invalidate_while_pinned(
        fixture,
        postgres,
        address,
        join::JOIN,
        profile,
        &[proof::MAP],
        &current[0],
    );
    for (file, mapping) in ["first.ttl", "second.ttl"].into_iter().zip(&original) {
        fixture.write(file, mapping);
    }
    poll(&initial[0].rows);
    for (query, before) in [join::JOIN, join::REVERSED].into_iter().zip(&initial) {
        let repaired = proof::capture(address, query, &fixture.token, profile, &[proof::MAP]);
        assert_eq!(repaired.rows, before.rows);
        assert_eq!(repaired.documents, before.documents);
        assert_eq!(repaired.header["policy"], before.header["policy"]);
        assert_ne!(repaired.header["snapshot"], before.header["snapshot"]);
        assert_ne!(repaired.header["logicalPlan"], before.header["logicalPlan"]);
        assert_ne!(repaired.sources, before.sources);
    }
    assert_eq!(proof::health(address, "/readyz"), Some(200));
    postgres.assert_encrypted_sessions();
    mysql.assert_encrypted_sessions();
}
