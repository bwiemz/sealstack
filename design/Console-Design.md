# Console — Design Notes

**Companion to**: `console/` in the drop-in.
**Status**: Unverified skeleton. 22 files, ~1,400 LOC across Svelte 5, TypeScript, and CSS. Targets `@sveltejs/kit` 2.x, `svelte` 5.x, `tailwindcss` 4.x. No `pnpm install && pnpm check` run in this environment — verify locally.

---

## 1. What this is (and isn't)

The console is the web face of Signet for operators — engineering leads, infra teams, security auditors. It's a narrow SPA with seven meaningful views, not a sprawling admin app. It covers what the CLI covers, in shapes a human can read.

What it **is**: a pure SvelteKit SPA. Every request goes browser → gateway. No SSR, no proxy layer, no BFF pattern. `+layout.ts` sets `ssr = false` explicitly.

What it **isn't**: a dashboard platform. There's no metrics warehouse, no Prometheus scrape, no user management. Those are separate concerns with better-suited tools.

Why SPA-only: in development the console runs on :7071 and the gateway on :7070 — different origins. Going through a SvelteKit server would require proxy setup, CORS handling, and SSR-while-gateway-unreachable error paths. The console is cheap to re-render; client-only is the right trade.

---

## 2. Aesthetic commitment

I opened this work by committing to **industrial editorial** — Linear × trading terminal × Swiss rail timetable. Dense monospace UI punctuated by italic serif for editorial moments. Dark by default. Amber accent used only for live / pending / CTA states, never decoratively.

The commitment shows up in three places:

### Typography

- **Instrument Serif (italic)** — page titles, section labels (small-caps), hero numbers, and quoted query text. Loaded directly via `@font-face` from `fonts.gstatic.com`. Chosen because it's distinctive (not Inter, not Space Grotesk) and reads as editorial-serious without the stuffiness of a traditional serif.
- **JetBrains Mono** — everything else. 95% of the UI is mono. Loaded from Google Fonts. Tabular lining figures enabled globally (`font-variant-numeric: tabular-nums lining-nums` on `.num`). Reads as terminal-coded, which matches "platform for operators."

The mono-primary choice is bold. Most SaaS consoles use sans for chrome and mono for code blocks. Inverting that — mono everywhere, serif as editorial contrast — is the single biggest visual decision in the console.

### Palette

11-step grayscale (`--color-ink-0` → `--color-ink-10`) plus three accent families (`forge`, `healthy`, `denied`), each with a full / dim / bg triple. Declared as Tailwind 4 `@theme` custom properties.

The accent discipline: amber is **earned**. It marks active states (forge glow on pending dots), primary CTAs (`btn-forge`), and the typographic "flourish" on the dashboard (the "context" in "context, engineered"). It does not appear in charts, gradients, or borders "for color."

Green = healthy **now**. Red = denied **now**. Everything that isn't actively doing one of those three things is grayscale.

### Recurring visual devices

Four things repeat across every page and give the console its identity:

1. **The label + rule divider** (`.label` + `.rule` in `app.css`, wrapped by `SectionHeader.svelte`). Small-caps italic serif label riding a horizontal rule — like a newspaper section head or a financial report. Every page uses it to mark zones.
2. **Status dots** (`.dot-*`). Tinted colored dots with a subtle box-shadow glow on the live ones. `<Status>` component wraps them with a label.
3. **Tabular numbers** (`.num`). Every number — scores, counts, timings, bytes, versions — uses tabular lining figures. The UI stays visually aligned even when the numbers shift.
4. **`.serif italic`** for quoted content. Query strings in the receipt viewer, excerpts in result lists, footnotes under metrics — they all render in italic Instrument Serif. The mono UI treats the actual content as "quotes" — it's a visual separation between operator and data.

---

## 3. Architecture

```
  Browser
    │
    ├── +layout.svelte (sidebar nav + main grid)
    │       │
    │       └── Route pages
    │             ├── +page.svelte                 (dashboard)
    │             ├── schemas/+page.svelte
    │             ├── schemas/[qualified]/+page.svelte
    │             ├── connectors/+page.svelte
    │             ├── query/+page.svelte
    │             ├── receipts/+page.svelte
    │             ├── receipts/[id]/+page.svelte  ← The wow moment
    │             └── settings/+page.svelte
    │
    ├── lib/api.ts          (Client, ApiError, envelope unwrap)
    ├── lib/types.ts        (TS mirrors of engine DTOs)
    ├── lib/stores/settings (localStorage-backed config)
    └── lib/components/     (Metric, Status, Tag, SectionHeader, CodeBlock, EmptyState)
            │
            ▼
   HTTP → Gateway :7070
```

