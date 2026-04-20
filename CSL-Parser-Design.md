# CSL Parser — Design Notes

**Companion to**: `crates/cfg-csl/` in the `contextforge-crates.tar.gz` drop-in.
**Status**: Unverified skeleton. Follows documented `winnow` 0.6 APIs; expect a round of compiler-driven fixes before first green build.

---

## Why winnow

The three realistic options were `nom`, `winnow`, and `pest`. The tradeoffs:

- **nom** — mature, well-documented, but its error-handling story is weaker than winnow's and `nom` 8.x still uses the older mutable-parser API.
- **pest** — great DX with its PEG grammar file, but inflexible for custom error recovery and loses compile-time type safety.
- **winnow** — the modern successor to nom. Same combinator-style as nom 8 but with first-class support for `&mut Input` parsers, `Located`/`Stateful` wrappers, and the `with_span` combinator that gives us byte ranges without any bookkeeping.

Brandon's NeuralScript uses a hand-written recursive-descent parser in pure Rust. CSL is smaller — a schema-definition language, not a general-purpose one — so the combinator approach keeps parser code roughly 1:1 with the grammar productions and is easier for contributors to read.

## Architecture

```
┌────────────┐     ┌────────────┐     ┌────────────┐     ┌────────────┐
│   source   ├────▶│   parser   ├────▶│ type check ├────▶│   codegen  │
│   (&str)   │     │    (AST)   │     │ (TypedFile)│     │ (targets)  │
└────────────┘     └────────────┘     └────────────┘     └────────────┘
                         │                  │                  │
                         ▼                  ▼                  ▼
                    CslError::Parse    CslError::Type     CslError::Codegen
                    (spans + miette rendering)
```

Every pass preserves byte spans. Errors are `miette::Diagnostic`-derived so `cargo check` output and a rendered CLI error both come from the same source.

### Module layout

| File | Purpose | Lines |
|---|---|---|
| `lib.rs` | Public entry points (`compile`, `compile_project`), target bitflags, output bundle | ~80 |
| `span.rs` | Byte-range spans with merge/point constructors + `miette::SourceSpan` conversion | ~60 |
| `ast.rs` | Every AST node, with derive(Serialize) for snapshot testing and future RPC exposure | ~380 |
| `error.rs` | `CslError` enum with `thiserror + miette::Diagnostic`; rich variants for common cases | ~160 |
| `parser.rs` | winnow combinators implementing the grammar. Pratt-style expression parser with 7 precedence levels | ~600 |
| `types.rs` | Pass 1 indexes decls; Pass 2 validates primary keys + references + decorator presence | ~180 |
| `codegen/mod.rs` | Dispatches to per-target modules based on `CompileTargets` flags | ~60 |
| `codegen/sql.rs` | Postgres DDL emitter: tables, FKs, indexes, chunk tables | ~120 |
| `codegen/mcp.rs` | The interesting one. CSL schema → MCP tool descriptors. See the MCP generator doc. | ~300 |

### Design decisions worth calling out

**1. No separate lexer.** winnow's combinator style merges lexing and parsing; we skip whitespace at the tail of each lexical unit with the `lex` helper. This is slightly slower than a two-phase approach on very large inputs but keeps the codebase small and debuggable. For CSL (a schema language, not source code), inputs are bounded.

**2. `Located<&str>` input.** This is winnow's wrapper that tracks byte offsets for every span produced by `.with_span()`. Zero overhead on the happy path, and we don't have to thread positional bookkeeping through the combinators.

**3. Pratt-style expression parser with manual precedence.** CSL has a small enough operator set that a table-driven Pratt parser was overkill. The expression parser is a descending chain of functions — `expr_or → expr_and → expr_not → expr_cmp → expr_add → expr_mul → expr_unary → expr_atom` — each handling one precedence level. Left-associativity falls out of the `loop { ... }` pattern. This is the idiom used in Rust-style hand-rolled parsers and it reads almost like a BNF.

**4. `cut_err` after keyword commit.** When the parser sees `schema`, `relations`, `context`, `policy`, or `enum`, it uses `cut_err` to prevent backtracking. This means if parsing fails after one of those keywords, the error is reported at the point of failure rather than silently backtracking and reporting a useless "unexpected token at start of file" later.

