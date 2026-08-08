use toml::Value;

fn collect_cheatsheets(value: &Value, out: &mut Vec<String>) {
    if let Value::Table(table) = value {
        if let Some(quickrefs) = table.get("quickref").and_then(|q| q.as_array()) {
            for quickref in quickrefs {
                if let Some(path) = quickref.get("cheatsheet").and_then(|c| c.as_str()) {
                    out.push(path.to_string());
                }
            }
        }
        for child in table.values() {
            collect_cheatsheets(child, out);
        }
    }
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
