# CSL Codegen — Rust Typed Structs + WASM Policy Bundles

**Date:** 2026-04-21
**Status:** Design, pending implementation
**Target milestone:** Phase 1 — Public OSS launch (unblocks "purely declarative system" claim in launch pitch)
**Scope:** `crates/sealstack-csl` codegen expansion. No runtime ABI changes.

---

## 0. Goal and non-goals

### Goal

Fill the last two holes in the CSL compile path so that `sealstack compile <project>` emits, for every `.csl` input, both:

1. **`out/rust/generated.rs`** — a typed Rust module that app developers include in their Rust code to work with sealstack records as real structs instead of `serde_json::Value`.
2. **`out/policy/<namespace>.<schema>.wasm`** — one self-contained WASM bundle per schema with a `policy { ... }` block, matching the ABI already fixed in [`crates/sealstack-engine/src/policy.rs`](../../../crates/sealstack-engine/src/policy.rs) so the existing `WasmPolicy::load_from_dir` registers them without runtime changes.

After this lands, the `AllowAllPolicy` placeholder is no longer on the critical path — any CSL project with declared policies gets real enforcement.

### Non-goals (explicitly out of scope)

- **TypeScript and Python emit.** `CompileTargets::TYPESCRIPT` / `::PYTHON` remain stubbed. Each target deserves its own design pass; bundling them in shortchanges at least one language.
- **Per-predicate native Rust → WASM compilation** (original approach C). Out for v0.1; the runtime-interpreter approach (B) is strictly sufficient for current predicate complexity.
- **Downgrade SQL migrations.** Still scoped for a later slice.
- **Query builders, DAOs, `TryFrom<serde_json::Value>` helpers** on the generated Rust. Real usage patterns first, then helpers.
- **Typed `Ref<T>` wrappers, typed vector fields.** FKs stay as `String`, vectors stay off the struct entirely.
- **Runtime WASM ABI changes.** The current ABI (`memory`, `sealstack_alloc`, `sealstack_evaluate`, JSON in / i32 out) stays untouched so existing hand-authored bundles and fixtures keep working.

### Success criteria

- `cargo test -p sealstack-csl` covers both new targets with snapshot tests and an end-to-end WASM instantiation test.
- `cargo check` of a freshly-generated `out/rust/generated.rs` (from any supported CSL fixture) succeeds with no warnings under the crate's configured clippy lints.
- An `end_to_end.rs`-style integration test compiles a CSL file with `policy { read: caller.id == self.owner }`, loads the emitted `.wasm` into `WasmPolicy`, and verifies `Allow` / `Deny` verdicts match the predicate's semantics over fixture records.
- `sealstack compile` wall-clock on a 10-schema project stays under 1 second on a laptop (rebuilds should be sub-200ms per the existing perf target).

---

## 1. Existing state (what this builds on)

Already landed in [`crates/sealstack-csl/`](../../../crates/sealstack-csl/):

- Full parser ([`parser.rs`](../../../crates/sealstack-csl/src/parser.rs)), AST ([`ast.rs`](../../../crates/sealstack-csl/src/ast.rs)), type checker ([`types.rs`](../../../crates/sealstack-csl/src/types.rs)).
- SQL DDL emit ([`codegen/sql.rs`](../../../crates/sealstack-csl/src/codegen/sql.rs)).
- MCP tool descriptor emit ([`codegen/mcp.rs`](../../../crates/sealstack-csl/src/codegen/mcp.rs)).
- `SchemaMeta` JSON emit ([`codegen/mod.rs::emit_schemas_meta`](../../../crates/sealstack-csl/src/codegen/mod.rs)).
- Vector store plan emit (YAML, minimal).
- `CompileTargets` bitflags with `RUST`, `TYPESCRIPT`, `PYTHON` defined but stubbed.
- `CompileOutput.rust: String` field already threaded through to the CLI.

Runtime-side (untouched by this spec):

