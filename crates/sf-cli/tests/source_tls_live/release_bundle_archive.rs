//! Adversarial image.tar fixtures for the offline verifier, built in memory.
use super::*;
use serde_json::json;

const OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const OCI_LAYER: &str = "application/vnd.oci.image.layer.v1.tar";

#[derive(Clone)]
pub(super) struct Member {
    name: String,
    kind: u8,
    link: String,
    data: Vec<u8>,
}

fn special(name: &str, kind: u8, link: &str, data: &[u8]) -> Member {
    Member {
        name: name.to_owned(),
        kind,
        link: link.to_owned(),
        data: data.to_vec(),
    }
}

fn file(name: &str, data: &[u8]) -> Member {
    special(name, b'0', "", data)
}

fn dir(name: &str) -> Member {
    special(name, b'5', "", &[])
}

fn block(member: &Member) -> Vec<u8> {
    let name = member.name.as_bytes();
    let link = member.link.as_bytes();
    let mut header = [0u8; 512];
    header[..name.len()].copy_from_slice(name);
    header[100..108].copy_from_slice(b"0000644\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    let size = format!("{:011o}\0", member.data.len());
    header[124..136].copy_from_slice(size.as_bytes());
    header[136..148].copy_from_slice(b"00000000000\0");
    header[148..156].copy_from_slice(b"        ");
    header[156] = member.kind;
    header[157..157 + link.len()].copy_from_slice(link);
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
    let field = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(field.as_bytes());
    let mut out = header.to_vec();
    out.extend_from_slice(&member.data);
    out.resize(out.len().div_ceil(512) * 512, 0);
    out
}

pub(super) fn render(members: &[Member]) -> Vec<u8> {
    let mut out: Vec<u8> = members.iter().flat_map(block).collect();
    out.extend_from_slice(&[0u8; 1024]);
    out
}

fn layer() -> Vec<u8> {
    vec![0u8; 1024]
}

fn layer_hex() -> String {
    sha256_bytes(&layer())
}

pub(super) fn config(revision: &str) -> Vec<u8> {
    let diff = format!("sha256:{}", layer_hex());
    let labels = json!({
        "org.opencontainers.image.revision": revision,
        "org.opencontainers.image.version": PACKAGE,
    });
    let value = json!({
        "architecture": "amd64",
        "os": "linux",
        "config": {"Labels": labels},
        "rootfs": {"type": "layers", "diff_ids": [diff]},
    });
    value.to_string().into_bytes()
}

pub(super) fn id(config: &[u8]) -> String {
    format!("sha256:{}", sha256_bytes(config))
}

fn docker_manifest(config_path: &str) -> Member {
    let layers = [format!("{}/layer.tar", layer_hex())];
    let value = json!([{
        "Config": config_path,
        "RepoTags": ["semantic-fabric:fixture"],
        "Layers": layers,
    }]);
    file("manifest.json", value.to_string().as_bytes())
}

pub(super) fn docker(claimed: &str, config: &[u8]) -> Vec<Member> {
    let layer_dir = layer_hex();
    let layer_path = format!("{layer_dir}/layer.tar");
    vec![
        file(&format!("{claimed}.json"), config),
        dir(&format!("{layer_dir}/")),
        file(&layer_path, &layer()),
        docker_manifest(&format!("{claimed}.json")),
    ]
}

fn manifest(claimed: &str, size: usize, note: &str) -> Vec<u8> {
    let value = json!({
        "schemaVersion": 2,
        "mediaType": OCI_MANIFEST,
        "annotations": {"note": note},
        "config": {
            "mediaType": OCI_CONFIG,
            "digest": format!("sha256:{claimed}"),
            "size": size,
        },
        "layers": [{
            "mediaType": OCI_LAYER,
            "digest": format!("sha256:{}", layer_hex()),
            "size": 1024,
        }],
    });
    value.to_string().into_bytes()
}

fn blob(hex: &str, data: &[u8]) -> Member {
    file(&format!("blobs/sha256/{hex}"), data)
}

fn oci_from(claimed: &str, config: &[u8], manifests: &[Vec<u8>]) -> Vec<Member> {
    let mut entries = Vec::new();
    for data in manifests {
        let entry = json!({
            "mediaType": OCI_MANIFEST,
            "digest": format!("sha256:{}", sha256_bytes(data)),
            "size": data.len(),
        });
        entries.push(entry);
    }
    let index = json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": entries,
    });
    let mut members = vec![
        file("oci-layout", br#"{"imageLayoutVersion":"1.0.0"}"#),
        file("index.json", index.to_string().as_bytes()),
        dir("blobs/"),
        dir("blobs/sha256/"),
        blob(claimed, config),
        blob(&layer_hex(), &layer()),
    ];
    for data in manifests {
        members.push(blob(&sha256_bytes(data), data));
    }
    members
}

