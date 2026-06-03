//! Integration tests for ToolRegistry builtin initialization (MVP-0 Day 1).
//!
//! Verifies that `register_all_builtin()` succeeds, registers at least 8 tools,
//! and produces a registry with unique, non-empty tool names.
//!
//! Related:
//!   - `qianxun-core/src/tools/mod.rs` — `ToolRegistry::register_all_builtin`
//!   - `docs/30_子项目规划/05-mvp-0-checklist.md` — Day 1 §1.1 / §1.4
//!   - `docs/.mavis/plans/qianxun-multi-agent-architecture.md` §3.1 F7 / §3.2 缺口 7

use qianxun_core::tools::ToolRegistry;

#[test]
fn builtin_registry_loads_eight_tools() {
    let mut registry = ToolRegistry::new();
    let n = registry.register_all_builtin().expect("register_all_builtin");
    assert!(
        n >= 8,
        "expected >= 8 builtin tools, got {n}. (MVP-0 Day 1.1 contract: 8 core file/search/exec tools)"
    );
}

#[test]
fn builtin_tools_have_unique_names() {
    let mut registry = ToolRegistry::new();
    registry.register_all_builtin().expect("register_all_builtin");
    let names: Vec<String> = registry.list_names();
    let unique: std::collections::HashSet<_> = names.iter().cloned().collect();
    assert_eq!(
        names.len(),
        unique.len(),
        "duplicate tool names detected: {names:?}"
    );
}

#[test]
fn builtin_tools_count_matches_list_names() {
    let mut registry = ToolRegistry::new();
    registry.register_all_builtin().expect("register_all_builtin");
    assert_eq!(
        registry.builtin_count(),
        registry.list_names().len(),
        "builtin_count() and list_names().len() must agree"
    );
}

#[test]
fn builtin_tools_include_core_set() {
    // The original 8 tools promised by 05-mvp-0-checklist §1.1
    let mut registry = ToolRegistry::new();
    registry.register_all_builtin().expect("register_all_builtin");
    let names: std::collections::HashSet<String> =
        registry.list_names().into_iter().collect();

    for required in [
        "read_text_file",
        "write_text_file",
        "search",
        "grep",
        "list_directory",
        "execute_command",
        "edit_file",
    ] {
        assert!(
            names.contains(required),
            "core builtin '{required}' missing from registry; got {names:?}"
        );
    }
}

#[test]
fn register_all_builtin_is_idempotent_on_fresh_registry() {
    // Two independent registries should produce the same tool set.
    let mut a = ToolRegistry::new();
    let mut b = ToolRegistry::new();
    let na = a.register_all_builtin().expect("a");
    let nb = b.register_all_builtin().expect("b");
    assert_eq!(na, nb, "two fresh registries should register same count");
    let mut a_names = a.list_names();
    let mut b_names = b.list_names();
    a_names.sort();
    b_names.sort();
    assert_eq!(a_names, b_names, "two fresh registries should have same names");
}
