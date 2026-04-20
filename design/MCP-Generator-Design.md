# MCP Server Auto-Generation — Design Notes

**Companion to**: `crates/signet-csl/src/codegen/mcp.rs` and `crates/signet-gateway/src/mcp/` in the drop-in.
**Status**: Unverified skeleton. Targets MCP spec revision **2025-11-25** (streamable HTTP transport, OAuth 2.1 discovery).

The thesis of Signet is that an enterprise context platform should expose **every schema as a typed MCP server**, automatically. You declare the schema once in CSL; you get a running MCP endpoint with properly-typed tools, inferred permissions, and auditable receipts. No hand-written tool wiring, no drift between your permission model and what the LLM sees.

This document explains how that works end-to-end: the compile-time descriptor generation, the runtime dispatch, the transport layer, and the auth story.

---

## End-to-end flow

```
┌───────────┐  compile  ┌─────────────┐  boot-time  ┌──────────────┐
│  x.csl    ├──────────▶│  manifest   ├─────────────▶│   registry   │
│           │           │   .json     │             │   (per-tool) │
└───────────┘           └─────────────┘             └───────┬──────┘
                                                             │
                                   ┌─────────────────────────┤
                                   ▼                         ▼
                       ┌─────────────────────┐   ┌───────────────────────┐
                       │  POST /mcp/<name>   │   │  .well-known/oauth-*  │
                       │    JSON-RPC         │   │     discovery         │
                       └──────────┬──────────┘   └───────────────────────┘
                                  │
                                  ▼
                       ┌─────────────────────┐
                       │     dispatch()      │
                       │  (protocol.rs)      │
                       └──────────┬──────────┘
                                  │
                                  ▼
                       ┌─────────────────────┐
                       │  ToolHandler::invoke│
                       │  → EngineFacade     │
                       └─────────────────────┘
```

1. **Compile-time.** `signet-csl` parses the CSL source, type-checks it, and emits a JSON manifest of MCP servers. Each schema becomes one server with a fixed toolset: `search_X`, `get_X`, `list_X`, one `list_X_<rel>` per `many` relation, and one `aggregate_X_<facet>` per `@facet` field.

2. **Boot-time.** The gateway reads the manifest, iterates every tool descriptor, and inserts a `GeneratedHandler` into the `ToolRegistry`. The handler carries the descriptor plus a `HandlerKind` enum that tells `invoke` which engine method to call. No reflection, no macros — just pattern matching on a small enum.

3. **Runtime.** A POST to `/mcp/<server_name>` with a JSON-RPC body (`tools/list`, `tools/call`, etc.) hits the transport layer, which resolves the session, authenticates the caller, and dispatches to `protocol::dispatch`. For `tools/call`, that function looks up the handler in the registry and invokes it with the caller context attached. The handler validates arguments, calls into `signet-engine`, and packages the result into a `ToolsCallResult` with structured content.

---

## Why auto-generation is the right default

Every hand-written MCP server is a liability:

- **Drift.** Your tool descriptor says `query: string`, your runtime handler expects `query_text: string`. Someone updated one side and not the other; the LLM gets an error at the worst moment.
- **Permission leaks.** Your CSL schema says `read` requires `caller.team == self.owner.team`. Your handler queries the DB and returns everything. The permission predicate lives in two places, and only one is enforced.
- **Boilerplate.** Ten schemas × five default tools each = fifty handler implementations, all nearly identical. No one reviews the 51st one carefully.

Auto-generation inverts this: CSL is the single source of truth, and the generated tools are guaranteed to match the schema. If the schema changes, tools change. If the schema adds a `@facet`, a new aggregate tool appears. If a field is marked `@pii`, it's redacted on the wire without any handler code being touched.

---

## Tool contract (per schema)

For schema `Customer { ... }` with namespace `acme.crm`:

| Tool | Input | Output | When emitted |
|---|---|---|---|
| `search_customer` | `{query, top_k, filters, freshness}` | `{results[], receipt_id, took_ms}` | Always |
| `get_customer` | `{id}` | Full record | Always |
| `list_customer` | `{filters, limit, cursor, order_by, direction}` | `{items[], next_cursor, total_est}` | Always |
| `list_customer_tickets` | `{parent_id, limit, cursor}` | `{items[], next_cursor}` | For each `many` relation |
| `aggregate_customer_tier` | `{filters, buckets}` | `{facet, buckets[]}` | For each `@facet` field |

Each tool's `inputSchema` is auto-derived from the CSL types:

