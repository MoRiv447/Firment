use firment_tools::plan_registry;

#[test]
fn plan_registry_exposes_only_read_only_tools() {
    let registry = plan_registry();
    let names = registry.names();
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"list_dir"));
    assert!(names.contains(&"glob"));
    assert!(names.contains(&"grep"));
    assert!(names.contains(&"symbols"));
    assert!(names.contains(&"models"));
    assert!(names.contains(&"web_search"));
    assert!(names.contains(&"web_fetch"));
    assert!(names.contains(&"task"));
    assert!(names.contains(&"todo"));
    assert!(names.contains(&"ask_user"));
    assert!(names.contains(&"elf_analyze"));
    assert!(names.contains(&"periph_init"));
    assert!(names.contains(&"device_log"));
    assert!(names.contains(&"observe"));
    assert!(!names.contains(&"write_file"));
    assert!(!names.contains(&"edit_file"));
    assert!(!names.contains(&"shell"));
    // Mutating registries must never leak into plan mode: pinmap's
    // claim/release write workbench.toml.
    assert!(!names.contains(&"pinmap"));
    assert!(!names.contains(&"device_cmd"));
    assert!(!names.contains(&"decision"));
    assert_eq!(names.len(), 15);
}
