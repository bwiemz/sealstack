# Governance

ContextForge uses a Working Groups model.

## Roles

- **Maintainers**: merge rights on the public repo. Listed in `CODEOWNERS`.
- **Working Group leads**: own a subsystem (engine, gateway, CSL, connectors, console, docs).
- **Contributors**: anyone with an accepted PR.

## Decision making

- Routine changes: maintainer consensus (lazy consensus, 72h).
- Cross-cutting or breaking changes: Working Group leads + one maintainer sign-off.
- Release gates: any maintainer can block a release with a rationale.

## Working Groups

- `wg/engine` — retrieval, memory, policy
- `wg/gateway` — REST, MCP, auth
- `wg/csl` — schema language, compiler
- `wg/connectors` — ingestion runtime, SDK
- `wg/console` — SvelteKit UI
- `wg/docs` — documentation site
