<script lang="ts">
  import { page } from '$app/stores';
  import { Client, ApiError } from '$lib/api';
  import { loadSettings } from '$stores/settings';
  import type { SchemaMeta } from '$lib/types';
  import SectionHeader from '$components/SectionHeader.svelte';
  import Tag from '$components/Tag.svelte';
  import CodeBlock from '$components/CodeBlock.svelte';
  import EmptyState from '$components/EmptyState.svelte';
  import { Search } from 'lucide-svelte';

  let meta = $state<SchemaMeta | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);

  $effect(() => {
    const qualified = $page.params.qualified;
    if (!qualified) {
      error = 'missing schema name';
      loading = false;
      return;
    }
    loading = true;
    error = null;
    const s = loadSettings();
    const c = new Client({ gatewayUrl: s.gatewayUrl, user: s.user });
    c.getSchema(qualified)
      .then((m) => (meta = m))
      .catch((e) => (error = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e)))
      .finally(() => (loading = false));
  });
</script>

<svelte:head>
  <title>{$page.params.qualified} · SealStack</title>
</svelte:head>

<div class="px-10 py-10 max-w-5xl">
  {#if loading}
    <p class="serif italic text-[var(--color-ink-7)] shimmer">loading…</p>
  {:else if error}
    <EmptyState title="Schema unavailable" body={error} />
  {:else if meta}
    <!-- Masthead -->
    <header class="mb-8">
      <div class="flex items-baseline justify-between gap-6">
        <div>
          <div class="label">{meta.namespace}</div>
          <h1 class="serif text-6xl text-[var(--color-ink-10)] mt-2 leading-none">
            {meta.name}<span class="text-[var(--color-forge)]">.</span>
          </h1>
          <div class="flex items-baseline gap-4 mt-4 text-[13px]">
            <span class="num text-[var(--color-ink-8)]">version · v{meta.version}</span>
            <span class="num text-[var(--color-ink-8)]">table · {meta.table}</span>
            <span class="num text-[var(--color-ink-8)]">collection · {meta.collection}</span>
          </div>
        </div>
        <a
          href="/query?schema={meta.namespace}.{meta.name}"
          class="btn btn-forge flex items-center gap-2"
        >
          <Search size={12} strokeWidth={2} />
          query this schema
        </a>
      </div>
    </header>

    <!-- Fields -->
    <SectionHeader label="fields" count={meta.fields.length} />
    <div class="panel overflow-hidden mb-8">
      <table class="data">
        <thead>
          <tr>
            <th>name</th>
            <th style="width: 10rem">type</th>
            <th style="width: 8rem">column</th>
            <th>annotations</th>
          </tr>
        </thead>
        <tbody>
          {#each meta.fields as f}
            <tr>
              <td class="font-medium text-[var(--color-ink-10)]">
                {f.name}
                {#if f.primary}
                  <Tag tone="forge">pk</Tag>
                {/if}
              </td>
              <td class="num text-[var(--color-ink-9)]">
                {f.ty}{f.optional ? '?' : ''}
              </td>
              <td class="num text-[var(--color-ink-8)]">{f.column}</td>
              <td class="flex flex-wrap gap-1">
                {#if f.indexed}<Tag>indexed</Tag>{/if}
                {#if f.searchable}<Tag>searchable</Tag>{/if}
                {#if f.chunked}<Tag tone="forge">chunked</Tag>{/if}
                {#if f.facet}<Tag>facet</Tag>{/if}
                {#if f.unique}<Tag>unique</Tag>{/if}
                {#if f.boost !== null}<Tag>boost · {f.boost}</Tag>{/if}
                {#if f.pii !== null}<Tag tone="denied">pii · {f.pii}</Tag>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <!-- Relations -->
    {#if Object.keys(meta.relations).length > 0}
      <SectionHeader label="relations" count={Object.keys(meta.relations).length} />
      <div class="panel overflow-hidden mb-8">
        <table class="data">
          <thead>
            <tr>
              <th>name</th>
              <th style="width: 5rem">kind</th>
              <th>target</th>
              <th style="width: 10rem">fk</th>
            </tr>
          </thead>
          <tbody>
            {#each Object.values(meta.relations) as r}
              <tr>
                <td class="text-[var(--color-ink-10)]">{r.name}</td>
                <td>
                  <Tag>{r.kind}</Tag>
                </td>
                <td class="num">
                  <a
                    href="/schemas/{r.target_namespace}.{r.target_schema}"
                    class="link"
                  >
                    {r.target_namespace}.{r.target_schema}
                  </a>
                </td>
                <td class="num text-[var(--color-ink-8)]">{r.foreign_key ?? '—'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    <!-- Context -->
    <SectionHeader label="context" />
    <div class="panel p-6 mb-8 grid grid-cols-2 gap-6 text-[13px]">
      <div>
        <div class="label">embedder</div>
        <div class="num text-[var(--color-ink-10)] mt-1.5">{meta.context.embedder}</div>
      </div>
      <div>
        <div class="label">vector dims</div>
        <div class="num text-[var(--color-ink-10)] mt-1.5">{meta.context.vector_dims}</div>
      </div>
      <div>
        <div class="label">chunking</div>
        <pre class="code text-[11px] mt-1.5">{JSON.stringify(meta.context.chunking, null, 2)}</pre>
      </div>
      <div>
        <div class="label">freshness decay</div>
        <pre class="code text-[11px] mt-1.5">{JSON.stringify(meta.context.freshness_decay, null, 2)}</pre>
      </div>
      <div>
        <div class="label">default top_k</div>
        <div class="num text-[var(--color-ink-10)] mt-1.5">
          {meta.context.default_top_k ?? '—'}
        </div>
      </div>
      <div>
        <div class="label">hybrid α</div>
        <div class="num text-[var(--color-ink-10)] mt-1.5">
          {meta.hybrid_alpha ?? 'engine default'}
        </div>
      </div>
    </div>

    <!-- Raw meta -->
    <SectionHeader label="raw metadata" />
    <CodeBlock value={JSON.stringify(meta, null, 2)} maxHeight="24rem" />
  {/if}
</div>
