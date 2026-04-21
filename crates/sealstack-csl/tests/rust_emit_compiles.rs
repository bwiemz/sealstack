//! Verifies that the Rust code emitted by `CompileTargets::RUST` actually
//! compiles via a real `cargo check`, not just a `syn::parse_file` shortcut.
//!
//! Gated behind the `slow-tests` feature because it spins up cargo in a
//! tempdir (5–10s wall clock). CI runs `cargo test --features slow-tests`.

#![cfg(feature = "slow-tests")]

use std::process::Command;

use sealstack_csl::{CompileTargets, compile};

#[test]
fn generated_rust_compiles_via_cargo_check() {
    let src = include_str!("fixtures/rust_shapes.csl");
    let out = compile(src, CompileTargets::RUST).expect("compile");

    let dir = tempfile::tempdir().expect("tempdir");
    let pkg = dir.path().join("generated_check");
    std::fs::create_dir_all(pkg.join("src")).unwrap();
    std::fs::write(
        pkg.join("Cargo.toml"),
        r#"[package]
name = "generated_check"
version = "0.0.0"
edition = "2024"

[dependencies]
serde      = { version = "1", features = ["derive"] }
serde_json = "1"

[workspace]
"#,
    )
    .unwrap();
    std::fs::write(pkg.join("src/lib.rs"), &out.rust).unwrap();

    let status = Command::new("cargo")
        .args(["check", "--quiet"])
        .current_dir(&pkg)
        .status()
        .expect("spawn cargo check");

    assert!(
        status.success(),
        "generated Rust failed to compile; inspect {}",
        pkg.display()
    );
}
