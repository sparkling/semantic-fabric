//! Required live proof that native reload retains authenticated TLS source routing.
use super::*;

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
    assert_eq!(request(address, query, None).unwrap().0, 401);
    first.assert_encrypted_sessions();
    if let Some(second) = second {
        second.assert_encrypted_sessions();
    }
    std::fs::write(&path, "not turtle").unwrap();
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
}
