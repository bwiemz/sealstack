//! Snapshot tests for TypeScript codegen.

use sealstack_csl::{CompileTargets, compile};

#[test]
fn typescript_emit_matches_rust_shapes_snapshot() {
    let src = include_str!("fixtures/rust_shapes.csl");
    let out = compile(src, CompileTargets::TYPESCRIPT).expect("compile");
    insta::assert_snapshot!("rust_shapes.ts", out.typescript);
}

#[test]
fn typescript_emit_matches_hello_snapshot() {
    let src = include_str!("fixtures/hello.csl");
    let out = compile(src, CompileTargets::TYPESCRIPT).expect("compile");
    insta::assert_snapshot!("hello.ts", out.typescript);
}