**5. Identifier validation happens post-tokenize.** The parser accepts any `[A-Za-z_][A-Za-z0-9_]*` as a candidate identifier and then checks against a reserved-words list. Alternative would be to distinguish via lookahead in the combinator; the current design is simpler and has better error locality.

**6. Type checker is a two-pass.** Pass 1 indexes every top-level declaration; Pass 2 validates references across the full symbol table. This lets us accept forward references (Schema A can `Ref<B>` before B is declared) without a topological sort.

**7. Decorator evaluation is deferred.** The parser accepts arbitrary decorator expressions; the type checker only validates what it needs (`@primary`, `@chunked`, `@embedded_from`, `@indexed`, `@searchable`, `@unique`, `@facet`). Unknown decorators are silently preserved and emitted as warnings. This keeps the parser forward-compatible: new decorators can be added in codegen without parser changes.

## Known issues (to fix during first compile pass)

1. **`ident_raw` is dead code.** Left over from an earlier draft that distinguished raw and validated identifiers. Remove it.

2. **`ws` vs `skip_ws` naming inconsistency.** Internal. Rename both to `skip_trivia`.

3. **Error span collection.** When the parser fails deep inside a nested rule, the reported span is the current input position at failure, not the start of the rule that committed. `cut_err` helps but for truly great errors we'd want to carry a context stack. v0.2 work.

4. **Field decorator trailing semicolons are optional**, but the grammar is ambiguous if a field's final decorator has a `= expr` assignment and the next line is another field. Current behavior: the parser greedily consumes whitespace and expects either `}`, a decorator starting with `@`, an identifier starting a new field, or a block keyword. In practice this works; in edge cases (e.g., `@permission.read = has_role(caller, "admin") permission: String`) the parser may fail. Specify explicit separators (either semicolons, or a newline-terminated variant of field_decl) if this becomes a real issue.

5. **Duration parsing is greedy and may mis-segment** in edge cases like `30days`. The current alternation puts `ns`, `us`, `ms`, `mo` before `s`/`m`, which handles the main cases, but `30d` adjacent to other tokens could grab too much. Fix: require a word boundary after the unit.

6. **List comprehension projection `self.notes[*].shared_with`** mentioned in the spec is not parsed. Adding it is 10 lines in `expr_atom` + an `Index` / `Project` variant on `Expr`.

## Running the parser

```bash
# In the repo root:
cd crates/cfg-csl
cargo test

# First, if the `unreachable!` in `ident_raw` trips a warning,
# remove that function — nothing calls it.
# Second, expect 1–3 trait-signature mismatches against winnow 0.6's
# actual API. Fix them by comparing with the examples in
# https://github.com/winnow-rs/winnow/tree/main/examples
```

## Snapshot tests

The `tests/parse_smoke.rs` test compiles the hello fixture and snapshots its output against `insta`. The first run emits `.snap.new` files which you accept with `cargo insta review`. CI then diffs against the committed snapshots.

This is the core regression harness: any grammar change or codegen shift shows up as a snapshot diff, and the PR reviewer sees exactly what changed in the generated SQL / MCP descriptors / vector plan.

## What's next after this scaffolds green

Once the parser compiles and the tests pass, priority order for extending it:

1. **Full expression coverage** — index expression (`a[b]`), member-access chains with safe navigation (`self.assignee?.id`), method calls on collections (`list.contains(x)`).
2. **Computed fields** — parser already accepts `@computed("pipeline.id")`; the type checker needs to resolve the pipeline name against a runtime registry passed in as context.
3. **Policy totality checking** — the spec §13 calls for the type checker to verify every policy predicate evaluates to a boolean for every possible caller. Requires a small abstract interpreter; tracked as a v0.2 feature.
4. **Migration diffing** — given two typed files of versions N and N+1, produce `Up` and `Down` SQL. The logic is mostly structural diff; reuse it to validate backward compatibility at CI time.
5. **LSP server** — once the parser is stable, expose it over the Language Server Protocol for VS Code integration. Gives you go-to-definition, hover-over-type, and inline error squiggles in CSL files. This is the closing-argument DX win against Prisma and GraphQL SDL, both of which have slow or non-existent tooling.

---
