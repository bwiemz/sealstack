<script lang="ts">
  import { Client, ApiError } from '$lib/api';
  import { loadSettings } from '$stores/settings';
  import type { SchemaSummary, SearchResponse } from '$lib/types';
  import SectionHeader from '$components/SectionHeader.svelte';
  import EmptyState from '$components/EmptyState.svelte';
  import { Play, Loader2 } from 'lucide-svelte';

  // --- State --------------------------------------------------------------

  let schemas = $state<SchemaSummary[]>([]);
  let loadingSchemas = $state(true);

  let query = $state('');
  let selectedSchema = $state('');
  let topK = $state<number | ''>('');
  let filtersRaw = $state('');
  let filtersError = $state<string | null>(null);

  let running = $state(false);
  let response = $state<SearchResponse | null>(null);
  let runError = $state<string | null>(null);

  const clientFor = () => {
    const s = loadSettings();
    return new Client({ gatewayUrl: s.gatewayUrl, user: s.user });
  };

  // --- Effects ------------------------------------------------------------

  $effect(() => {
    const c = clientFor();
    c.listSchemas()
      .then((ss) => {
        schemas = ss;
        if (!selectedSchema && ss.length > 0) {
          selectedSchema = `${ss[0].namespace}.${ss[0].name}`;
        }
      })
      .finally(() => (loadingSchemas = false));
  });

  // --- Actions ------------------------------------------------------------

  async function runQuery() {
    runError = null;
    filtersError = null;
    response = null;

    if (!selectedSchema) {
      runError = 'Choose a schema.';
      return;
    }
    if (!query.trim()) {
      runError = 'Enter a query string.';
      return;
    }

    let filters: Record<string, unknown> = {};
    if (filtersRaw.trim().length > 0) {
      try {
        filters = JSON.parse(filtersRaw);
        if (filters === null || typeof filters !== 'object' || Array.isArray(filters)) {
          filtersError = 'Filters must be a JSON object.';
          return;
        }
      } catch (e) {
        filtersError = `Filters: ${e instanceof Error ? e.message : String(e)}`;
        return;
      }
    }

    running = true;
    try {
      response = await clientFor().query({
        schema: selectedSchema,
        query: query.trim(),
        top_k: typeof topK === 'number' && topK > 0 ? topK : undefined,
        filters
      });
    } catch (e) {
      runError = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    } finally {
      running = false;
    }
  }

  function onKey(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      runQuery();
    }
  }
</script>

<svelte:head>
  <title>Query · ContextForge</title>
</svelte:head>