- [`crates/sealstack-engine/src/policy.rs`](../../../crates/sealstack-engine/src/policy.rs) has `WasmPolicy::load_from_dir` scanning a directory for `<ns>.<schema>.wasm` files. The ABI expected per bundle is:
  - `memory` export, minimum 1 page.
  - `sealstack_alloc(n_bytes: i32) -> i32` — bump allocator.
  - `sealstack_evaluate(input_ptr: i32, input_len: i32) -> i32` — `1` = allow, `0` = deny, negative = host reports `EngineError::Backend`.
  - Input bytes are a UTF-8 `serde_json::to_vec(&PolicyInputWire)` of `{ namespace, schema, action, caller, record }`.
  - **No host imports.** The bundle is instantiated with `&[]` as imports (see `WasmPolicyHandle::call`).

CLI ([`crates/sealstack-cli/src/commands/compile.rs`](../../../crates/sealstack-cli/src/commands/compile.rs)) writes outputs under `sql/`, `mcp/`, `vector/`, `schemas/`. It does **not** write `rust/` or `policy/` yet.

---

## 2. Rust typed struct emit

### 2.1 Module entry point

New module `crates/sealstack-csl/src/codegen/rust.rs` with:

```rust
pub fn emit_rust(typed: &TypedFile) -> CslResult<String>;
```

Wired into `codegen::emit` behind `CompileTargets::RUST`, replacing the current stub that emits only `// Rust codegen not yet implemented.`.

### 2.2 Output file shape

A single file containing one `pub mod <namespace>` per compiled namespace plus a top-level doc comment and narrow clippy allows. Flat namespace modules (`acme.crm` → `pub mod acme_crm`), decision rationale documented inline in the module doc so future readers aren't left wondering.

```rust
//! Generated by `sealstack compile`. Do not edit by hand.
//!
//! CSL namespaces with dots are flattened to snake_case modules:
//! `acme.crm` → `acme_crm`. This trades nested-module structure for
//! shorter paths in compile errors consumers will actually read.
//! See `docs/superpowers/specs/2026-04-21-csl-codegen-rust-wasm-design.md` §2.

#![allow(clippy::pedantic, clippy::nursery)]
#![allow(dead_code)]  // consumers may import a subset of the generated types

pub mod acme_crm {
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub enum Tier {
        #[serde(rename = "free")]       Free,
        #[serde(rename = "pro")]        Pro,
        #[serde(rename = "enterprise")] Enterprise,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct Customer {
        pub id:           String,
        pub external_id:  String,
        pub name:         String,
        pub domain:       String,
        pub tier:         Tier,
        pub owner:        String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub health_score: Option<f32>,
        pub summary:      String,
        pub created_at:   String,
        pub tenant:       String,  // required, no serde default — see §2.5
    }

    impl Customer {
        pub const NAMESPACE:   &'static str = "acme.crm";
        pub const SCHEMA:      &'static str = "Customer";
        pub const TABLE:       &'static str = "customer";
        pub const VERSION:     u32          = 2;
        pub const PRIMARY_KEY: &'static str = "id";

        pub const RELATION_TICKETS:   (&'static str, &'static str) = ("Ticket",   "customer");
        pub const RELATION_CONTRACTS: (&'static str, &'static str) = ("Contract", "customer");
        pub const RELATION_NOTES:     (&'static str, &'static str) = ("Note",     "customer");
    }
}
```

**Rationale** for each decision is in §2.3–§2.8 below.

### 2.3 Primitive type mapping

