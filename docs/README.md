# SealStack docs

Astro + Starlight site for <https://sealstack.ai>.

## Dev

```bash
cd docs
npm install
npm run dev        # http://localhost:4321
npm run check      # typecheck + broken-link check
npm run build      # emits dist/
```

## Adding a page

1. Drop a `.md` / `.mdx` file into `src/content/docs/<section>/`.
2. Add an entry to the `sidebar` in [`astro.config.mjs`](astro.config.mjs).
