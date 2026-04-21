//! End-to-end: compile a CSL policy, patch the runtime, instantiate via
//! wasmtime (matching how the gateway loads bundles), and assert verdicts
//! for a matrix of (caller, record, action) inputs.

use sealstack_csl::codegen::policy;
use sealstack_csl::{parser, types};
use wasmtime::{Engine, Instance, Module, Store};

fn compile_and_instantiate(src: &str, schema: &str) -> (Engine, Module) {
    let file = parser::parse_file(src).unwrap();
    let typed = types::check(&file).unwrap();
    let bundles = policy::emit_policy_bundles(&typed).unwrap();
    let bundle = bundles
        .iter()
        .find(|b| b.schema == schema)
        .expect("bundle for schema");
    let engine = Engine::default();
    let module = Module::new(&engine, &bundle.wasm).expect("module compiles");
    (engine, module)
}

fn evaluate(engine: &Engine, module: &Module, input_json: &str) -> i32 {
    let mut store = Store::new(engine, ());
    let instance = Instance::new(&mut store, module, &[]).expect("instantiate");
    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("memory export");
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "sealstack_alloc")
        .expect("alloc export");
    let evaluate = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, "sealstack_evaluate")
        .expect("evaluate export");

    let bytes = input_json.as_bytes();
    let ptr = alloc
        .call(&mut store, bytes.len() as i32)
        .expect("alloc call");
    memory
        .write(&mut store, ptr as usize, bytes)
        .expect("write");
    evaluate
        .call(&mut store, (ptr, bytes.len() as i32))
        .expect("evaluate call")
}

#[test]
fn admin_caller_is_allowed() {
    let src = r#"
        schema Doc {
            id:    Ulid   @primary
            owner: String
            policy { read: has_role(caller, "admin") }
        }
    "#;
    let (engine, module) = compile_and_instantiate(src, "Doc");
    let input = r#"{"namespace":"default","schema":"Doc","action":"read","caller":{"id":"u1","email":"a@b","groups":[],"team":"","tenant":"","roles":["admin"],"attrs":{}},"record":{"id":"r1","owner":"u2","tenant":""}}"#;
    assert_eq!(evaluate(&engine, &module, input), 1);
}

#[test]
fn non_admin_is_denied_on_admin_only_read() {
    let src = r#"
        schema Doc {
            id:    Ulid   @primary
            policy { read: has_role(caller, "admin") }
        }
    "#;
    let (engine, module) = compile_and_instantiate(src, "Doc");
    let input = r#"{"namespace":"default","schema":"Doc","action":"read","caller":{"id":"u1","email":"a@b","groups":[],"team":"","tenant":"","roles":["user"],"attrs":{}},"record":{"id":"r1","tenant":""}}"#;
    assert_eq!(evaluate(&engine, &module, input), 0);
}

#[test]
fn empty_policy_block_denies_all_actions() {
    let src = r#"
        schema Locked {
            id: Ulid @primary
            policy { }
        }
    "#;
    let (engine, module) = compile_and_instantiate(src, "Locked");
    for action in ["read", "list", "write", "delete"] {
        let input = format!(
            r#"{{"namespace":"default","schema":"Locked","action":"{action}","caller":{{"id":"u","email":"","groups":[],"team":"","tenant":"","roles":[],"attrs":{{}}}},"record":{{"id":"r","tenant":""}}}}"#,
        );
        assert_eq!(evaluate(&engine, &module, &input), 0, "action {action}");
    }
}

#[test]
fn no_matching_action_row_denies() {
    let src = r#"
        schema ReadOnly {
            id: Ulid @primary
            policy { read: true }
        }
    "#;
    let (engine, module) = compile_and_instantiate(src, "ReadOnly");
    let input = r#"{"namespace":"default","schema":"ReadOnly","action":"write","caller":{"id":"u","email":"","groups":[],"team":"","tenant":"","roles":[],"attrs":{}},"record":{"id":"r","tenant":""}}"#;
    assert_eq!(evaluate(&engine, &module, input), 0);
}

#[test]
fn tampered_magic_returns_negative() {
    let src = r#"
        schema Doc {
            id: Ulid @primary
            policy { read: true }
        }
    "#;
    let file = parser::parse_file(src).unwrap();
    let typed = types::check(&file).unwrap();
    let mut bundle = policy::emit_policy_bundles(&typed).unwrap().remove(0);
    // Find the SLIR magic and flip one byte.
    let pos = bundle
        .wasm
        .windows(4)
        .position(|w| w == b"SLIR")
        .unwrap();
    bundle.wasm[pos + 1] = b'X';

    let engine = Engine::default();
    let module = Module::new(&engine, &bundle.wasm).unwrap();
    let input = r#"{"namespace":"default","schema":"Doc","action":"read","caller":{"id":"u","email":"","groups":[],"team":"","tenant":"","roles":[],"attrs":{}},"record":{"id":"r","tenant":""}}"#;
    assert_eq!(evaluate(&engine, &module, input), -1);
}

#[test]
fn truncated_length_header_returns_negative() {
    let src = r#"
        schema Doc {
            id: Ulid @primary
            policy { read: true }
        }
    "#;
    let file = parser::parse_file(src).unwrap();
    let typed = types::check(&file).unwrap();
    let mut bundle = policy::emit_policy_bundles(&typed).unwrap().remove(0);
    // Find SLIR and overwrite the 4-byte length header with a value that
    // exceeds the data section payload (IR_SECTION_BYTES = 4104 on the
    // wasm side). 0x10000 is large enough to fail the runtime's
    // `declared_len + 8 > ir_full.len()` length check while staying far
    // below u32::MAX (u32::MAX + 8 would wrap on wasm32's 32-bit usize
    // and bypass the check, triggering a slice-index panic — a latent
    // runtime bug out of scope for this test).
    let pos = bundle.wasm.windows(4).position(|w| w == b"SLIR").unwrap();
    bundle.wasm[pos + 4..pos + 8].copy_from_slice(&0x0001_0000u32.to_le_bytes());

    let engine = Engine::default();
    let module = Module::new(&engine, &bundle.wasm).unwrap();
    let input = r#"{"namespace":"default","schema":"Doc","action":"read","caller":{"id":"u","email":"","groups":[],"team":"","tenant":"","roles":[],"attrs":{}},"record":{"id":"r","tenant":""}}"#;
    assert_eq!(evaluate(&engine, &module, input), -1);
}

#[test]
fn missing_caller_attribute_compared_to_string_returns_false() {
    // caller.attrs.region is absent; policy compares it to "us-east-1".
    // Expected: Null == Str returns false, which yields deny (not -1 error).
    let src = r#"
        schema Doc {
            id: Ulid @primary
            policy { read: caller.region == "us-east-1" }
        }
    "#;
    let file = parser::parse_file(src).unwrap();
    let typed = types::check(&file).unwrap();
    let bundles = policy::emit_policy_bundles(&typed).unwrap();
    let bundle = bundles.iter().find(|b| b.schema == "Doc").unwrap();

    let engine = Engine::default();
    let module = Module::new(&engine, &bundle.wasm).unwrap();
    // Caller has no `region` field.
    let input = r#"{"namespace":"default","schema":"Doc","action":"read","caller":{"id":"u","email":"","groups":[],"team":"","tenant":"","roles":[],"attrs":{}},"record":{"id":"r","tenant":""}}"#;
    assert_eq!(evaluate(&engine, &module, input), 0, "expected deny, got non-zero");
}