fn oci(claimed: &str, config: &[u8]) -> Vec<Member> {
    oci_from(claimed, config, &[manifest(claimed, config.len(), "one")])
}

fn subject() -> (Vec<u8>, String, String) {
    let bytes = config(REV);
    let image = id(&bytes);
    let hex = image[7..].to_owned();
    (bytes, image, hex)
}

fn built(image: &str, members: &[Member]) -> Bundle {
    let bundle = Bundle::build(image, render(members));
    assemble(&bundle.dir);
    bundle
}

fn accepted(image: &str, members: &[Member]) {
    let bundle = built(image, members);
    let output = verify(&bundle.dir, &[]);
    assert!(output.status.success(), "{}", stderr(&output));
}

fn rejected(image: &str, members: &[Member], needle: &str) {
    let bundle = built(image, members);
    verify_fails(&bundle.dir, &[], needle);
}

fn with_copy(base: &[Member], original: &str, alias: &str) -> Vec<Member> {
    let found = base.iter().find(|m| m.name == original);
    let mut copy = found.unwrap().clone();
    copy.name = alias.to_owned();
    let mut members = base.to_vec();
    members.push(copy);
    members
}

fn replace(base: &[Member], name: &str, with: Member) -> Vec<Member> {
    let mut members = Vec::new();
    for member in base {
        if member.name == name {
            members.push(with.clone());
        } else {
            members.push(member.clone());
        }
    }
    members
}

fn remove(base: &[Member], name: &str) -> Vec<Member> {
    let mut members = base.to_vec();
    members.retain(|m| m.name != name);
    members
}

fn with_manifest(base: &[Member], manifest: Member) -> Vec<Member> {
    replace(base, "manifest.json", manifest)
}

#[test]
fn genuine_docker_oci_and_combined_archives_verify() {
    let (bytes, image, hex) = subject();
    accepted(&image, &docker(&hex, &bytes));
    accepted(&image, &oci(&hex, &bytes));
    let mut both = oci(&hex, &bytes);
    let layers = [format!("blobs/sha256/{}", layer_hex())];
    let value = json!([{
        "Config": format!("blobs/sha256/{hex}"),
        "RepoTags": ["semantic-fabric:fixture"],
        "Layers": layers,
    }]);
    both.push(file("manifest.json", value.to_string().as_bytes()));
    accepted(&image, &both);
}

#[test]
fn dot_slash_prefixed_member_names_verify() {
    let (bytes, image, hex) = subject();
    let mut members = docker(&hex, &bytes);
    for member in &mut members {
        member.name = format!("./{}", member.name);
    }
    accepted(&image, &members);
}

#[test]
fn sha256_helper_matches_known_empty_object_digest() {
    let known = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
    assert_eq!(sha256_bytes(b"{}"), known);
}

#[test]
fn filename_only_witness_is_rejected() {
    // The former verifier accepted any member named for imageId, whatever its bytes.
    let fake = "f".repeat(64);
    let image = format!("sha256:{fake}");
    let bare = [blob(&fake, b"{}")];
    rejected(&image, &bare, "neither manifest.json nor");
    rejected(&image, &oci(&fake, b"{}"), "digest does not match");
    rejected(&image, &docker(&fake, b"{}"), "Docker config digest");
}

#[test]
fn genuine_digest_over_non_image_bytes_is_rejected() {
    let image = id(b"{}");
    let hex = image[7..].to_owned();
    rejected(&image, &docker(&hex, b"{}"), "not linux/amd64");
    rejected(&image, &oci(&hex, b"{}"), "not linux/amd64");
}

#[test]
fn renamed_other_config_payload_is_rejected() {
    let (_, image, hex) = subject();
    let other = config(OTHER_REV);
    rejected(&image, &docker(&hex, &other), "Docker config digest");
    rejected(&image, &oci(&hex, &other), "digest does not match");
}

#[test]
fn config_labels_must_match_the_artifact() {
    let other = config(OTHER_REV);
    let image = id(&other);
    let hex = image[7..].to_owned();
    rejected(&image, &docker(&hex, &other), "labels do not match");
    rejected(&image, &oci(&hex, &other), "labels do not match");
}

#[test]
fn duplicate_members_are_rejected() {
    let (bytes, image, hex) = subject();
    let docker_base = docker(&hex, &bytes);
    let oci_base = oci(&hex, &bytes);
    let config_json = format!("{hex}.json");
    let config_blob = format!("blobs/sha256/{hex}");
    let cases = [
        with_copy(&docker_base, &config_json, &config_json),
        with_copy(&docker_base, "manifest.json", "./manifest.json"),
        with_copy(&oci_base, &config_blob, &config_blob),
        with_copy(&oci_base, "index.json", "index.json"),
    ];
    for members in cases {
        rejected(&image, &members, "duplicate");
    }
}

