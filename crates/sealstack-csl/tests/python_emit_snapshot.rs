//! Snapshot tests for Python codegen.

use sealstack_csl::{CompileTargets, compile};

#[test]
fn python_emit_matches_rust_shapes_snapshot() {
    let src = include_str!("fixtures/rust_shapes.csl");
    let out = compile(src, CompileTargets::PYTHON).expect("compile");
    insta::assert_snapshot!("rust_shapes.py", out.python);
}

#[test]
fn python_emit_matches_hello_snapshot() {
    let src = include_str!("fixtures/hello.csl");
    let out = compile(src, CompileTargets::PYTHON).expect("compile");
    insta::assert_snapshot!("hello.py", out.python);
}
