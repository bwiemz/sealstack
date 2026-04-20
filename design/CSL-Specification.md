# Context Schema Language (CSL) — Specification

**Version**: 0.1.0-draft
**Status**: Design proposal
**Target compiler**: `signet-csl` (Rust, winnow parser)

---

## 0. Design Principles

CSL is a typed schema language for enterprise context. It sits in the same abstraction slot as Prisma (DB schema), GraphQL SDL (API shape), and LookML (semantic layer) — but its output is a *running context runtime*, not a database or an API.

1. **Types first.** Every context item has a declared shape. Untyped context is the source of 80% of hallucination failure modes.
2. **Permissions are predicates, not annotations.** Access control is a compilable predicate attached to fields and entities; it runs at retrieval time, before the LLM sees anything.
3. **Context behavior is declarative.** Chunking, embedding, freshness, and retrieval weighting are declared alongside the schema, not hardcoded in Python glue.
4. **One source, many targets.** A CSL file compiles to SQL migrations, vector-store collections, MCP tool descriptors, TypeScript/Rust/Python types, and lineage graphs.
5. **Migrations are first-class.** Schema versions are tracked, diffs produce migration scripts, and downgrades are explicit.
6. **No runtime magic.** Everything is statically analyzable. If the compiler accepts it, the runtime has no ambiguity.

Naming inspiration: Prisma for the relational ergonomics, LookML for the semantic-layer pattern, GraphQL SDL for the type-first presentation, and NeuralScript's `@` decorator style for attribute metadata.

---

## 1. Lexical Structure

```
whitespace   = " " | "\t" | "\r" | "\n"
comment      = "//" ..EOL  |  "/*" ... "*/"
ident        = /[A-Za-z_][A-Za-z0-9_]*/
type_ident   = /[A-Z][A-Za-z0-9_]*/
path         = ident ("." ident)*
integer      = /-?[0-9]+/
float        = /-?[0-9]+\.[0-9]+/
string       = /"([^"\\]|\\.)*"/
duration     = integer ("ns"|"us"|"ms"|"s"|"m"|"h"|"d"|"w"|"mo"|"y")
keyword      = "schema"|"entity"|"relation"|"enum"|"policy"|"context"
             | "import"|"namespace"|"version"|"from"|"via"|"as"
             | "one"|"many"|"optional"|"required"
             | "true"|"false"|"null"
             | "and"|"or"|"not"|"in"
```

Identifiers are case-sensitive. Type identifiers must begin with an uppercase letter. Field identifiers, relation names, and decorator names must begin with lowercase. This matches the NeuralScript convention and eliminates a class of typo bugs.

---

## 2. Grammar (EBNF)

```ebnf
file            = namespace_decl? import_stmt* top_decl* ;

namespace_decl  = "namespace" path ";" ;
import_stmt     = "import" string ("as" ident)? ";" ;

top_decl        = schema_decl
                | enum_decl
                | policy_decl
                | context_profile_decl ;

schema_decl     = "schema" type_ident ("version" integer)? "{"
                     field_decl*
                     relation_block?
                     context_block?
                     policy_block?
                  "}" ;

field_decl      = ident ":" type_expr decorator* ";"? ;

type_expr       = primitive
                | "Ref" "<" type_ident ">"
                | "List" "<" type_expr ">"
                | "Map" "<" type_expr "," type_expr ">"
                | type_ident
                | type_expr "?"                       (* optional *) ;

primitive       = "String" | "Text" | "Ulid" | "Uuid"
                | "I32" | "I64" | "F32" | "F64"
                | "Bool" | "Instant" | "Duration"
                | "Json" | "Vector" "<" integer ">" ;

decorator       = "@" path ( "(" arg_list? ")" )? ( "=" expr )? ;
arg_list        = expr ("," expr)* ;

relation_block  = "relations" "{" relation_decl* "}" ;
relation_decl   = ident ":" ("one"|"many") type_ident
                    "via" path ("on_delete" delete_policy)? ";"? ;
delete_policy   = "cascade" | "restrict" | "set_null" ;

context_block   = "context" "{" context_stmt* "}" ;
context_stmt    = ident "=" expr ";"? ;

policy_block    = "policy" "{" policy_rule* "}" ;
policy_rule     = (ident ":")? action_list "=" expr ";"? ;
action_list     = action ("|" action)* ;
action          = "read" | "list" | "write" | "delete" ;

enum_decl       = "enum" type_ident "{" enum_variant ("," enum_variant)* ","? "}" ";"? ;
enum_variant    = ident ( "(" string ")" )? ;

policy_decl     = "policy" ident "{" policy_rule* "}" ;      (* named, reusable *)
context_profile_decl = "context" "profile" ident "{" context_stmt* "}" ;

expr            = literal
                | path
                | unary_op expr
                | expr binary_op expr
                | "(" expr ")"
                | func_call
                | "[" expr_list? "]"           (* list literal *) ;
func_call       = path "(" expr_list? ")" ;
expr_list       = expr ("," expr)* ;
literal         = integer | float | string | duration | "true" | "false" | "null" ;
binary_op       = "==" | "!=" | "<" | "<=" | ">" | ">="
                | "and" | "or" | "in" | "not" "in"
                | "+" | "-" | "*" | "/" ;
unary_op        = "not" | "-" ;
```

