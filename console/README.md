# SealStack Console

The administrative web interface for SealStack.

## Stack

- **SvelteKit 2** + **Svelte 5** (runes)
- **TypeScript** (strict)
- **Tailwind 4** (CSS-first config via `@tailwindcss/vite`)
- **lucide-svelte** for iconography
- No other component library — custom primitives live under `src/lib/components/`

The console is a pure SPA. Every API call goes directly from the browser to
the gateway; there is no server-side rendering or proxying. `+layout.ts` sets
`ssr = false` explicitly.

## Development

```bash
pnpm install           # or npm install
cp .env.example .env
pnpm dev               # serves on http://localhost:7071
```

The console expects a SealStack gateway on `http://localhost:7070` by
default. Override via the Settings page or by setting
`PUBLIC_GATEWAY_URL` in `.env`.

With the gateway running (`sealstack dev` in the workspace root), the full e2e loop
works:

1. Open `http://localhost:7071`
2. Settings → verify the gateway is reachable
3. Schemas → confirm `examples.Doc` is registered (run `sealstack schema apply`
   first if not)
4. Connectors → add a `local-files` binding, click **sync**
5. Query → run a search; click the receipt id to inspect provenance

## Routes

| Path | Purpose |
|------|---------|
| `/` | Dashboard: gateway health + schema / connector counts + quickstart |
| `/schemas` | List every registered schema |
| `/schemas/[qualified]` | Field / relation / context inspection for one schema |
| `/connectors` | List bindings; add new ones; trigger syncs |
| `/query` | Query playground with result list + receipt link |
| `/receipts` | Look up a receipt by id |
| `/receipts/[id]` | **The receipt viewer** — document-style provenance audit |
| `/settings` | Gateway URL + caller identity; persisted to localStorage |

## Aesthetic

Industrial editorial — dense monospace UI (JetBrains Mono) punctuated by
italic serif (Instrument Serif) for editorial moments and section labels.
Deep near-black canvas, subtle panel borders, forge-amber accent used
sparingly for active / pending / CTA states. The receipt viewer is the
visual differentiator — laid out like a legal document rather than a
typical SaaS card.

See [`../design/Console-Design.md`](../design/Console-Design.md) for the full design notes.

## Build

```bash
pnpm build
```

`@sveltejs/adapter-auto` picks the right adapter for your deploy target.
For a self-hosted static bundle behind Nginx or the gateway's own static
handler, swap to `@sveltejs/adapter-static` in `svelte.config.js`.

## Project layout

```
src/
├── app.css              # design tokens + primitive utility classes
├── app.html             # root HTML
├── lib/
│   ├── api.ts           # typed gateway client (envelope unwrap, ApiError)
│   ├── types.ts         # TypeScript mirrors of engine DTOs
│   ├── components/      # Metric, Status, SectionHeader, Tag, CodeBlock, EmptyState
│   └── stores/
│       └── settings.ts  # localStorage-backed settings
└── routes/
    ├── +layout.svelte   # sidebar + chrome
    ├── +layout.ts       # ssr = false
    ├── +page.svelte     # dashboard
    ├── schemas/
    ├── connectors/
    ├── query/
    ├── receipts/
    └── settings/
```
