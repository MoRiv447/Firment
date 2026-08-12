use toml::Value;

fn collect_cheatsheets(value: &Value, out: &mut Vec<String>) {
    if let Value::Table(table) = value {
        if let Some(quickrefs) = table.get("quickref").and_then(|q| q.as_array()) {
            for quickref in quickrefs {
                if let Some(path) = quickref.get("cheatsheet").and_then(|c| c.as_str()) {
                    // Skip empty links: a quickref may exist without a
                    // cheatsheet file (e.g. a pure doc_section pointer).
                    if !path.is_empty() {
                        out.push(path.to_string());
                    }
                }
            }
        }
        for child in table.values() {
            collect_cheatsheets(child, out);
        }
    }
}

#[test]
fn seed_index_is_not_injected_twice_when_project_index_is_identical() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("vendor-index.toml"),
        firment_core::kb::seed_index_text(),
    )
    .unwrap();
    let prompt = firment_core::context::default_system_prompt(dir.path());
    assert_eq!(
        prompt.matches("Hardware knowledge base").count(),
        1,
        "seed + identical project index must not both be injected"
    );
}

#[test]
fn distinct_project_index_is_injected_alongside_seed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("vendor-index.toml"),
        "meta = { schema_version = \"1\" }\n[my-board]\n",
    )
    .unwrap();
    let prompt = firment_core::context::default_system_prompt(dir.path());
    assert_eq!(
        prompt.matches("Hardware knowledge base").count(),
        2,
        "seed and a distinct project index should both be injected"
    );
    assert!(prompt.contains("my-board"), "project index content missing");
}

#[test]
fn vendor_knowledge_base_files_parse_and_link() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index_path = root.join("docs").join("vendor-index.toml");
    assert!(index_path.is_file(), "missing {}", index_path.display());
    let text = std::fs::read_to_string(&index_path).unwrap();
    let index: Value = toml::from_str(&text).unwrap_or_else(|e| panic!("bad index toml: {e}"));
    assert!(
        index
            .get("meta")
            .and_then(|m| m.get("schema_version"))
            .and_then(|v| v.as_str())
            .is_some(),
        "meta.schema_version missing"
    );
    let mut cheatsheets = Vec::new();
    collect_cheatsheets(&index, &mut cheatsheets);
    assert!(!cheatsheets.is_empty(), "no quickref.cheatsheet links");
    for cheatsheet in &cheatsheets {
        let path = root.join("docs").join(cheatsheet);
        assert!(path.is_file(), "cheatsheet missing: {}", path.display());
        let content = std::fs::read_to_string(&path).unwrap();
        toml::from_str::<Value>(&content)
            .unwrap_or_else(|e| panic!("bad cheatsheet toml {}: {e}", path.display()));
    }
}
