<script lang="ts">
  import { Client, ApiError } from '$lib/api';
  import { loadSettings } from '$stores/settings';
  import type { SchemaSummary } from '$lib/types';
  import SectionHeader from '$components/SectionHeader.svelte';
  import EmptyState from '$components/EmptyState.svelte';

  let schemas = $state<SchemaSummary[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);

  $effect(() => {
    const s = loadSettings();
    const c = new Client({ gatewayUrl: s.gatewayUrl, user: s.user });
    c.listSchemas()
      .then((ss) => (schemas = ss))
      .catch((e) => (error = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e)))
      .finally(() => (loading = false));
  });
</script>

<svelte:head>
  <title>Schemas · SealStack</title>
</svelte:head>

<div class="px-10 py-10">
  <header class="mb-8">
    <div class="label">schemas</div>
    <h1 class="serif text-5xl text-[var(--color-ink-10)] mt-2 leading-none">registered.</h1>
    <p class="serif italic text-[var(--color-ink-8)] text-lg mt-3 max-w-xl leading-snug">
      Every schema is a CSL namespace + name + version. Apply new ones from the CLI —
      <code class="code inline text-[12px] px-1.5 py-0.5">sealstack schema apply</code>.
    </p>
  </header>

  <SectionHeader label="inventory" count={schemas.length} />

  {#if loading}
    <p class="serif italic text-[var(--color-ink-7)] shimmer">loading…</p>
  {:else if error}
    <EmptyState title="Cannot reach gateway" body={error} />
  {:else if schemas.length === 0}
    <EmptyState
      title="No schemas yet."
      body="Compile a CSL file and apply it to the gateway to see it here. The CLI scaffolds a starter schema with `sealstack init`."
    />
  {:else}
    <div class="panel overflow-hidden">
      <table class="data">
        <thead>
          <tr>
            <th>qualified name</th>
            <th style="width: 5rem">version</th>
            <th style="width: 14rem">table</th>
            <th style="width: 8rem">facets</th>
            <th style="width: 10rem">relations</th>
          </tr>
        </thead>
        <tbody>
          {#each schemas as s}
            {@const qualified = `${s.namespace}.${s.name}`}
            <tr>
              <td>
                <a href="/schemas/{qualified}" class="link font-medium">
                  {s.namespace}<span class="text-[var(--color-ink-7)]">.</span>{s.name}
                </a>
              </td>
              <td class="num text-[var(--color-ink-9)]">v{s.version}</td>
              <td class="num text-[var(--color-ink-8)]">{s.table ?? '—'}</td>
              <td class="num text-[var(--color-ink-8)]">
                {s.facets?.length ?? 0}
                {#if s.facets && s.facets.length > 0}
                  <span class="text-[var(--color-ink-7)] ml-1">({s.facets.join(', ')})</span>
                {/if}
              </td>
              <td class="num text-[var(--color-ink-8)]">
                {s.relations?.length ?? 0}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>