### Data-fetching pattern

Every page fetches in `$effect` with a fresh `Client` built from `loadSettings()`. No `+page.ts` load functions, no TanStack Query, no server-side caching. Two reasons:

1. Settings change at runtime (Settings page → localStorage). A `load` function that reads settings at route-activation time gets stale the moment the user swaps gateway URLs. Re-fetching on every `$effect` run picks up the new settings automatically.
2. The data is small and the API is fast. A schemas list is a dozen rows; a receipt is a few KB. Caching adds complexity without meaningful latency savings for this use case.

Every `$effect` that fetches handles the full triple: `loading: boolean`, `error: string | null`, and the actual state. No pending-state cheating.

### API client (`lib/api.ts`)

Mirrors the CLI's `src/client.rs` one-to-one. The `Client` class constructor takes a `gatewayUrl`, a `user` string, and an optional custom `fetch`. Every endpoint is a method that returns a typed value extracted from the `{ data, error }` envelope.

`ApiError` is the one custom error. Code + message + optional status. Pages render it as `${code}: ${message}`. Network failures fall through as plain `Error`.

### State (`lib/stores/settings.ts`)

Lightweight localStorage wrapper. No Svelte store machinery — just `loadSettings()` / `saveSettings()` functions. Svelte 5 runes handle reactivity at the component level without needing a separate store API for settings this simple.

---

## 4. The receipt viewer

This is the visual differentiator — the page a buyer evaluating Signet should remember.

### Design intent

Glean shows citations. Signet shows **receipts**: a document the customer can save, audit, and sign. The receipt viewer leans hard into that framing. Wide margins, no sidebar intrusion, a 4xl italic serif title ("verdict & provenance"), and a bibliographic header block laid out in two columns like the front page of a court filing.

### Layout (top to bottom)

1. **Masthead.** `signet · receipt` label, italic serif title, full receipt ID in mono, JSON download button.
2. **Bibliographic block.** Two-column grid — issued / caller / schema / elapsed. Each field gets a small-caps label + a mono value. Caller tenant rendered as `user @ tenant` in a mono-with-muted dim pattern; roles render as chips.
3. **Query block.** The original query rendered as a blockquote — italic serif, left-border amber, large size. This is what's being audited.
4. **Latency breakdown.** Stacked horizontal bar segmented by stage (embed / vector / bm25 / fuse / rerank / policy). Each segment labeled inline where it's wide enough. Below: a six-column legend with each stage's exact millisecond count.
5. **Policy verdicts.** Rendered **diff-style**. Each rule as a row with a `+` (check icon, green background) or `−` (X icon, red background) prefix, the decision word in accent color, and the rule name. Optional italic serif message below in muted gray, indented. Reads like a git diff — immediately parseable as "what got allowed vs. denied."
6. **Sources.** Provenance table — numbered, score, italic serif excerpt, record/chunk IDs in mono. Scores stacked (base / rerank / freshness) in small type when present.
7. **Signed footer.** Left column: signature (or a muted serif italic "Unsigned. This receipt was issued by a development instance."). Right column: retention notice. Separated by a horizontal rule. Reads like the back of a contract.

### Why this works

Three specific moves earn the "document not card" feeling:

1. **Wide margin, centered column.** `max-w-4xl mx-auto`. The sidebar stays, but the receipt content gets breathing room nobody else gets. Signals importance.
2. **The italic serif isn't decorative.** The query is a quote. The excerpts are quotes. The footer's legal-disclaimer copy is italic serif. The typography *is* the content distinction between UI chrome and audited material.
3. **Diff-style verdicts.** `+` and `−` prefixes are an unusual move for a web UI. They land because the mental model they invoke (code diff / git log) is the right one — this is a change log of what was allowed vs. denied, and operators read diffs daily.

---

## 5. Page-by-page notes

### Dashboard

Hero line reads "**context, engineered.**" — the amber accent on the first word is the only typographic flourish in the whole console. Deliberately unique to this page.

