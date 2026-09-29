//! Offline serving-bundle tests. Synthetic local fixture; no Docker or network.
use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[path = "source_tls_live/release_bundle_archive.rs"]
mod archive;

const REV: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_REV: &str = "89abcdef89abcdef89abcdef89abcdef89abcdef";
const PACKAGE: &str = "0.1.0-dev.1-git.0123456789ab";
const OTHER: &str = "sha256:0011223344556677001122334455667700112233445566770011223344556677";
const CHECK_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const CHECK_B: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const EPOCH: &str = "1700000000";
const UNSAFE_PATHS: [&str; 4] = [
    "../outside",
    "/etc/passwd",
    "evidence/../artifact.json",
    "evidence//rustc.txt",
];

const CARGO_LOCK: &str = r#"version = 4

[[package]]
name = "alpha"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "@CHECK_A@"

[[package]]
name = "beta"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "@CHECK_B@"

[[package]]
name = "dev-only"
version = "3.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "@CHECK_B@"

[[package]]
name = "gamma"
version = "0.3.0"
source = "git+https://git.invalid/gamma#0123456789abcdef0123456789abcdef01234567"

[[package]]
name = "sf-cli"
version = "0.1.0-dev.1"
dependencies = [
 "alpha",
 "beta 2.0.0",
]
"#;

const METADATA: &str = r#"{"version": 1, "packages": [
{"name": "sf-cli", "version": "0.1.0-dev.1", "source": null, "license": "MIT OR Apache-2.0"},
{"name": "alpha", "version": "1.0.0", "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "Apache-2.0 OR MIT"},
{"name": "beta", "version": "2.0.0", "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "MIT/Apache-2.0"},
{"name": "gamma", "version": "0.3.0", "source": "git+https://git.invalid/gamma#0123456789abcdef0123456789abcdef01234567", "license": null},
{"name": "dev-only", "version": "3.0.0", "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "MIT"}
]}
"#;

const TREE: &str = "sf-cli v0.1.0-dev.1 (/build/crates/sf-cli)|\nalpha v1.0.0|default,std\nbeta v2.0.0 (proc-macro)|\nalpha v1.0.0|default,std (*)\ngamma v0.3.0|\n";

const RUSTC: &str = "rustc 1.96.0 (0123456789a 2026-01-01)\nbinary: rustc\ncommit-hash: 0123456789abcdef\nhost: x86_64-unknown-linux-gnu\nrelease: 1.96.0\n";

const INSPECT: &str = r#"[{"Id": "@IMAGE@", "Architecture": "amd64", "Os": "linux", "Config": {"Labels": {"org.opencontainers.image.revision": "@REV@", "org.opencontainers.image.version": "0.1.0-dev.1-git.0123456789ab"}}}]
"#;

const ARTIFACT: &str = r#"{
  "schemaVersion": 1,
  "sourceRevision": "@REV@",
  "crateVersion": "0.1.0-dev.1",
  "packageVersion": "0.1.0-dev.1-git.0123456789ab",
  "platform": "linux/amd64",
  "imageId": "@IMAGE@",
  "imageArchive": {"path": "image.tar", "sha256": "@ARCHIVE@"},
  "cargoLockSha256": "@LOCK@",
  "buildCommand": "cargo build --locked --release -p sf-cli --no-default-features",
  "admission": "unqualified; live smoke, advisory review or waivers, signature and release checks remain required"
}
"#;

struct Bundle {
    root: PathBuf,
    dir: PathBuf,
    image: String,
}

impl Drop for Bundle {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Bundle {
    fn build(image: &str, tar: Vec<u8>) -> Bundle {
        let name = format!("sf-bundle-{}", uuid::Uuid::new_v4());
        let root = std::env::temp_dir().join(name);
        let dir = root.join("bundle");
        fs::create_dir_all(dir.join("evidence")).unwrap();
        let image = image.to_owned();
        let bundle = Bundle { root, dir, image };
        write_fixture(&bundle.dir, &bundle.image, &tar);
        bundle
    }

    fn assembled() -> Bundle {
        let bytes = archive::config(REV);
        let image = archive::id(&bytes);
        let members = archive::docker(&image[7..], &bytes);
        let bundle = Bundle::build(&image, archive::render(&members));
        assemble(&bundle.dir);
        bundle
    }
}

fn fill(template: &str, image: &str) -> String {
    let text = template.replace("@IMAGE@", image);
    let text = text.replace("@REV@", REV);
    let text = text.replace("@CHECK_A@", CHECK_A);
    text.replace("@CHECK_B@", CHECK_B)
}

fn write(dir: &Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).unwrap();
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap()
}

fn read_json(dir: &Path, name: &str) -> serde_json::Value {
    serde_json::from_str(&read(dir, name)).unwrap()
}

fn edit(dir: &Path, name: &str, from: &str, to: &str) {
    let text = read(dir, name);
    assert!(text.contains(from), "{name} lacks {from}");
    write(dir, name, &text.replacen(from, to, 1));
}

