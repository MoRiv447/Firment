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
    assert!(!names.contains(&"write_file"));
    assert!(!names.contains(&"edit_file"));
    assert!(!names.contains(&"shell"));
    assert_eq!(names.len(), 5);
}