Four metric tiles: schemas count, connectors count, scheduled syncs count, gateway status. The gateway tile goes `hot` (amber number) when the gateway is **down** — unusual but correct: red would clash with the amber accent elsewhere, and making the number amber draws the eye to the thing that needs attention.

Below: two side-by-side list previews (recent schemas, recent connectors) that gracefully empty-state into quickstart CLI snippets. Empty states are operator-educational — a new user who lands on the dashboard with a running-but-empty gateway gets a literal `signet schema apply` incantation to copy.

### Schemas list

Table: qualified name, version, table, facets (count + inline name list), relations (count). Qualified name is a link to the detail view; uses the `.` separator rendered in muted gray to help the eye parse `namespace.Name`.

### Schema detail

Header tracks the list pattern: small-caps namespace label, italic serif `Name.` with the final period in amber — a typographic joke on the dotted qualified name format.

Three tables below: fields (name + type + column + annotation chips), relations (name / kind / target / fk), context (grid of six cells for embedder, vector_dims, chunking, freshness decay, default_top_k, hybrid_alpha). Raw metadata in a collapsible code block with copy.

Annotation chips use `<Tag>` with semantic tones: `forge` for `pk` and `chunked` (these are definitive / identity-bearing fields), `denied` for `pii`, default gray for everything else.

### Connectors

Inline "new binding" form with three fields: kind (select), target schema (select populated from `listSchemas`), root path (shown conditionally for `local-files`). `github` and `slack` are in the kind dropdown as `not yet implemented` — signals the intended roadmap without silently hiding options.

Table with sync buttons per row. Syncing state is per-binding (keyed object); a spinner replaces the icon while in flight. Completion renders inline as `X/Y · Nms` — resources ingested / resources seen / elapsed.

### Query

The playground. Three-column top row: schema selector, top_k input, run button (amber CTA). Below: query textarea (italic serif, 17px — treats what the user types as the thing being quoted) and filters JSON textarea (mono, code-style background).

Keyboard: `⌘↵` / `Ctrl+↵` runs from anywhere in the form. Button label shows the shortcut.

Results as a vertical list of panels. Each hit: numbered prefix, amber score, ID on the right in small mono, italic serif excerpt, optional metadata chips. A prominent "view full receipt →" link lives both inline with each hit's receipt id and as a centered button below the last hit. The intent: every query result should feel one click away from its audit trail.

### Settings

localStorage wrapper around two fields (gateway URL, caller identity). The save button does two things: persist and re-check reachability. Reachability surfaces as a status dot next to the form buttons.

"About" block at the bottom with build version, GitHub link, license. Short and unceremonious — the interesting parts are above.

### Receipts index

When someone clicks the sidebar "receipts" link without a specific id in mind, they land here. Single input + view button. Empty-state copy points them to the playground to generate one.

---

## 6. What's intentionally missing

- **Search / filtering in list views.** Schemas and connectors lists are expected to be small enough (10s, not 1000s) that filtering adds complexity without value. Revisit if scale demands.
- **Pagination.** Same reason. The gateway returns everything; the console shows everything.
- **Realtime updates.** No WebSockets, no SSE. Sync progress fetches on completion, not during. Adding live updates requires the gateway to expose a streaming sync endpoint; not in v0.1.
- **Delete operations.** You can register a connector but not deregister from the UI. Matches the CLI surface. Production tool should add `DELETE /v1/connectors/{id}`.
- **Multi-tenant awareness.** Tenant shows on receipts (read-only). Switching tenants is a future auth-system concern.
- **Theme toggle.** Dark only. The aesthetic commitment doesn't survive translation to light mode; building both would require doubling the design budget. Revisit if operators demand it.

---

## 7. Known gaps — priority order

1. **Authentication.** No JWT validation; console sends `X-Cfg-User` header with whatever is in settings. Trivial to spoof. Fine for v0.1 dev, must be replaced before any production deployment.

2. **Error boundaries.** A thrown exception inside an `$effect` surfaces as a console error, not a UI state. Need a `+error.svelte` per route (or global) that renders the error with the same italic-serif empty-state aesthetic.

3. **Loading skeletons.** Current loading states are a single `shimmer` line. Proper skeletons (placeholder rows in tables, placeholder metrics on the dashboard) would make slow gateways less janky.

