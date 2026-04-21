# Deploying the docs site to Cloudflare Pages

The site at `docs/` publishes to Cloudflare Pages via **Cloudflare's Git
integration** — connect the repo once, and every push to `main` that
touches `docs/**` triggers a build. Pull requests get preview deploys at a
per-commit URL.

No GitHub Actions workflow is involved. The build runs on Cloudflare's
builder.

## Why this page exists

The first build will fail if the Pages project is pointed at the repo root,
because the workspace's root `package.json` runs `pnpm -r build` across
every Node project — including `console` and the SDKs that aren't what we
want deployed. The fix below scopes the build to `docs/`.

## Dashboard setup

### Create the project (once)

1. **Workers & Pages → Create → Pages → Connect to Git.**
2. Authorize Cloudflare's GitHub app against `bwiemz/sealstack`.
3. **Project name**: `sealstack-docs`.
4. **Production branch**: `main`.

### Build settings (this is the bit that matters)

Use these values when the dashboard prompts for build settings — or go to
**project settings → Builds & deployments → Build configuration → Edit**
if you already created the project with defaults:

| Field                                  | Value                       |
|----------------------------------------|-----------------------------|
| Framework preset                       | **Astro**                   |
| Build command                          | `npm ci && npm run build`   |
| Build output directory                 | `dist`                      |
| **Root directory (advanced)**          | `docs`                      |
| Environment variables                  | (none required)             |

The **root directory = `docs`** is the non-obvious one — it scopes Node /
build detection to that subfolder, bypassing the pnpm workspace at the
repo root entirely. Without it, Cloudflare walks into the root
`package.json`, runs `pnpm -r build`, and fails because dependencies
weren't installed for the workspaces it tries to build.

### Redeploy

After saving the build settings, click **Deployments → Retry deployment**
on the failed build. It should complete in under a minute; the Starlight
output ends up at `docs/dist/` which Cloudflare picks up as the publish
directory.

### Custom domain

Project → **Custom domains → Set up a custom domain** → enter `sealstack.ai`.
Because DNS is Cloudflare-managed, the CNAME / ALIAS record and TLS
certificate are provisioned automatically — no manual DNS step.

### (Optional) Block the `pages.dev` subdomain

Every Pages project is also reachable at `sealstack-docs.pages.dev`. Once
`sealstack.ai` is live, turn it off under **project settings → General →
Access policy → Block access to pages.dev subdomain** to avoid duplicate
content for SEO.

## Troubleshooting

**`sh: 1: vite: not found` or similar** — build is running at the repo
root instead of `docs/`. Re-check the Root directory setting.

**Build passes but `sealstack.ai` serves a 404** — the custom domain isn't
yet bound to the project. The DNS record was probably created but pointed
at the wrong Pages project. Project → Custom domains → verify.

**Build succeeds but only the first page renders** — Astro's `site` field
in [`astro.config.mjs`](astro.config.mjs) must match the production URL.
It's currently set to `https://sealstack.ai`; if you host on a subdomain,
update it.

## Local preview

```bash
cd docs
npm install
npm run dev       # http://localhost:4321
npm run build     # writes dist/
npm run preview   # serves dist/ on http://localhost:4321
```