fn sha256(path: &Path) -> String {
    let output = Command::new("sha256sum").arg(path).output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    text[..64].to_owned()
}

fn sha256_bytes(data: &[u8]) -> String {
    let mut command = Command::new("sha256sum");
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(data).unwrap();
    let output = child.wait_with_output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    text[..64].to_owned()
}

fn write_fixture(dir: &Path, image: &str, tar: &[u8]) {
    let evidence = dir.join("evidence");
    write(&evidence, "Cargo.lock", &fill(CARGO_LOCK, image));
    write(&evidence, "cargo-metadata.json", METADATA);
    write(&evidence, "serving-dependencies.txt", TREE);
    write(&evidence, "rustc.txt", RUSTC);
    write(&evidence, "LICENSE-MIT", "MIT fixture license text\n");
    write(&evidence, "LICENSE-APACHE", "Apache fixture license text\n");
    write(dir, "image.id", image);
    write(dir, "image-inspect.json", &fill(INSPECT, image));
    fs::write(dir.join("image.tar"), tar).unwrap();
    let archive_sha = sha256(&dir.join("image.tar"));
    let lock = sha256(&evidence.join("Cargo.lock"));
    let artifact = fill(ARTIFACT, image).replace("@ARCHIVE@", &archive_sha);
    let artifact = artifact.replace("@LOCK@", &lock);
    write(dir, "artifact.json", &artifact);
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn python(script: &str, args: &[&str]) -> Output {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script_path = manifest.join("../../scripts/release").join(script);
    let mut command = Command::new("python3");
    command.env("PYTHONDONTWRITEBYTECODE", "1");
    command.arg(script_path);
    command.args(args);
    command.output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assemble(dir: &Path) {
    let args = ["assemble", arg(dir), "--source-date-epoch", EPOCH];
    let output = python("serving-evidence.py", &args);
    assert!(output.status.success(), "{}", stderr(&output));
}

fn seal(dir: &Path) {
    let output = python("serving-evidence.py", &["seal", arg(dir)]);
    assert!(output.status.success(), "{}", stderr(&output));
}

fn verify(dir: &Path, extra: &[&str]) -> Output {
    let mut args = vec![arg(dir)];
    args.extend_from_slice(extra);
    python("verify-serving-bundle.py", &args)
}

fn verify_fails(dir: &Path, extra: &[&str], needle: &str) {
    let output = verify(dir, extra);
    let message = stderr(&output);
    assert!(!output.status.success(), "accepted a bad bundle");
    assert!(message.contains(needle), "wanted {needle:?}: {message}");
}

fn package<'a>(sbom: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    for item in sbom["packages"].as_array().unwrap() {
        if item["name"] == name {
            return item;
        }
    }
    panic!("package {name} missing");
}

#[test]
fn valid_bundle_verifies_offline_and_stays_unqualified() {
    let bundle = Bundle::assembled();
    let image = bundle.image.as_str();
    let expect = ["--expect-image-id", image, "--expect-revision", REV];
    let output = verify(&bundle.dir, &expect);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unqualified"));
    assert_eq!(read(&bundle.dir, "SHA256SUMS").lines().count(), 13);
    let sbom_text = read(&bundle.dir, "evidence/sbom.spdx.json");
    assert!(!sbom_text.contains("dev-only"));
    let sbom: serde_json::Value = serde_json::from_str(&sbom_text).unwrap();
    assert_eq!(sbom["spdxVersion"], "SPDX-2.3");
    let alpha = package(&sbom, "alpha");
    assert_eq!(alpha["licenseDeclared"], "Apache-2.0 OR MIT");
    assert_eq!(alpha["checksums"][0]["checksumValue"], CHECK_A);
    let note = alpha["comment"].as_str().unwrap();
    assert!(note.contains("default,std"));
    let beta = package(&sbom, "beta");
    assert_eq!(beta["licenseDeclared"], "NOASSERTION");
    let gamma = package(&sbom, "gamma");
    assert!(gamma["checksums"].is_null());
    assert_eq!(gamma["licenseDeclared"], "NOASSERTION");
    assert!(package(&sbom, "sf-cli")["checksums"].is_null());
    let graph = read_json(&bundle.dir, "evidence/cargo-serving-graph.json");
    assert_eq!(graph["packages"].as_array().unwrap().len(), 4);
    let prov = read_json(&bundle.dir, "evidence/provenance.json");
    assert_eq!(prov["subject"]["imageId"], image);
    assert_eq!(prov["signature"]["status"], "unsigned");
    assert_eq!(prov["admission"], "unqualified");
}

#[test]
fn assembly_is_deterministic() {
    let bundle = Bundle::assembled();
    let first = read(&bundle.dir, "SHA256SUMS");
    assemble(&bundle.dir);
    assert_eq!(first, read(&bundle.dir, "SHA256SUMS"));
}

#[test]
fn tampered_file_digest_is_rejected() {
    let bundle = Bundle::assembled();
    let text = read(&bundle.dir, "evidence/Cargo.lock");
    write(&bundle.dir, "evidence/Cargo.lock", &format!("{text}\n"));
    verify_fails(&bundle.dir, &[], "digest mismatch");
}

