# Principal Mapping ADR

How connector authors map source-system ACL primitives to the SDK's typed
[`Principal`](../src/principal.rs) enum.

## The closed set

`Principal` is a closed-set type with five variants:

- `User(String)` — individual identified by source-system identifier.
- `Group(String)` — named group whose membership is resolved at policy time.
- `Domain(String)` — anyone with an email under this domain.
- `Anyone` — publicly readable AND discoverable.
- `AnyoneWithLink` — readable to whoever has the URL; NOT discoverable.

There is no `Custom(String)` escape hatch. When a connector's source
ACL primitives don't obviously fit, the resolution is a deliberate
semantic-mapping decision, not stringly-typed drift.

## Design-pressure principle

> When a connector's source has ACL primitives that don't obviously fit
> the closed set, the resolution is a deliberate semantic-mapping
> decision, not an escape hatch. If no variant fits, the conversation is
> "should the SDK extend its closed set?" — surfaced as a design proposal,
> not papered over.

Closed-set discipline pushes design pressure onto the connector-author
side, not the SDK side. A `Custom` variant would relieve that pressure
and re-introduce the stringly-typed semantics the typed enum exists to
prevent.

## The mapping is semantic, not lexical

Pick the variant whose semantics match the source concept, not the
variant that happens to be most permissive. A connector author who
defaults to `Group` for everything they don't recognize is making the
typed enum useless.

## Worked examples

| Source concept | Variant | Identifier |
|---|---|---|
| Slack channel | `Group` | `slack:CXXX` |
| GitHub org/user owner | `Group` | `github:octocat` |
| GitHub team | `Group` | `github:acme/team-name` |
| Google Drive user permission | `User` | `alice@acme.com` |
| Google Drive group permission | `Group` | `eng@acme.com` |
| Google Drive domain permission | `Domain` | `acme.com` |
| Google Drive `type=anyone, allowFileDiscovery=true` | `Anyone` | (no inner identifier) |
| Google Drive `type=anyone, allowFileDiscovery=false` | `AnyoneWithLink` | (no inner identifier) |
| Notion workspace member | `User` | individual identifier (NOT `Group`) |
| Notion guest | `User` | guest identifier |
| Salesforce role | `Group` | `salesforce:role-name` |
| Domain-restricted Drive share | `Domain` | `acme.com` |

## In-identifier source-prefix convention

Connectors emitting `Group` identifiers should prefix them with their
connector name (`slack:CXXX`, `github:octocat`) for cross-connector
identifier disambiguation in shared policy rules. Without the prefix,
two connectors both emitting a `Group("admins")` would produce
ambiguous policy semantics.

The SDK doesn't enforce this — it's a convention, not a contract. But
every existing connector follows it, and breaking it produces ambiguous
policy semantics when two connectors both emit a `Group("admins")`.

## When to extend the closed set

Don't reach for `Custom`. There isn't one. If a source ACL primitive
genuinely doesn't fit any variant — not because mapping is hard, but
because the underlying concept is something else (a date-ranged ACL,
a quorum-based one, a per-resource-attribute one) — open a design
discussion to extend the enum or the `PermissionPredicate` shape.

This forces design pressure to surface as a real conversation rather
than as a quietly-growing collection of prefix conventions.

## Asymmetric legacy-alias treatment

The `Deserialize` impl on `Principal` accepts `"*"` as a synonym for
`Anyone`. This is a **semantic rename** of an unambiguous existing
wire shape — `"*"` always meant "anyone" in `PermissionPredicate::public_read()`'s
old impl, and the alias preserves backward-compat with test fixtures
and dev wire data without effort.

The `Deserialize` impl does **NOT** accept `"slack:..."` or
`"github:..."` (or any other connector-specific prefix) as legacy
aliases. Those were stringly-typed identifiers without typed semantic
anchoring; aliasing them would silently re-map data and mask the
design pressure this slice exerts on the connector emission paths.
Connectors must explicitly migrate to the typed enum; pre-migration
wire data fails to deserialize loudly, which is the correct signal.

## Roles project to actions

`PermissionPredicate.action: String` carries the role information
(`"read"`, `"write"`, `"list"`, `"delete"`). Connectors project source
roles down to these canonical actions at the connector boundary. The
SDK doesn't model role hierarchies in the type system because role
mapping is connector-specific (Drive's `commenter` doesn't map cleanly
to anything in Slack).