---

## 3. Type System

### 3.1 Primitive types

| CSL type      | SQL type          | Rust type         | TS type    | Notes |
|---------------|-------------------|-------------------|------------|-------|
| `String`      | `text`            | `String`          | `string`   | Indexed; default 1 KiB limit |
| `Text`        | `text`            | `String`          | `string`   | Large free text; chunkable |
| `Ulid`        | `bytea(16)`       | `ulid::Ulid`      | `string`   | Preferred primary key |
| `Uuid`        | `uuid`            | `uuid::Uuid`      | `string`   |       |
| `I32` / `I64` | `int4` / `int8`   | `i32` / `i64`     | `number`   |       |
| `F32` / `F64` | `real` / `double` | `f32` / `f64`     | `number`   |       |
| `Bool`        | `boolean`         | `bool`            | `boolean`  |       |
| `Instant`     | `timestamptz`     | `OffsetDateTime`  | `string`   | UTC, nanosecond precision |
| `Duration`    | `interval`        | `std::time::Duration` | `string` |     |
| `Json`        | `jsonb`           | `serde_json::Value` | `unknown`|       |
| `Vector<N>`   | vector store     | `[f32; N]`        | `number[]` | N is embedding dim |

### 3.2 Optionality

Every type is non-nullable by default. `?` makes it optional. This matches Rust semantics (`Option<T>`) and inverts SQL's default to a safer one.

### 3.3 References and relations

`Ref<T>` is a foreign key to an entity of type `T`. In the generated SQL, it becomes the appropriate ID column plus a foreign-key constraint. In the retrieval layer, it becomes a graph edge.

Relations expressed via `relation { ... }` are bidirectional convenience declarations. `many` relations do not create physical columns on the owning side — they are resolved via the `via` reverse path.

### 3.4 Soundness rules

1. Every schema must have exactly one field annotated `@primary`.
2. Every `Ref<T>` field's target type must resolve in the current namespace or an imported one.
3. Cyclic `Ref` chains are allowed; cyclic `via` chains are rejected at compile time.
4. Every decorator is resolved to a known attribute or compilation fails. No silent ignores.
5. Permission predicates must type-check against the symbol table of the enclosing schema plus the built-in `caller` context.
6. `Vector<N>` fields require an `@embedded_from` decorator pointing to a `Text` field or a computed expression; the compiler enforces this.

---

## 4. Built-in Decorators

### 4.1 Field decorators

| Decorator                  | Target           | Meaning |
|----------------------------|------------------|---------|
| `@primary`                 | scalar field     | Primary key. Exactly one per schema. |
| `@searchable`              | String/Text      | Include in BM25 index. |
| `@facet`                   | scalar           | Expose as a filter facet in retrieval. |
| `@indexed`                 | scalar           | Add a DB index for exact-match queries. |
| `@unique`                  | scalar           | Unique constraint. |
| `@default(value)`          | scalar           | Default value literal. |
| `@computed("pipeline.id")` | any              | Populated by a named compute pipeline, not written directly. |
| `@embedded_from(field)`    | Vector\<N\>        | Source field for embedding generation. |
| `@chunked`                 | Text             | This field is chunked per the schema's `context` block. |
| `@pii(kind)`               | String/Text      | Marks PII; kind ∈ `"email"|"phone"|"ssn"|"name"|"address"|"custom"`. |
| `@redact(policy)`          | any              | Apply redaction policy on read for non-authorized callers. |
| `@lineage`                 | any              | Always include lineage metadata for this field in receipts. |
| `@deprecated("reason")`    | any              | Emits a warning if queried; removable in next major version. |
| `@boost(factor)`           | scalar           | Retrieval score multiplier for matches on this field. |
| `@language(expr)`          | Text             | Dynamic language detection; drives tokenizer selection. |