| CSL type | JSON Schema |
|---|---|
| `String`, `Text`, `Ulid`, `Uuid`, `Instant`, `Duration` | `"string"` |
| `I32`, `I64` | `"integer"` |
| `F32`, `F64` | `"number"` |
| `Bool` | `"boolean"` |
| `Json` | `"object"` |
| `Vector<N>` | `{type: "array", items: {type: "number"}, minItems: N, maxItems: N}` |
| `List<T>` | `"array"` with items derived from `T` |
| `Ref<T>` | `"string"` (holds the target's primary key) |
| `T?` | Same as `T`, not in `required` |

### Why this schema shape

**Search always has a receipt_id in the response.** This is Signet's signature behavior: every retrieval produces a signed provenance record. The LLM gets text; the audit system gets `caller + query + sources + timestamps`. Without this, you can't answer "how did the model know that?" — which is the single most common question from compliance.

**Filters live inside a nested object, not as top-level params.** Otherwise adding a new facet to the CSL schema changes the top-level param set and breaks every client. The nested-object pattern keeps the tool contract stable.

**Cursor-based pagination, not offset.** Offset pagination falls apart at scale and misses inserts. MCP clients now consistently handle cursors; no need to support both.

**Facet filters accept either a scalar or an array.** `{tier: "Pro"}` and `{tier: ["Pro", "Enterprise"]}` both work. The JSON Schema uses `oneOf` to express this.

---

## Runtime pieces

### `signet-gateway/src/mcp/types.rs`

The wire vocabulary. JSON-RPC 2.0 request/response, MCP-specific `InitializeParams`, `ServerCapabilities`, `ToolDescriptor`, and the `Caller` struct that downstream code uses for permission decisions.

Worth calling out: **`Caller` is built from the authenticated request context, not from anything the MCP client sent.** Clients can claim whatever they want in their arguments; the gateway ignores caller claims in the JSON body and reads identity from the OAuth token it validated. This is the main attack-surface reduction in the design.

### `signet-gateway/src/mcp/registry.rs`

`DashMap<(server_name, tool_name), Arc<dyn ToolHandler>>`. O(1) lookup, no locking on the hot path. The `ToolHandler` trait is async and takes both a caller and the raw arguments; each handler returns either a JSON `Value` (success) or a `ToolError` (mapped to JSON-RPC error codes by the dispatcher).

### `signet-gateway/src/mcp/protocol.rs`

The JSON-RPC dispatcher. Fields roles:

- `initialize` → returns the gateway's `ServerInfo`, `PROTOCOL_VERSION`, and advertised capabilities.
- `tools/list` → reads the registry, emits a stable sorted list of descriptors for this server.
- `tools/call` → validates the tool exists, invokes it, and maps `ToolError` → `JsonRpcError`. Application errors are surfaced as a *successful* JSON-RPC response with `isError: true` per MCP convention; protocol errors (missing tool, bad params) are surfaced as proper JSON-RPC errors.
- `resources/list` → stub in v0.1.
- `ping` → returns `{}`.

Everything else returns `METHOD_NOT_FOUND`.

### `signet-gateway/src/mcp/transport.rs`

Streamable HTTP per the 2025-11 MCP spec:

- **One POST endpoint per server** at `/mcp/<qualified_name>`. Accepts a single JSON-RPC request or a batch array.
- **Optional GET** on the same endpoint opens an SSE channel for server-initiated notifications (tool-list changes, progress updates). v0.1 accepts the connection and returns; v0.2 wires it into a per-session broadcast channel.
- **DELETE** terminates a session.
- **Session tracking** via the `Mcp-Session-Id` header. Sessions are opaque ULIDs, created lazily on first request, reaped after 30 min of inactivity.

The session cache is a `DashMap<String, Session>` with opportunistic reaping on every request. For a single-process gateway this is fine; for a multi-node deployment you'd replace the DashMap with Redis (the `redis_url` config field is already there).

### `signet-gateway/src/mcp/oauth.rs`

OAuth 2.1 discovery. Serves two well-known endpoints:

- `/.well-known/oauth-protected-resource` — this endpoint always serves, advertising the acceptable authorization server(s) for this resource. The MCP client reads this and bounces through the advertised IdP.
- `/.well-known/oauth-authorization-server` — only served when the gateway is configured to also act as its own IdP (rare; mostly useful for self-contained demos). In production this would be served by Okta/Auth0/Entra ID and our endpoint returns 404.

The gateway does **not** implement the OAuth flow itself — it validates bearer tokens (via JWKS URL lookup) and trusts the advertised IdP to handle the dance. This is the right division: context platforms should not be identity providers.

### `signet-gateway/src/mcp/handlers.rs`

The `GeneratedHandler` struct plus the `EngineFacade` trait. The handler holds:
- The pre-computed descriptor.
- A `HandlerKind` enum tag.
- Optional `relation` / `facet` name (for `ListRelation` / `Aggregate` shapes).
- An `Arc<dyn EngineFacade>` pointing at the engine.

The `EngineFacade` trait is deliberately narrow — five async methods covering the five handler shapes. This gives us:

1. **Testability.** `StubEngine` implements the trait with `Unimplemented` everywhere, letting gateway tests exercise routing and session handling without a real engine.
2. **Decoupling.** The gateway crate doesn't need to depend on specific engine internals. When the engine module tree is refactored (and it will be), the gateway doesn't feel it.
3. **Future flexibility.** Different engine implementations — one for the cloud multi-tenant deployment, one for the self-hosted single-tenant — satisfy the same facade trait and plug in interchangeably.

---

## Security model

Three layers, each owns one concern:

1. **Transport layer authenticates the caller.** OAuth 2.1 bearer token validation against the advertised IdP's JWKS. A valid token populates `Caller { id, tenant, groups, roles, attrs }`; an invalid one returns 401 before any MCP logic runs.

2. **Handler layer validates arguments.** Each `invoke` checks argument presence and types. Malformed args → `ToolError::InvalidArgs` → JSON-RPC `INVALID_PARAMS`.

3. **Engine layer enforces policy.** The CSL policy block is compiled to a WASM predicate at schema-apply time; the engine runs it against each candidate record before returning results. A record that fails policy never reaches the gateway, let alone the LLM. The engine also tags each returned record with which policies were evaluated; those tags end up in the receipt.

This separation matters because **policy bugs in one schema shouldn't leak data from another**. The engine refuses to return a record unless its specific schema's policy approved the caller for the specific action. Cross-schema leaks would require a bug in the engine itself, not just in one schema definition.

---

## Receipts

Every successful `tools/call` produces a receipt:

```json
{
  "id":         "01HQVZ...",
  "caller":     "user:abc",
  "tenant":     "tenant:xyz",
  "tool":       "search_customer",
  "args_hash":  "sha256:...",
  "sources": [
    {"chunk_id": "01HQW1...", "schema": "Customer", "record_id": "01HQW0...", "score": 0.87}
  ],
  "policies_applied": ["customer.read.v2"],
  "answer_hash":      "sha256:...",
  "timestamp":        "2026-04-20T16:41:14Z",
  "signature":        "ed25519:..."
}
```

The receipt is signed by the gateway's private key. Compliance teams can replay a query and re-verify the signature; legal teams can subpoena it when the question is "what data did this model see on October 3rd when it produced that output?"

The `answer_hash` is filled in after the LLM answers; the gateway doesn't compute it, but it exposes a follow-up endpoint for the client to post back the final answer and close the receipt. Clients that don't close get a partial receipt, which is still useful for audit.

---

## Known gaps before v0.1 ships

1. **No real auth wire-up.** The transport layer's `resolve_session` returns an anonymous caller. Real implementation reads the `Authorization: Bearer <token>` header, validates against the configured JWKS, and populates `Caller` from the JWT claims. Maybe 60 lines of code; held off because it's mostly plumbing.

2. **Stub engine only.** `StubEngine` returns `Unimplemented` for every call. Real implementations live in `signet-engine`. The gateway is ready for them.

3. **No SSE notifications.** `GET /mcp/<n>` opens an SSE connection but emits nothing. Per-session broadcast wiring is about 100 lines with `tokio::sync::broadcast` but not needed for the scaffold.

4. **No streaming results.** `tools/call` returns a single response. Long-running tools (e.g., `list_customer` over 1M records) would benefit from streaming; the spec supports it via SSE. v0.2.

5. **No tool-level rate limiting.** The gateway has a global timeout layer but no per-tool budgets. Add a `tower::limit` layer with keys derived from `Caller.id + tool_name` before enterprise use.

6. **No JSON Schema validation at runtime.** The `inputSchema` is exposed to clients but not enforced server-side. Each handler validates arguments individually. A global schema validator (using `jsonschema` crate) would give us consistent error messages; held off to avoid an extra allocation per call on the hot path.

## Try it yourself

Once the crates compile:

```bash
cd /path/to/signet
cargo run --bin signet-gateway &
GATEWAY_PID=$!

# The registry is empty until you register handlers; add this as the boot step.
# In a test:
cargo test -p signet-gateway --test mcp_smoke

# Or poke it manually:
curl http://localhost:7070/.well-known/oauth-protected-resource | jq

curl -X POST http://localhost:7070/mcp/Note \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'

kill $GATEWAY_PID
```

The `tools/list` call will return an empty array until you wire `signet-engine` and register generated handlers at boot. That wiring is the next piece of the scaffold — a ~50-line `register_schema` function that takes a `TypedFile` and a `ToolRegistry` and inserts handlers for every schema.

---

## How this plays against Glean and the category

Glean doesn't expose MCP servers per datasource. Their proprietary "Agent" building is a walled-garden where you build agents inside their UI against their capability set. MCP is additive to them — they support MCP *clients* consuming Glean, not Glean-as-MCP-server.

Signet's posture is the inverse: we are the MCP server provider. Your data sources become typed MCP endpoints your agents consume from Claude, ChatGPT, Cursor, or your own framework. That difference is why the integration story on the battle card lands on enterprise buyers who already have agent-building tools and just need a context substrate.

---
