//! External regression for the retired public R7 authority path. The crate
//! doctest proves imports fail; this source assertion prevents an accidental
//! `pub mod r7` restoration from being hidden by internal test compilation.

#[test]
fn runtime_r7_namespace_remains_private() {
    let root = include_str!("../src/lib.rs");
    assert!(root.contains("mod r7;"));
    assert!(!root.contains("pub mod r7;"));
}