### 4.2 Schema-level decorators

| Decorator                     | Meaning |
|-------------------------------|---------|
| `@tenant_scope(key)`          | Logical tenant isolation key. Default: inherits from namespace. |
| `@retention(duration)`        | Max age; older rows are archived or deleted per policy. |
| `@soft_delete`                | Uses `deleted_at` column rather than hard delete. |
| `@audit`                      | Emits audit events for every create/update/delete. |

### 4.3 Permission shorthand decorators

Shortcuts that desugar to `policy { ... }` blocks:

```csl
owner: Ref<User> @permission.read = (caller.id == self.owner.id)
```

is equivalent to:

```csl
policy {
  read: (caller.id == self.owner.id);
}
```

on that field. Shorthand forms: `@permission.read`, `@permission.write`, `@permission.list`, `@permission.delete`.

---

## 5. The `context { }` Block

This is the feature no other schema language has. It colocates retrieval and embedding behavior with the entity definition.

```csl
context {
  // Chunking
  chunking        = semantic(max_tokens = 512, overlap = 64)
  // or: fixed(size = 1024)
  //     recursive(split = ["\n\n", "\n", ". ", " "], max_tokens = 800)
  //     layout                    // for PDFs
  //     ast(language = "rust")    // for source code

  // Embedding
  embedder        = "voyage-3"
  embedder_batch  = 128

  // Vector store
  vector_dims     = 1024
  index           = hnsw(m = 32, ef_construction = 200)

  // Retrieval
  hybrid_alpha    = 0.6          // weight on dense vs sparse (1.0 = dense only)
  reranker        = "bge-reranker-v2"
  default_top_k   = 16

  // Freshness
  freshness_decay = exponential(half_life = 30d)
  // or: linear(window = 90d)
  //     step(cliffs = [7d, 30d, 180d], factors = [1.0, 0.8, 0.4])
  //     none

  // Policy hooks
  pre_retrieval   = [filter_permissions, filter_tenant]
  post_retrieval  = [redact_pii, sign_receipt]
}
```

All values are compile-time constants and statically checked against a small list of strategy names. Adding a new strategy means registering it in the Rust runtime and updating the CSL standard library of strategy names — nothing in user schemas changes.

Shared context profiles keep schemas DRY:

```csl
context profile Documents {
  chunking        = semantic(max_tokens = 512, overlap = 64)
  embedder        = "voyage-3"
  freshness_decay = exponential(half_life = 60d)
}

schema Wiki {
  // ...fields...
  context { use = Documents; reranker = "bge-reranker-v2"; }
}
```

Explicit values in a schema's `context` block override the profile.

---

## 6. Permissions — The Predicate Language

Permissions compile to WASM predicates that run in the policy engine.

### 6.1 Available bindings

- `self` — the entity being evaluated. All its fields and Refs are reachable.
- `caller` — the authenticated principal. Fields: `id`, `email`, `groups: List<String>`, `team`, `tenant`, `roles: List<String>`, `attrs: Map<String, Json>`.
- `now()` — current UTC instant.
- `tenant` — shorthand for `caller.tenant`.

### 6.2 Operators

Comparison: `== != < <= > >=`
Logical: `and or not`
Set: `in`, `not in` (RHS must be a `List<T>`)
Arithmetic: `+ - * /` (numeric only)
Member access: `self.owner.team`, `caller.attrs["region"]`
Function call: `now()`, `has_role(caller, "admin")`, `tenant_match(caller, self)`

### 6.3 Reference traversal

