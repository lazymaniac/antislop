pub fn external(path: &std::path::Path) -> std::path::PathBuf {
    let path = path.canonicalize().expect("canonical test resource root");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    assert!(
        !path.starts_with(source),
        "test resources must be outside sources: {path:?}"
    );
    path
}