4. **Receipt viewer — timing bar edge cases.** If any stage is 0ms (e.g. no rerank configured), the stacked bar gets a zero-width segment that can cause layout issues. Filter to non-zero stages before rendering.

5. **Connector scheduling UI.** `interval_secs` is visible in the list but not editable from the UI. Add a dropdown on the add form (`on-demand`, `5m`, `15m`, `1h`, `custom…`).

6. **Schema search.** Browser find (`⌘F`) works on the current page but doesn't expand a collapsed context block or drill into relations. A page-level search would help.

7. **Query history.** localStorage-backed list of recent queries with quick-replay. Maybe three or four deep.

8. **Schema apply from UI.** Upload a `.csl` file, compile via an endpoint the gateway doesn't yet have, apply. Meaningful scope expansion; belongs in a later milestone.

9. **Activity feed on dashboard.** Stream of recent syncs, queries, receipts. Requires the gateway to expose an event endpoint.

10. **Mobile.** The console is desktop-first. Media queries for narrow viewports haven't been written. Operators use desktops; the audience tolerance for no-mobile is high. Worth revisiting for read-only receipt views (audit sharing).

---

## 8. Verification checklist

Unverified — no `pnpm install` in this environment. Most likely adjustments against a real Svelte 5 toolchain:

1. **Svelte 5 Snippet type import.** I use `import('svelte').Snippet` inline in `$props()` typing. If Svelte 5's type exports settle on a different path, the fix is `import type { Snippet } from 'svelte'` at the top.

2. **`$derived.by()`.** Used in the receipt viewer for the `timingStages` computation. Stable in Svelte 5.1+; if you're on an earlier alpha, fall back to `$derived(...)` with an IIFE.

3. **`$page` store.** SvelteKit 2 exports `page` from `$app/stores`. The `$page.params.id` pattern is standard; if the parameter name `qualified` (bracketed directory `[qualified]`) doesn't surface on `$page.params`, `$page.params['qualified']` forces the string-key access.

4. **Tailwind 4 `@theme` block.** Brand-new as of Tailwind 4. If the version pinned in `package.json` doesn't support it, fall back to a `tailwind.config.ts` with `theme.extend.colors`. The CSS variables work either way.

5. **`adapter-auto`.** Works for Vercel / Netlify / Node. For self-hosted behind nginx, swap to `@sveltejs/adapter-static` — one-line change in `svelte.config.js`.

6. **`dot-{kind}` class binding.** The dynamic class composition in `Status.svelte` relies on Tailwind seeing `.dot-healthy`, `.dot-pending`, `.dot-denied`, `.dot-muted` at build time. Since these are defined in `app.css` as custom classes (not dynamically composed Tailwind utilities), no safelist is needed. If switching to `bg-[var(--color-healthy)]` style bindings, add safelist entries.

7. **Lucide icon imports.** The icon names I imported (`Activity`, `Database`, `Plug`, `Search`, `ScrollText`, `Settings`, `Play`, `Loader2`, `Plus`, `RefreshCw`, `X`, `Check`, `Download`, `ArrowUpRight`, `Save`, `CircleCheck`) should all exist in lucide-svelte 0.454. If any were renamed, the fix is cosmetic.

If `pnpm check` passes and `pnpm dev` boots without a TypeScript scream, the console is structurally sound. The visual polish is already in `app.css`.

---

## 9. The demo flow

Because this is the last piece of v0.1, here's the full script that takes a new user from "I just cloned the repo" to "I see a receipt":

```bash
# Backend (one terminal)
cd signet
signet init
signet dev
signet schema apply schemas/doc.csl
signet connector add local-files --schema examples.Doc --root ./sample-docs
signet connector sync local-files/examples.Doc

# Frontend (another terminal)
cd console
pnpm install
pnpm dev
```

Open `http://localhost:7071`:

1. Dashboard shows "ready" status dot, schema count = 1, connector count = 1.
2. Schemas → click `examples.Doc` → see fields, relations, context, raw metadata.
3. Query → type "getting started" → run → see the hit list → click the receipt id.
4. Receipt viewer → see the full audit: caller, query, latency breakdown, policy verdicts, sources, signed footer.

Every pixel on that receipt page ties back to something real in the engine. That's the whole thesis.

*End of console design notes.*