Predicates may traverse `Ref` chains up to depth 4 at compile time. Chains beyond that must be expressed as named, memoized helpers (registered in the runtime's auxiliary symbol table). This cap prevents pathological retrieval-time graph walks.

### 6.4 Example

```csl
schema Ticket {
  id:          Ulid    @primary
  title:       String  @searchable
  body:        Text    @chunked
  customer:    Ref<Customer>
  assignee:    Ref<User>?
  priority:    Enum("low","medium","high","critical") @facet
  created_at:  Instant

  policy {
    // Anyone on the owning team, the assignee, or an admin can read
    read:   has_role(caller, "admin")
         or caller.id == self.assignee?.id
         or caller.team == self.customer.owner.team

    // Only assignees and admins can write
    write:  has_role(caller, "admin")
         or caller.id == self.assignee?.id

    // Everyone on the tenant can see that tickets exist (list), but body hidden
    list:   tenant_match(caller, self)
  }
}
```

The compiler checks:
- `self.assignee?.id` is well-typed (safe navigation on the optional Ref).
- `self.customer.owner.team` resolves (3-deep, within the depth-4 limit).
- `has_role` and `tenant_match` are registered built-ins.

### 6.5 Field-level redaction

`@redact(policy)` applies to the returned value rather than filtering the record entirely:

```csl
email: String @pii("email") @redact(mask_except_role("admin"))
```

Non-admins see `j***@company.com`; admins see the full value. The record is still listable.

---

## 7. Computed Fields and Pipelines

```csl
schema Customer {
  id:           Ulid   @primary
  name:         String
  health_score: F32    @computed("analytics.customer_health")
  summary:      Text   @computed("llm.summarize", inputs = ["tickets", "notes"])
}
```

Named pipelines are registered in the runtime. The compiler verifies the pipeline exists and that its output type matches the field's declared type. Computed fields are read-only from external writers.

---

## 8. Enums

```csl
enum Tier {
  Free("free"),
  Pro("pro"),
  Enterprise("enterprise")
}
```

The string form in parens is the wire representation; the identifier is the Rust/TS/SQL symbol. If omitted, the lowercased identifier is used.

---

## 9. Versioning and Migrations

Schemas carry a monotonic integer version:

```csl
schema Ticket version 3 { ... }
```

The compiler, given two schema files of versions N and N+1, emits:

1. A forward migration (SQL + vector store ops).
2. A downgrade migration (SQL only; vector store ops are best-effort).
3. A diff report describing added/removed/changed fields and their runtime impact.

Breaking changes — removing a field, narrowing a type, tightening a permission — require an explicit `@deprecated` step in an intermediate version OR a `--allow-breaking` flag at migration time. This is the dbt / Prisma pattern.

Decorators changes (e.g., adding `@indexed`) are non-breaking and migrate in place.

---

## 10. Compilation Targets

One CSL file, N artifacts:

| Target               | Produced by `signet compile`        | Notes |
|----------------------|----------------------------------|-------|
| SQL forward migration | `out/sql/NNNN_up.sql`            | Postgres dialect; other dialects later |
| SQL down migration   | `out/sql/NNNN_down.sql`          |       |
| Vector store plan    | `out/vector/plan.yaml`           | Collections, index configs |
| Rust types           | `out/rust/generated.rs`          | Via `quote!` codegen |
| TypeScript types     | `out/ts/generated.ts`            |       |
| Python types         | `out/py/generated.py`            | Pydantic v2 models |
| MCP tool descriptors | `out/mcp/tools.json`             | One MCP server per schema |
| Policy WASM          | `out/policy/<schema>.wasm`       | Compiled per schema |
| Lineage manifest     | `out/lineage/manifest.json`      | Source → chunk → embed graph |
| Docs site page       | `out/docs/<schema>.md`           | Human-readable schema reference |

All artifacts are content-addressed; the compiler emits a top-level `manifest.json` that pins every hash.

---

## 11. Worked Example — Customer 360

```csl
namespace acme.crm
version 1

import "stdlib/profiles.csl" as profiles
import "./users.csl"

enum Tier { Free, Pro, Enterprise }

schema Customer version 2 {
  id:            Ulid    @primary
  external_id:   String  @unique @indexed
  name:          String  @searchable @boost(2.0)
  domain:        String  @searchable @indexed
  tier:          Tier    @facet @default(Tier.Free)
  owner:         Ref<User>
  health_score:  F32?    @computed("analytics.customer_health")
  summary:       Text    @computed("llm.customer_summary") @chunked
  created_at:    Instant @default(now())

  relations {
    tickets:   many Ticket   via Ticket.customer on_delete cascade
    contracts: many Contract via Contract.customer on_delete restrict
    notes:     many Note     via Note.customer   on_delete cascade
  }

  context {
    use             = profiles.Documents
    embedder        = "voyage-3"
    vector_dims     = 1024
    default_top_k   = 12
    freshness_decay = exponential(half_life = 45d)
  }

  policy {
    read:  has_role(caller, "admin")
        or caller.team == self.owner.team
        or caller.id in self.notes[*].shared_with

    list:  tenant_match(caller, self)

    write: has_role(caller, "admin")
        or caller.id == self.owner.id

    delete: has_role(caller, "admin")
  }

  @audit
  @retention(7y)
  @tenant_scope(caller.tenant)
}
```

The compiler emits:

- Postgres migration creating `customer` table, foreign keys, `customer_name_trgm_idx`, etc.
- Qdrant collection `customer_v2` with a 1024-dim vector named `summary_vec`.
- A Rust `Customer` struct, a TS `Customer` interface, a Pydantic `Customer` model.
- An MCP server at `/mcp/acme.crm.customer` exposing tools: `search_customer(query, tier?, top_k?)`, `get_customer(id)`, `list_notes(customer_id)`, `find_related_tickets(customer_id)`.
- A compiled `customer_v2.wasm` policy bundle containing the three predicates.
- A lineage manifest describing the `summary` computed field's dependency on `tickets` and `notes`.

---

## 12. Comparison to Adjacent Languages

| Feature                        | CSL | Prisma | GraphQL SDL | LookML | dbt semantic |
|--------------------------------|:---:|:------:|:-----------:|:------:|:------------:|
| Typed fields                   |  ✅ |  ✅    |     ✅      |  ✅    |     ✅       |
| Relations                      |  ✅ |  ✅    |     ✅      |  ✅    |     ✅       |
| Permission predicates          |  ✅ |  ❌    |     ❌      |  ⚠️ (access grants) | ❌ |
| Chunking declaration           |  ✅ |  ❌    |     ❌      |  ❌    |     ❌       |
| Embedder declaration           |  ✅ |  ❌    |     ❌      |  ❌    |     ❌       |
| Freshness decay                |  ✅ |  ❌    |     ❌      |  ❌    |     ❌       |
| Compiles to migrations         |  ✅ |  ✅    |     ❌      |  ❌    |     ❌       |
| Compiles to vector plan        |  ✅ |  ❌    |     ❌      |  ❌    |     ❌       |
| Compiles to MCP tools          |  ✅ |  ❌    |     ❌      |  ❌    |     ❌       |
| Compiles to WASM policy        |  ✅ |  ❌    |     ❌      |  ❌    |     ❌       |
| Versioned schemas              |  ✅ |  ⚠️     |     ⚠️       |  ❌    |     ⚠️        |
| Computed fields                |  ✅ |  ⚠️     |     ⚠️       |  ✅    |     ✅       |

Nothing else covers the retrieval + embedding + policy + MCP dimensions. That is the design thesis.

---

## 13. Error Messages — Design Goals

Borrowing Rust's approach: errors cite spans, suggest fixes, and point to documentation.

```
error[E0207]: reference target not found
 --> schemas/ticket.csl:5:16
  |
5 |   customer: Ref<Custmer>
  |                ^^^^^^^ no schema named `Custmer` in namespace `acme.crm`
  |
  = help: did you mean `Customer`?
  = note: schemas currently in scope: Customer, User, Ticket, Contract, Note
```

```
error[E0312]: permission predicate is not total
 --> schemas/ticket.csl:22:5
  |
22 |     read: has_role(caller, "admin")
  |     ^^^^ this predicate does not cover non-admin callers
  |
  = note: every `read` policy must evaluate to a boolean for every possible caller
  = help: add an `or` branch covering non-admin cases, or explicitly deny:
          `read: has_role(caller, "admin") or false`
  = help: or use the built-in profile `policy admin_only` if that is the intent
```

Totality checking for policies is a key static-analysis feature that nothing comparable ships.

---

## 14. Open Questions

1. **Streaming fields.** How do we express "this field is a live stream (e.g., a chat channel)" vs "this field is a snapshot"? Candidate: a `@streaming(source)` decorator with its own retrieval semantics.
2. **Multi-language content.** Should `@language` be explicit per field, per chunk, or detected? Current lean: detected with override.
3. **Cost-aware retrieval.** Should the `context` block expose per-query cost budgets? Probably yes, but not in v0.1.
4. **Policy inheritance.** Should schemas be able to `extends` a base policy block? Lean: no in v0.1 — keeps the type system trivially composable. Reconsider when we see real usage.
5. **Cross-tenant joins.** Forbidden by default. Should there be an escape hatch for federated searches (e.g., a managed-services provider serving multiple customers)? Probably a separate mode, not a language feature.
6. **Temporal queries.** Should the language natively express "as of timestamp X"? Postgres has temporal tables; we could expose them. Deferred.

---

## 15. Grammar Implementation Notes

- Parser: `winnow` (over `nom` for better error messages).
- Lexer: inline in the parser combinators; no separate lex pass.
- AST: fully owned tree with `proc_macro2::Span` for source positions.
- Type checker: bidirectional type inference over a closed universe; no inference for primitives.
- Codegen: `quote!` for Rust, a small templating engine (`minijinja`) for the rest.
- Incremental compilation: file-level hashing with memoized per-schema artifacts; sub-200ms rebuilds for 100-schema projects is the target.

*End of CSL specification draft.*