#[test]
fn links_and_directories_in_place_of_config_are_rejected() {
    let (bytes, image, hex) = subject();
    let docker_base = docker(&hex, &bytes);
    let oci_base = oci(&hex, &bytes);
    let docker_path = format!("{hex}.json");
    let oci_path = format!("blobs/sha256/{hex}");
    let kinds = [
        (b'2', "symlink"),
        (b'1', "hardlink"),
        (b'5', "not a regular file"),
    ];
    for (kind, needle) in kinds {
        let bad = special(&docker_path, kind, "manifest.json", &[]);
        let members = replace(&docker_base, &docker_path, bad);
        rejected(&image, &members, needle);
        let bad = special(&oci_path, kind, "index.json", &[]);
        let members = replace(&oci_base, &oci_path, bad);
        rejected(&image, &members, needle);
    }
    let mut extra = docker_base.clone();
    extra.push(special("extra", b'2', "manifest.json", &[]));
    rejected(&image, &extra, "symlink");
}

#[test]
fn wrong_docker_manifest_linkage_is_rejected() {
    let (bytes, image, hex) = subject();
    let base = docker(&hex, &bytes);
    let path = format!("{hex}.json");
    let members = remove(&base, &path);
    rejected(&image, &members, "lacks Docker config");
    let layer_path = format!("{}/layer.tar", layer_hex());
    let members = remove(&base, &layer_path);
    rejected(&image, &members, "lacks Docker layer");
    let members = with_manifest(&base, file("manifest.json", b"[]"));
    rejected(&image, &members, "exactly one image");
    let no_layers = br#"[{"Config":"x.json"}]"#;
    let members = with_manifest(&base, file("manifest.json", no_layers));
    rejected(&image, &members, "Layers");
    let members = with_manifest(&base, docker_manifest("../outside"));
    rejected(&image, &members, "unsafe path");
    let other = config(OTHER_REV);
    let other_json = format!("{}.json", sha256_bytes(&other));
    let mut members = with_manifest(&base, docker_manifest(&other_json));
    members.push(file(&other_json, &other));
    rejected(&image, &members, "Docker config digest");
    let only_config = [file(&path, &bytes)];
    rejected(&image, &only_config, "neither manifest.json nor");
}

#[test]
fn wrong_oci_manifest_linkage_is_rejected() {
    let (bytes, image, hex) = subject();
    let base = oci(&hex, &bytes);
    let members = remove(&base, "oci-layout");
    rejected(&image, &members, "lacks oci-layout");
    let members = remove(&base, &format!("blobs/sha256/{hex}"));
    rejected(&image, &members, "lacks manifest config");
    let members = remove(&base, &format!("blobs/sha256/{}", layer_hex()));
    rejected(&image, &members, "lacks layer blob");
    let other = config(OTHER_REV);
    let other_hex = sha256_bytes(&other);
    rejected(&image, &oci(&other_hex, &other), "no OCI manifest");
    let wrong_size = manifest(&hex, bytes.len() + 1, "one");
    let members = oci_from(&hex, &bytes, &[wrong_size]);
    rejected(&image, &members, "size differs");
    let one = manifest(&hex, bytes.len(), "one");
    let two = manifest(&hex, bytes.len(), "two");
    let members = oci_from(&hex, &bytes, &[one, two]);
    rejected(&image, &members, "more than one");
    let good = manifest(&hex, bytes.len(), "one");
    let good_hex = sha256_bytes(&good);
    let text = String::from_utf8(good).unwrap();
    let forged = text.replace("\"schemaVersion\":2", "\"schemaVersion\":3");
    let path = format!("blobs/sha256/{good_hex}");
    let tampered = blob(&good_hex, forged.as_bytes());
    let members = replace(&base, &path, tampered);
    rejected(&image, &members, "digest does not match");
}

#[test]
fn unsafe_member_paths_are_rejected() {
    let (bytes, image, hex) = subject();
    let base = docker(&hex, &bytes);
    let names = [
        "../evil",
        "/etc/passwd",
        "blobs/../x",
        "a//b",
        "a/./b",
        "a\\b",
    ];
    for name in names {
        let mut members = base.clone();
        members.push(file(name, b"x"));
        rejected(&image, &members, "unsafe");
    }
}

#[test]
fn non_tar_archive_is_rejected() {
    let (_, image, _) = subject();
    let bundle = Bundle::build(&image, b"not a tar archive".to_vec());
    assemble(&bundle.dir);
    verify_fails(&bundle.dir, &[], "readable tar");
}