#[test]
fn missing_declared_file_is_rejected() {
    let bundle = Bundle::assembled();
    fs::remove_file(bundle.dir.join("evidence/rustc.txt")).unwrap();
    verify_fails(&bundle.dir, &[], "missing file");
}

#[test]
fn missing_inventory_is_rejected() {
    let bundle = Bundle::assembled();
    fs::remove_file(bundle.dir.join("SHA256SUMS")).unwrap();
    verify_fails(&bundle.dir, &[], "missing file");
}

#[test]
fn undeclared_file_is_rejected() {
    let bundle = Bundle::assembled();
    write(&bundle.dir, "evidence/extra.txt", "unlisted");
    verify_fails(&bundle.dir, &[], "undeclared");
}

#[test]
fn malformed_inventory_is_rejected() {
    let bundle = Bundle::assembled();
    write(&bundle.dir, "SHA256SUMS", "not a checksum line\n");
    verify_fails(&bundle.dir, &[], "malformed");
}

#[test]
fn unsafe_inventory_paths_are_rejected() {
    let bundle = Bundle::assembled();
    let zero = "0".repeat(64);
    for name in UNSAFE_PATHS {
        write(&bundle.dir, "SHA256SUMS", &format!("{zero}  {name}\n"));
        verify_fails(&bundle.dir, &[], "unsafe path");
    }
}

#[test]
fn symlinked_file_is_rejected() {
    let bundle = Bundle::assembled();
    let path = bundle.dir.join("evidence/rustc.txt");
    fs::remove_file(&path).unwrap();
    symlink("Cargo.lock", &path).unwrap();
    verify_fails(&bundle.dir, &[], "symlink");
}

#[test]
fn symlinked_bundle_directory_is_rejected() {
    let bundle = Bundle::assembled();
    let link = bundle.root.join("linked");
    symlink(&bundle.dir, &link).unwrap();
    verify_fails(&link, &[], "symlink");
}

#[test]
fn mismatched_image_id_in_artifact_is_rejected() {
    let bundle = Bundle::assembled();
    edit(&bundle.dir, "artifact.json", &bundle.image, OTHER);
    seal(&bundle.dir);
    verify_fails(&bundle.dir, &[], "imageId");
}

#[test]
fn mismatched_image_id_file_is_rejected() {
    let bundle = Bundle::assembled();
    write(&bundle.dir, "image.id", OTHER);
    seal(&bundle.dir);
    verify_fails(&bundle.dir, &[], "imageId");
}

#[test]
fn unexpected_image_id_or_revision_is_rejected() {
    let bundle = Bundle::assembled();
    let other_image = ["--expect-image-id", OTHER];
    verify_fails(&bundle.dir, &other_image, "expected image ID");
    let other_rev = ["--expect-revision", OTHER_REV];
    verify_fails(&bundle.dir, &other_rev, "expected source revision");
}

#[test]
fn mismatched_source_revision_label_is_rejected() {
    let bundle = Bundle::assembled();
    edit(&bundle.dir, "image-inspect.json", REV, OTHER_REV);
    seal(&bundle.dir);
    verify_fails(&bundle.dir, &[], "revision");
}

#[test]
fn altered_cargo_metadata_is_rejected() {
    let bundle = Bundle::assembled();
    let path = "evidence/cargo-metadata.json";
    edit(&bundle.dir, path, "Apache-2.0 OR MIT", "MIT");
    seal(&bundle.dir);
    verify_fails(&bundle.dir, &[], "cargo-serving-graph.json");
}

#[test]
fn altered_provenance_is_rejected() {
    let bundle = Bundle::assembled();
    let path = "evidence/provenance.json";
    let (from, to) = (r#""status": "unsigned""#, r#""status": "signed""#);
    edit(&bundle.dir, path, from, to);
    seal(&bundle.dir);
    verify_fails(&bundle.dir, &[], "provenance.json");
}

#[test]
fn unsupported_schemas_are_rejected() {
    let bundle = Bundle::assembled();
    let (from, to) = ("\"schemaVersion\": 1", "\"schemaVersion\": 2");
    edit(&bundle.dir, "artifact.json", from, to);
    seal(&bundle.dir);
    verify_fails(&bundle.dir, &[], "unsupported schema");
}

#[test]
fn unsupported_metadata_schema_is_rejected() {
    let bundle = Bundle::assembled();
    let (from, to) = ("\"version\": 1,", "\"version\": 2,");
    edit(&bundle.dir, "evidence/cargo-metadata.json", from, to);
    seal(&bundle.dir);
    verify_fails(&bundle.dir, &[], "unsupported schema");
}

#[test]
fn unsupported_sbom_schema_is_rejected() {
    let bundle = Bundle::assembled();
    let (from, to) = ("\"SPDX-2.3\"", "\"SPDX-2.2\"");
    edit(&bundle.dir, "evidence/sbom.spdx.json", from, to);
    seal(&bundle.dir);
    verify_fails(&bundle.dir, &[], "unsupported schema");
}