<div class="px-10 py-10 max-w-6xl">
  <header class="mb-8">
    <div class="label">query</div>
    <h1 class="serif text-5xl text-[var(--color-ink-10)] mt-2 leading-none">playground.</h1>
    <p class="serif italic text-[var(--color-ink-7)] text-lg mt-3 max-w-xl leading-snug">
      Every run issues a receipt. Read retrieval results and policy verdicts side-by-side.
    </p>
  </header>

  <!-- Composition panel -->
  <div class="panel p-6">
    <SectionHeader label="request" />

    <div class="grid grid-cols-3 gap-4 mb-4">
      <!-- Schema selector -->
      <div>
        <div class="label mb-2">schema</div>
        <select
          class="field"
          bind:value={selectedSchema}
          disabled={loadingSchemas || schemas.length === 0}
        >
          {#if loadingSchemas}
            <option>loading…</option>
          {:else if schemas.length === 0}
            <option>no schemas registered</option>
          {:else}
            {#each schemas as s}
              {@const qualified = `${s.namespace}.${s.name}`}
              <option value={qualified}>{qualified} · v{s.version}</option>
            {/each}
          {/if}
        </select>
      </div>

      <!-- Top K -->
      <div>
        <div class="label mb-2">top_k</div>
        <input
          type="number"
          min="1"
          max="200"
          class="field num"
          placeholder="default"
          bind:value={topK}
        />
      </div>

      <!-- Run -->
      <div class="flex flex-col justify-end">
        <button
          class="btn btn-forge flex items-center justify-center gap-2 py-2.5"
          onclick={runQuery}
          disabled={running}
        >
          {#if running}
            <Loader2 size={14} class="animate-spin" />
            <span>running…</span>
          {:else}
            <Play size={12} strokeWidth={2} />
            <span>run · ⌘↵</span>
          {/if}
        </button>
      </div>
    </div>

    <!-- Query -->
    <div class="mb-4">
      <div class="label mb-2">query string</div>
      <textarea
        class="field serif italic text-lg"
        style="font-size: 17px; min-height: 5rem; resize: vertical;"
        placeholder="what does the setup guide say about postgres?"
        bind:value={query}
        onkeydown={onKey}
      ></textarea>
    </div>

    <!-- Filters -->
    <div>
      <div class="flex items-center justify-between mb-2">
        <span class="label">filters · json</span>
        {#if filtersError}
          <span class="text-[var(--color-denied)] text-[11px]">{filtersError}</span>
        {/if}
      </div>
      <textarea
        class="field code"
        style="font-size: 12px; min-height: 4rem; resize: vertical; background: var(--color-ink-0);"
        placeholder={'{\n  "status": "open"\n}'}
        bind:value={filtersRaw}
        onkeydown={onKey}
      ></textarea>
    </div>
  </div>

  {#if runError}
    <div class="mt-6 panel p-4 border-[var(--color-denied-dim)] bg-[var(--color-denied-bg)] text-[var(--color-denied)] text-[13px]">
      {runError}
    </div>
  {/if}

  <!-- Results -->
  {#if response}
    <section class="mt-10">
      <SectionHeader label="results" count={response.results.length}>
        <span class="label">
          <span class="num text-[var(--color-ink-10)]">{response.elapsed_ms}</span>
          ms · receipt
          <a href="/receipts/{response.receipt_id}" class="link">
            {response.receipt_id.slice(0, 8)}…
          </a>
        </span>
      </SectionHeader>

      {#if response.results.length === 0}
        <EmptyState
          title="No matches."
          body="The retriever returned zero candidates after policy evaluation. Check that the schema has been populated and that your filters aren't over-constraining the search."
        />
      {:else}
        <ol class="space-y-4">
          {#each response.results as hit, i}
            <li class="panel p-5 hover:border-[var(--color-ink-4)] transition-colors">
              <div class="flex items-baseline gap-4 mb-2">
                <span class="num text-[var(--color-ink-7)] text-[12px]">
                  #{(i + 1).toString().padStart(2, '0')}
                </span>
                <span class="num text-[var(--color-forge)] font-medium">
                  {hit.score.toFixed(4)}
                </span>
                <span class="num text-[11px] text-[var(--color-ink-7)] ml-auto">
                  {hit.id}
                </span>
              </div>
              <p class="serif italic text-[var(--color-ink-10)] text-[17px] leading-snug">
                {hit.excerpt || '(no excerpt)'}
              </p>
              {#if hit.metadata && Object.keys(hit.metadata).length > 0}
                <div class="flex flex-wrap gap-x-4 gap-y-1 mt-3 text-[11px]">
                  {#each Object.entries(hit.metadata).slice(0, 6) as [key, value]}
                    <span>
                      <span class="label">{key}</span>
                      <span class="num text-[var(--color-ink-8)] ml-1">
                        {typeof value === 'string' ? value : JSON.stringify(value)}
                      </span>
                    </span>
                  {/each}
                </div>
              {/if}
            </li>
          {/each}
        </ol>

        <div class="mt-6 text-center">
          <a href="/receipts/{response.receipt_id}" class="btn">
            view full receipt →
          </a>
        </div>
      {/if}
    </section>
  {/if}
</div>
