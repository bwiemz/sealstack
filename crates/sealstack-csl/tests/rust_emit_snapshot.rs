//! Snapshot test for Rust struct codegen.

use sealstack_csl::{CompileTargets, compile};

#[test]
fn rust_emit_matches_snapshot() {
    let src = include_str!("fixtures/rust_shapes.csl");
    let out = compile(src, CompileTargets::RUST).expect("compile");
    insta::assert_snapshot!("rust_shapes.rs", out.rust);
}
