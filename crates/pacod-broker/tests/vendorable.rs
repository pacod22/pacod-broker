//! pacod-gw vendors the client crates (model, index, client). A manifest that inherits from this
//! workspace (`version.workspace`, `dep.workspace = true`) does not build outside it, so these
//! three spell everything out.

#[test]
fn the_vendored_crates_inherit_nothing_from_the_workspace() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for c in ["pacod-broker-model", "pacod-broker-index", "pacod-broker-client"] {
        let text = std::fs::read_to_string(root.join(c).join("Cargo.toml")).unwrap();
        for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
            assert!(!line.contains("workspace"), "{c}/Cargo.toml inherits from the workspace: {line}");
        }
        assert!(!text.contains("path = \"../../"), "{c}/Cargo.toml reaches outside crates/");
    }
}
