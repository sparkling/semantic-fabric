use sf_bench::performance::paths::RepositoryLayout;

fn layout() -> (tempfile::TempDir, RepositoryLayout) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let root = dir.path().canonicalize().unwrap();
    let layout = RepositoryLayout::new(root).unwrap();
    (dir, layout)
}

#[test]
fn should_create_fixed_output_once_and_never_overwrite_it() {
    let (_dir, layout) = layout();

    let path = layout
        .write_new_fixed("target/sf-performance/candidate-v1.tsv", b"first\n")
        .unwrap();
    let second = layout.write_new_fixed("target/sf-performance/candidate-v1.tsv", b"second\n");

    assert!(second.is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"first\n");
}

#[cfg(unix)]
#[test]
fn should_reject_a_symlink_in_a_fixed_read_path() {
    let (dir, layout) = layout();
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, dir.path().join("target")).unwrap();

    let result = layout.fixed_path("target/sf-performance/candidate-v1.tsv");

    assert!(result.is_err());
}

#[test]
fn should_reject_parent_components_even_when_they_resolve_inside_root() {
    let (_dir, layout) = layout();

    assert!(layout.fixed_path("target/../candidate.tsv").is_err());
}

#[cfg(unix)]
#[test]
fn should_reject_hard_linked_fixed_authorities() {
    let (dir, layout) = layout();
    let authority = dir.path().join("authority.tsv");
    let alias = dir.path().join("authority-alias.tsv");
    std::fs::write(&authority, b"authority\n").unwrap();
    std::fs::hard_link(&authority, &alias).unwrap();

    let error = layout.read_fixed("authority.tsv", 64).unwrap_err();

    assert!(error.to_string().contains("hard link"));
}

#[test]
fn should_remove_only_an_empty_directory_under_the_fixed_work_path() {
    let (dir, layout) = layout();
    let run = layout.create_run_directory("run-1-2").unwrap();

    layout.remove_run_directory(&run).unwrap();

    assert!(!run.exists());
    assert!(layout.remove_run_directory(dir.path()).is_err());
}
