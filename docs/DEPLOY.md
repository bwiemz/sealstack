# Deploying the docs site to Cloudflare Pages

The site at `docs/` publishes to Cloudflare Pages from GitHub Actions. One
project, two environments:

- **Production** — `main` branch, served at `sealstack.ai`
- **Preview** — every pull request touching `docs/**` gets a unique URL

The workflow lives at [`.github/workflows/docs-deploy.yml`](../.github/workflows/docs-deploy.yml).

## One-time setup

You need these pieces wired up in your Cloudflare and GitHub accounts.
Everything past that is automatic.

### 1. Create the Cloudflare Pages project

From the Cloudflare dashboard:

1. **Workers & Pages** → **Create application** → **Pages** → **Connect to Git**.
2. Authorize Cloudflare's GitHub app against `bwiemz/sealstack` (repo-scoped is fine).
3. Project name: `sealstack-docs` (must match `--project-name` in the workflow).
4. Production branch: `main`.
5. **Build settings** — these are also re-specified by the workflow, so pick
   anything reasonable; the workflow's `wrangler pages deploy dist` overrides
   them on every run.
6. Click **Save and Deploy** — the first build runs via Cloudflare's own
   builder. Subsequent builds come from the workflow.

### 2. Mint a Cloudflare API token

1. Cloudflare dashboard → top-right avatar → **My Profile** → **API Tokens** → **Create Token**.
2. Use the "Edit Cloudflare Workers" template and narrow it:
   - **Permissions**: `Account · Cloudflare Pages · Edit`
   - **Account resources**: your account
3. Copy the token — Cloudflare only shows it once.

### 3. Find your account id

Dashboard URL: `https://dash.cloudflare.com/<ACCOUNT_ID>/...`. That UUID-looking
segment is the account id.

### 4. Set GitHub repo secrets

GitHub repo → **Settings** → **Secrets and variables** → **Actions** → **New repository secret**:

- `CLOUDFLARE_API_TOKEN` — the token from step 2
- `CLOUDFLARE_ACCOUNT_ID` — the id from step 3

### 5. Wire the custom domain

In the Cloudflare Pages project → **Custom domains** → **Set up a custom domain**:

- Enter `sealstack.ai` (or `docs.sealstack.ai` if you want to reserve the apex
  for a marketing site later).
- Because `sealstack.ai` is already on Cloudflare DNS, the CNAME/ALIAS record
  is auto-created and TLS issues within ~1 minute.

### 6. (Optional) Strip the `pages.dev` subdomain

By default every Pages project is also reachable at
`sealstack-docs.pages.dev`. Turn it off via the project's **Settings →
General → Block access to pages.dev subdomain** toggle once the custom domain
is live — it prevents duplicate-content SEO noise.

## Verify

Push a change to anything under `docs/` on `main`. The workflow should:

1. Build the Starlight site (`npm run build` → `docs/dist/`).
2. Call `wrangler pages deploy dist --project-name=sealstack-docs`.
3. Publish, with the production URL updating within ~30 seconds.

Pull requests get a per-commit preview URL posted back as a bot comment.

## Troubleshooting

**`Error: Invalid API token`** — token likely lacks `Cloudflare Pages: Edit`
permission, or is scoped to the wrong account.

**404 on the custom domain for more than 2 minutes** — check the Cloudflare
DNS tab: the Pages integration should have added a `CNAME` for `@` (or
`docs`) pointing at `<project>.pages.dev`. If it's missing, add it manually.

**Build works locally but fails in CI** — the workflow runs `npm ci`, which
respects `package-lock.json` exactly. If you committed a `package.json`
change without refreshing the lockfile, CI will fail. Run `npm install` and
commit the lockfile.