| CSL           | Default Rust     | Rich-types feature (`rich-types`) |
|---------------|------------------|-----------------------------------|
| `String`      | `String`         | `String`                          |
| `Text`        | `String`         | `String`                          |
| `Ulid`        | `String`         | `ulid::Ulid`                      |
| `Uuid`        | `String`         | `uuid::Uuid`                      |
| `I32`         | `i32`            | `i32`                             |
| `I64`         | `i64`            | `i64`                             |
| `F32`         | `f32`            | `f32`                             |
| `F64`         | `f64`            | `f64`                             |
| `Bool`        | `bool`           | `bool`                            |
| `Instant`     | `String`         | `time::OffsetDateTime`            |
| `Duration`    | `String`         | `std::time::Duration`             |
| `Json`        | `serde_json::Value` | `serde_json::Value`            |
| `Vector<N>`   | *skipped*        | *skipped*                         |
| `Ref<T>`      | `String`         | `String` (same — FK semantics don't gain from typing here) |
| `List<T>`     | `Vec<T>`         | `Vec<T>`                          |
| `Map<K,V>`    | Not emitted in v0.1 | (type checker already rejects; this is belt-and-braces) |
| `Named(Enum)` | Rust `enum`      | Rust `enum`                       |
| `Named(Schema)` | `String`       | `String` (reference to another schema's PK) |
| `T?`          | `Option<T>`      | `Option<T>`                       |

The rich-types path is gated by `#[cfg(feature = "rich-types")]` **emitted in the consumer crate**, not in `sealstack-csl` itself. What the emitter does:

- **Default branch** renders `String` for Ulid/Uuid/Instant/Duration.
- **Rich branch** (only included in the generated output when the input schema uses any of those types) emits an additional `#[cfg(feature = "rich-types")]` block with the richly-typed aliases inside the same module. Consumers opt in by enabling the feature in their Cargo.toml.

For v0.1 simplicity we will **emit the default branch only**; rich-types is a future slice. The `String` default is future-compatible because switching a struct field from `String` to a `FromStr`-compatible type in a patch release is a breaking change consumers opt into at their own pace via feature flag.

### 2.4 Enum emit

Each top-level `enum` declaration renders as a Rust enum with `#[serde(rename = "...")]` per variant using the declared wire form (if present) or the lowercased identifier:

```csl
enum Tier { Free("free"), Pro, Enterprise }
```

→

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Tier {
    #[serde(rename = "free")]        Free,
    #[serde(rename = "pro")]         Pro,       // no explicit wire → lowercased
    #[serde(rename = "enterprise")]  Enterprise,
}
```

Enums emitted at module scope so they can be referenced by multiple schemas in the same namespace without duplication.

### 2.5 The tenant field

Every CSL-generated table carries a `tenant` column (see [`codegen/sql.rs`](../../../crates/sealstack-csl/src/codegen/sql.rs) — `"  tenant text NOT NULL DEFAULT ''"`). The Rust struct mirrors this: `pub tenant: String`, **no `#[serde(default)]`**.

This is a deliberate tightening versus the earlier draft. A JSON payload missing the `tenant` field now fails to deserialize, which is correct behavior for a security-relevant multi-tenant isolation key. The SQL default of empty string means rows written by older pipelines still deserialize (they have `tenant: ""`), but any caller synthesizing a record in code must explicitly set the field.

Platform-level schemas that truly have no tenant can be handled later by adding a `@cross_tenant` schema-level decorator that switches the emitted field to `Option<String>`. Not in scope here.

### 2.6 Relations

Many-relations (`relation { tickets: many Ticket via Ticket.customer ... }`) are **not** emitted as struct fields — they're virtual back-pointers resolved through foreign keys. They render as `impl`-scope associated constants so consumers can introspect without hand-duplicating strings:

```rust
impl Customer {
    pub const RELATION_TICKETS: (&'static str, &'static str) = ("Ticket", "customer");
}
```

Tuple shape is `(target_schema_name, foreign_key_field)`. If a many-relation names a schema in a different namespace (e.g., `many hr.Employee via ...`), the target is stored as `"hr.Employee"` and the consumer resolves it.

One-relations (if they exist in the AST — currently rare) render the same way; the `Ref<T>` field that backs them already appears on the struct as a `String` FK.

### 2.7 Associated constants per struct

Five per-schema constants, symmetrical and reflection-friendly:

```rust
pub const NAMESPACE:   &'static str = "acme.crm";
pub const SCHEMA:      &'static str = "Customer";
pub const TABLE:       &'static str = "customer";
pub const VERSION:     u32          = 2;
pub const PRIMARY_KEY: &'static str = "id";
```

`PRIMARY_KEY` is the fifth piece reflection-style consumers need (for building URLs, generic CRUD, etc.). The others mirror what's already emitted in `SchemaMeta`.

### 2.8 Clippy / lint allows

Two narrow `#![allow(...)]` lines at file scope, deliberately avoiding `clippy::all`:

```rust
#![allow(clippy::pedantic, clippy::nursery)]
#![allow(dead_code)]
```

Rationale:

- `clippy::pedantic` and `clippy::nursery` are the two noisy groups that generated code commonly trips (`module_name_repetitions`, `missing_errors_doc`, `needless_pass_by_value`, etc.). Suppressing them wholesale is acceptable; suppressing `clippy::all` would also hide genuinely useful lints from future clippy releases.
- `dead_code` is needed because consumers can legitimately import a subset of the generated types. Without this, an unused `Customer` struct in a user's crate would warn through no fault of theirs.
- `non_snake_case` is **not** suppressed. CSL's lexical rules already require field identifiers to begin with a lowercase letter (§1 of the spec), and the SQL emitter snake-cases them explicitly. If a non-snake name slips through, the lint firing is a real signal of an inconsistency worth fixing at the source, not silencing here.
- `clippy::struct_field_names` is **not** suppressed at file level. If it ever fires on generated code (e.g., `customer_id` inside a `Customer` struct), we add it per-struct. Defensive file-wide suppression hides real code smell in user code if users ever include the generated file via `include!`.

### 2.9 Ordering and determinism

- Enums emit before schemas within a namespace.
- Schemas emit in `typed.decl_order` (source order — already how `schemas_meta` does it).
- Namespaces emit in sorted order by collapsed module name, to guarantee byte-identical output for unchanged input and make snapshot tests stable.

---

## 3. WASM policy bundle emit

### 3.1 Approach — precompiled runtime + data-section patching (approach B)

One-time-built interpreter. At `sealstack compile` time, each schema's predicate compiles to a compact bytecode IR, the IR bytes are stamped into a data section of a clone of `policy_runtime.wasm`, and the result is written out as `<namespace>.<schema>.wasm`.

**Why this over alternatives** (recap from brainstorming):

- Approach A (pure WAT with embedded JSON parser) — infeasible to debug at v0.1 scope.
- Approach C (per-bundle cargo build) — requires `wasm32-unknown-unknown` toolchain on every machine running `sealstack compile`. Breaks the "one command compiles everything" promise.

**Bundle size** is acceptable: `serde_json` + a ~1 KB predicate IR is ~60–80 KB per bundle. A project with 20 schemas stays under 2 MB total, well inside acceptable footprint for a policy dir.

### 3.2 New crate: `sealstack-policy-runtime`

Location: `crates/sealstack-policy-runtime/`. A small Rust crate targeting `wasm32-unknown-unknown`, compiled once per release and the resulting `.wasm` checked into `sealstack-csl/assets/policy_runtime.wasm`.

**Responsibilities:**

1. Provide the three required exports (`memory`, `sealstack_alloc`, `sealstack_evaluate`) with the exact ABI signatures the runtime expects.
2. Bump-allocate a single input buffer inside linear memory (no `alloc` crate — the bump allocator is ~30 lines and avoids pulling in `wee_alloc` or similar).
3. At `sealstack_evaluate` entry, deserialize the input bytes as `PolicyInputWire` using `serde_json::from_slice` in no-std mode.
4. Walk the predicate IR (embedded in a linker-marked data section) against the `caller` and `record` JSON values.
5. Return `1` for allow, `0` for deny, `-1` for any internal error (malformed IR, bad JSON, depth exceeded).

**Data section contract.** The runtime references the IR via a `#[link_section = ".sealstack_predicate_ir"]` static:

```rust
#[used]
#[link_section = ".sealstack_predicate_ir"]
pub static PREDICATE_IR: [u8; IR_MAX_BYTES] = [0; IR_MAX_BYTES];
```

`IR_MAX_BYTES` is a compile-time upper bound (e.g., 4096). The first 4 bytes hold the actual IR length (little-endian u32); the rest is the serialized IR. The emitter patches both length and bytes at codegen time.

**Build pipeline:**

```
scripts/rebuild-policy-runtime.sh
  → cargo build -p sealstack-policy-runtime
      --target wasm32-unknown-unknown --release
  → cp target/wasm32-unknown-unknown/release/sealstack_policy_runtime.wasm
      crates/sealstack-csl/assets/policy_runtime.wasm
  → cargo test -p sealstack-csl wasm_policy_roundtrip
```

CI adds a job that runs the script and fails if the checked-in asset diverges, so a contributor can't accidentally land a runtime change without refreshing the asset. Running the script locally requires `rustup target add wasm32-unknown-unknown`; a `CONTRIBUTING.md` note describes this.

### 3.3 Predicate IR design

A tree of opcodes, post-order serialization, compact enough to stay well under the `IR_MAX_BYTES` cap.

**Opcodes (u8 tag, followed by payload):**

| Opcode | Name               | Payload                                        | Stack effect                        |
|--------|--------------------|------------------------------------------------|-------------------------------------|
| 0x01   | `LIT_NULL`         | —                                              | push Null                           |
| 0x02   | `LIT_BOOL`         | u8 (0/1)                                       | push Bool                           |
| 0x03   | `LIT_I64`          | i64 LE                                         | push I64                            |
| 0x04   | `LIT_F64`          | f64 LE                                         | push F64                            |
| 0x05   | `LIT_STR`          | u16 len LE + UTF-8 bytes                       | push Str                            |
| 0x06   | `LIT_DURATION_SECS`| i64 LE (seconds)                               | push I64                            |
| 0x10   | `LOAD_CALLER`      | u16 len + JSON-pointer segments as UTF-8       | push value or Null                  |
| 0x11   | `LOAD_SELF`        | u16 len + JSON-pointer segments as UTF-8       | push value or Null                  |
| 0x20   | `EQ`               | —                                              | pop2, push Bool                     |
| 0x21   | `NE`               | —                                              | pop2, push Bool                     |
| 0x22   | `LT`               | —                                              | pop2, push Bool                     |
| 0x23   | `LE`               | —                                              | pop2, push Bool                     |
| 0x24   | `GT`               | —                                              | pop2, push Bool                     |
| 0x25   | `GE`               | —                                              | pop2, push Bool                     |
| 0x30   | `AND`              | —                                              | pop2, short-circuit                 |
| 0x31   | `OR`               | —                                              | pop2, short-circuit                 |
| 0x32   | `NOT`              | —                                              | pop1, push Bool                     |
| 0x40   | `IN`               | —                                              | pop2 (value, list), push Bool       |
| 0x41   | `NOT_IN`           | —                                              | pop2, push Bool                     |
| 0x50   | `CALL_HAS_ROLE`    | —                                              | pop2 (caller, role_string)          |
| 0x51   | `CALL_TENANT_MATCH`| —                                              | pop2 (caller, self), push Bool      |
| 0x52   | `CALL_NOW`         | —                                              | push I64 (unix seconds)             |
| 0xF0   | `BRANCH_ACTION`    | u8 action_mask + u16 offset-if-mismatch        | conditional skip for per-action rules |
| 0xFE   | `DENY`             | —                                              | terminal; yields 0                  |
| 0xFF   | `ALLOW`            | —                                              | terminal; yields 1                  |

The IR is a single flat byte stream, not per-action. The top-level structure is:

```
<action_table_count: u8>
{ <action_mask: u8> <offset_to_rule_start: u16> }* action_table_count
<rule bytecode, concatenated>
```

The runtime at evaluation time reads the requested action from the input, scans the action table to find the matching rule, and begins interpreting at that offset. If no rule matches, the verdict is deny (default deny for unhandled actions — matches spec §6 totality expectation).

**Values at runtime** are a tagged Rust enum:

```rust
enum Val<'a> {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    Str(&'a str),           // borrows from the input JSON
    List(&'a [serde_json::Value]),
}
```

Borrowed-str keeps allocations out of the hot path; the whole evaluation runs over the parsed `serde_json::Value` without copying strings.

**JSON-pointer resolution** (opcodes `LOAD_CALLER`, `LOAD_SELF`) walks a segment list (e.g., `["owner", "team"]`) down a `serde_json::Value`. Missing segments yield `Null`. The AST type checker has already validated depth ≤ 4 per spec §6.3, so no runtime depth guard is needed.

**Comparison coercion** is minimal and explicit: two values are comparable only if their types match exactly (I64 vs I64, Str vs Str, etc.). Mixed-type comparison yields `false` for `EQ` and errors (return `-1`) for `LT`/`LE`/`GT`/`GE`. This matches the CSL type checker's expectations — at compile time, any comparison that mixes types will have been rejected, so a runtime mismatch indicates a bug or tampered IR.

### 3.4 Compiler changes — AST → IR

New module `crates/sealstack-csl/src/codegen/policy.rs`.

```rust
pub struct PolicyBundle {
    pub namespace: String,
    pub schema:    String,
    pub wasm:      Vec<u8>,
}

pub fn emit_policy_bundles(typed: &TypedFile) -> CslResult<Vec<PolicyBundle>>;
```

Per schema with a `policy { ... }` block:

1. **Lower AST → IR.** Walk the `PolicyBlock::rules`, emit an action-dispatch table, then post-order-emit each rule's `Expr` into the bytecode stream.
2. **Validate.** Assert every path in `LOAD_CALLER` / `LOAD_SELF` matches the type-checked symbol table. Assert every `CALL_*` is a registered built-in. Cap total IR length at 4 KiB (assert before patching).
3. **Patch.** Load `policy_runtime.wasm` (via `include_bytes!`) into a `wasm-encoder::Module` representation, locate the `.sealstack_predicate_ir` data segment by name, replace its initializer with `<len: u32 LE> <ir bytes> <zero padding to IR_MAX_BYTES>`.
4. **Serialize** the patched module, push onto the return `Vec<PolicyBundle>`.

**Dependencies added** to `sealstack-csl/Cargo.toml`:

- `wasmparser = "0.220"` — parse the runtime asset at patch time.
- `wasm-encoder = "0.220"` — re-encode after patching.
- `wat = "1"` — promote from dev-dep to regular dep so tests and production share the same version.

### 3.5 Runtime-side (no changes)

The existing `WasmPolicy::load_from_dir` already:

- Walks a directory for `*.wasm`.
- Parses filenames as `<namespace>.<schema>.wasm`.
- Instantiates via `wasmtime` with empty imports.
- Calls `sealstack_evaluate` via the documented ABI.

Zero runtime changes are required. The only thing that changes is the source of `.wasm` files: previously they came from hand-written WAT in tests; now `sealstack compile` emits them.

### 3.6 New CompileTargets flag + output field

```rust
bitflags! {
    pub struct CompileTargets: u32 {
        const SQL         = 0b0000_0001;
        const RUST        = 0b0000_0010;
        const MCP         = 0b0000_0100;
        const VECTOR_PLAN = 0b0000_1000;
        const TYPESCRIPT  = 0b0001_0000;
        const PYTHON      = 0b0010_0000;
        const WASM_POLICY = 0b0100_0000;  // NEW
    }
}

pub struct CompileOutput {
    // ... existing fields ...
    pub policy_bundles: Vec<PolicyBundle>,  // NEW
}
```

`CompileTargets::all()` includes `WASM_POLICY` automatically via `bitflags`'s generated `all` method.

### 3.7 CLI changes

[`crates/sealstack-cli/src/commands/compile.rs::write_outputs`](../../../crates/sealstack-cli/src/commands/compile.rs) gains:

```rust
if !out.rust.is_empty() && !out.rust.starts_with("// Rust codegen not yet implemented") {
    let rust_dir = output_dir.join("rust");
    std::fs::create_dir_all(&rust_dir)?;
    std::fs::write(rust_dir.join("generated.rs"), &out.rust)?;
}

if !out.policy_bundles.is_empty() {
    let policy_dir = output_dir.join("policy");
    std::fs::create_dir_all(&policy_dir)?;
    for bundle in &out.policy_bundles {
        let name = format!("{}.{}.wasm", bundle.namespace, bundle.schema);
        std::fs::write(policy_dir.join(name), &bundle.wasm)?;
    }
}
```

The stub-check on `out.rust` guards against accidentally overwriting a user's vendored `generated.rs` when a target hasn't been implemented yet. Once the stub is gone, the check reduces to `!out.rust.is_empty()`.

---

## 4. Testing plan

Four tiers, from fastest to slowest:

### 4.1 Snapshot tests (`insta`)

Extend `tests/fixtures/` with two new fixtures:

- `tests/fixtures/hello.csl` (already present) — baseline, exercises primitives, enums, indexes, relations.
- `tests/fixtures/with_policy.csl` — new, exercises `policy { read/write/list/delete }`, references, `has_role` / `tenant_match`, `in` operator, traversal through `Ref`.

For each fixture:

- `tests/rust_emit_snapshot.rs` — parse + type-check + `emit_rust`, assert the output matches a checked-in snapshot (`insta::assert_snapshot!`).
- `tests/ir_snapshot.rs` — lower the policy AST to IR, hex-dump, snapshot-assert the bytecode sequence. This catches accidental opcode reordering or payload layout drift.

### 4.2 Generated-code `cargo check`

`tests/rust_emit_compiles.rs` — write the generated Rust to a tempdir, populate a minimal `Cargo.toml` with `serde = { version = "1", features = ["derive"] }` + `serde_json = "1"`, run `cargo check --quiet`, assert exit 0 and zero stderr under the crate's configured clippy lints.

This catches issues no snapshot can: unresolved identifiers, missing derives, trait-bound inconsistencies.

### 4.3 WASM round-trip

`tests/wasm_policy_roundtrip.rs` (gated on the `wasm-policy` feature in `sealstack-engine`):

1. Compile `with_policy.csl` → emit policy bundles.
2. Write each bundle to a tempdir as `<ns>.<schema>.wasm`.
3. Load via `WasmPolicy::load_from_dir` (using the same runtime code the gateway uses).
4. For a table of (caller, record, action, expected_verdict) fixtures, call `policy.evaluate(...)` and assert the verdict.

Cases to cover:

- Admin caller allowed on all actions.
- Non-admin allowed on `read` by team match; denied on `write`.
- `list` vs `read` divergence on the same record.
- `caller.id in self.notes[*].shared_with` — exercises list-traversal.
- Missing caller attribute (`caller.attrs["region"]` absent) — evaluates to Null-compared-to-Str → false, not a trap.
- Tampered IR (truncated length header) — evaluate returns `-1`, host surfaces `EngineError::Backend`.

### 4.4 Integration with existing `end_to_end.rs`

The existing gateway integration test registers schemas and runs a query. Extend it (behind the same `SEALSTACK_DATABASE_URL` opt-in) to:

- Compile a CSL source with a policy.
- Load the emitted bundle into the gateway's `WasmPolicy`.
- Issue a `/v1/query` as an admin caller (expect results).
- Issue the same query as a non-matching caller (expect filtered results).

This asserts the full happy path end-to-end, but remains `#[ignore]`-gated per the current project convention.

---

## 5. Rollout and compatibility

- **Public API stability:** `CompileTargets`, `CompileOutput`, and `compile()` gain one new flag and one new field. No existing fields change shape. Any consumer that pattern-matches on `CompileOutput` exhaustively will need a rebuild; the field is `Vec<PolicyBundle>` so destructuring with `..` stays forward-compatible.
- **Runtime compatibility:** zero changes to the engine/gateway. Existing hand-authored test bundles continue to load identically.
- **CLI behavior:** users running `sealstack compile` start getting `out/rust/` and `out/policy/` directories. Existing `out/` contents are untouched.
- **Release note:** CHANGELOG entry under `## Unreleased`:
  > `sealstack compile` now emits typed Rust structs (`out/rust/generated.rs`) and WASM policy bundles (`out/policy/*.wasm`) from CSL `schema` and `policy { ... }` blocks. Bundles drop straight into a directory configured via `SEALSTACK_POLICY_DIR` for the gateway's `WasmPolicy` to load.

---

## 6. Risks and mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `policy_runtime.wasm` bundle size grows unacceptably with `serde_json` | Medium | Measure after first build; if over 100 KB, swap to `miniserde` or a hand-rolled JSON parser for just the input shape. |
| Data-section patching surprises us (wasm-encoder re-serialization reorders sections) | Low | Test-driven: the first roundtrip test validates the patched binary is identical to the source except for the one data segment. |
| CI drift where `policy_runtime.wasm` asset doesn't match source | Medium | CI job runs `scripts/rebuild-policy-runtime.sh` and fails if the asset differs from what's checked in. |
| 4 KiB IR cap too small for realistic policies | Low | Spec §6.3 caps `Ref` traversal at depth 4, and real CSL policies are short. If a real policy hits the cap, bump to 16 KiB — same data-section template, one constant change. |
| Generated Rust fails to compile under some combination of derives + user's workspace lints | Medium | §4.2 cargo-check test catches this before merge. The file opts out of `clippy::pedantic`/`nursery` which covers the common cases; workspace-specific lints remain the consumer's problem to allow. |
| Non-admin callers with the wrong caller shape cause policy evaluation errors at runtime instead of denials | Medium | Runtime `-1` return for internal errors is already surfaced as `EngineError::Backend` — the gateway does not silently allow on error. Audit check-in. |

---

## 7. Open questions / deferred

These are known-unknown and intentionally deferred past this slice:

1. **Rich-types feature wiring.** How exactly the `rich-types` feature gets forwarded to consumer crates is a UX detail — probably a `generated.rs`-local `#[cfg(feature = "rich-types")]` branch that a consumer enables via their own Cargo.toml feature. Punt until someone asks.
2. **Per-field redaction in the emitted Rust.** `@redact(policy)` is specced but neither SQL nor Rust emit honor it today. The right shape is probably a `redacted_view(&self, caller: &Caller) -> Self` helper generated alongside the struct. Separate slice.
3. **Downgrade migrations.** Already deferred repo-wide.
4. **Vendored `generated.rs` vs build-time emit.** Whether users commit `generated.rs` to their repo or regenerate on every `cargo build` via a `build.rs` is a usage pattern question. Document both in a follow-up; default to committed for the v0.1 launch.
5. **Policy hot-reload.** The runtime reads bundles at boot; a deployment that updates policies needs a restart. SIGHUP-to-reload is a small change to `WasmPolicy` but out of scope here.

---

## 8. Summary

- **Two new codegen paths** in `sealstack-csl`, one flag each, one small crate for the shared WASM runtime.
- **Zero runtime changes.**
- **Testing is in four tiers**: snapshot, cargo-check of emitted Rust, WASM round-trip, gateway integration.
- **~1 week of work** on a single session with no forked dependencies; the biggest risk is the `serde_json`-in-wasm bundle size, which we measure and swap if unacceptable.

After this lands, `AllowAllPolicy` stops being the de-facto production choice — any CSL project with `policy { ... }` blocks gets real enforcement from `sealstack compile` with no manual WAT authoring.
