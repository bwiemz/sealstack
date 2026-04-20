<script lang="ts">
  import { Client, ApiError } from '$lib/api';
  import { loadSettings } from '$stores/settings';
  import type { ConnectorBinding, SchemaSummary, SyncOutcome } from '$lib/types';
  import SectionHeader from '$components/SectionHeader.svelte';
  import EmptyState from '$components/EmptyState.svelte';
  import Tag from '$components/Tag.svelte';
  import Status from '$components/Status.svelte';
  import { Plus, RefreshCw, Loader2, X } from 'lucide-svelte';

  // --- State --------------------------------------------------------------

  let bindings = $state<ConnectorBinding[]>([]);
  let schemas = $state<SchemaSummary[]>([]);
  let loading = $state(true);
  let listError = $state<string | null>(null);

  // Add form
  let showForm = $state(false);
  let kind = $state('local-files');
  let schema = $state('');
  let root = $state('');
  let submitting = $state(false);
  let formError = $state<string | null>(null);

  // Per-binding sync state — keyed by binding id
  let syncing = $state<Record<string, boolean>>({});
  let outcomes = $state<Record<string, SyncOutcome>>({});
  let syncErrors = $state<Record<string, string>>({});

  const clientFor = () => {
    const s = loadSettings();
    return new Client({ gatewayUrl: s.gatewayUrl, user: s.user });
  };

  async function refresh() {
    const c = clientFor();
    try {
      const [bs, ss] = await Promise.all([c.listConnectors(), c.listSchemas()]);
      bindings = bs;
      schemas = ss;
      if (!schema && ss.length > 0) {
        schema = `${ss[0].namespace}.${ss[0].name}`;
      }
      listError = null;
    } catch (e) {
      listError = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    refresh();
  });

  async function addBinding() {
    formError = null;
    if (!kind.trim() || !schema.trim()) {
      formError = 'kind and schema are required.';
      return;
    }
    if (kind === 'local-files' && !root.trim()) {
      formError = 'local-files requires a root path.';
      return;
    }
    submitting = true;
    try {
      const config: Record<string, unknown> = {};
      if (kind === 'local-files') config.root = root.trim();
      await clientFor().registerConnector({ kind, schema, config });
      showForm = false;
      kind = 'local-files';
      root = '';
      await refresh();
    } catch (e) {
      formError = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    } finally {
      submitting = false;
    }
  }

  async function sync(id: string) {
    syncing[id] = true;
    delete syncErrors[id];
    try {
      outcomes[id] = await clientFor().syncConnector(id);
    } catch (e) {
      syncErrors[id] = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    } finally {
      syncing[id] = false;
    }
  }
</script>

<svelte:head>
  <title>Connectors · Signet</title>
</svelte:head>

<div class="px-10 py-10">
  <header class="mb-8 flex items-baseline justify-between gap-6">
    <div>
      <div class="label">connectors</div>
      <h1 class="serif text-5xl text-[var(--color-ink-10)] mt-2 leading-none">bindings.</h1>
      <p class="serif italic text-[var(--color-ink-8)] text-lg mt-3 max-w-xl leading-snug">
        Each binding couples a data source to a target schema. Syncs can run on
        demand or on a cadence.
      </p>
    </div>
    <button
      class="btn btn-forge flex items-center gap-2"
      onclick={() => (showForm = !showForm)}
    >
      {#if showForm}
        <X size={12} strokeWidth={2} /> cancel
      {:else}
        <Plus size={12} strokeWidth={2} /> new binding
      {/if}
    </button>
  </header>

  {#if showForm}
    <div class="panel p-6 mb-8">
      <SectionHeader label="new binding" />
      <div class="grid grid-cols-3 gap-4">
        <div>
          <div class="label mb-2">kind</div>
          <select class="field" bind:value={kind}>
            <option value="local-files">local-files</option>
            <option value="github">github · not yet implemented</option>
            <option value="slack">slack · not yet implemented</option>
          </select>
        </div>
        <div>
          <div class="label mb-2">target schema</div>
          <select class="field" bind:value={schema} disabled={schemas.length === 0}>
            {#if schemas.length === 0}
              <option>no schemas registered</option>
            {:else}
              {#each schemas as s}
                <option value="{s.namespace}.{s.name}">{s.namespace}.{s.name}</option>
              {/each}
            {/if}
          </select>
        </div>
        {#if kind === 'local-files'}
          <div>
            <div class="label mb-2">root path</div>
            <input
              type="text"
              class="field"
              placeholder="/abs/path/to/docs"
              bind:value={root}
            />
          </div>
        {/if}
      </div>
      {#if formError}
        <div class="mt-4 text-[var(--color-denied)] text-[13px]">{formError}</div>
      {/if}
      <div class="mt-6 flex justify-end gap-3">
        <button class="btn" onclick={() => (showForm = false)} disabled={submitting}>
          cancel
        </button>
        <button class="btn btn-forge" onclick={addBinding} disabled={submitting}>
          {submitting ? 'registering…' : 'register binding'}
        </button>
      </div>
    </div>
  {/if}

  <SectionHeader label="inventory" count={bindings.length} />

  {#if loading}
    <p class="serif italic text-[var(--color-ink-7)] shimmer">loading…</p>
  {:else if listError}
    <EmptyState title="Cannot reach gateway" body={listError} />
  {:else if bindings.length === 0}
    <EmptyState
      title="No bindings yet."
      body="A binding links a connector instance to a target schema. Register one above, or use the CLI."
    />
  {:else}
    <div class="panel overflow-hidden">
      <table class="data">
        <thead>
          <tr>
            <th>binding id</th>
            <th style="width: 8rem">kind</th>
            <th style="width: 12rem">target</th>
            <th style="width: 7rem">interval</th>
            <th style="width: 10rem">last sync</th>
            <th style="width: 8rem"></th>
          </tr>
        </thead>
        <tbody>
          {#each bindings as b}
            {@const outcome = outcomes[b.id]}
            {@const err = syncErrors[b.id]}
            <tr>
              <td>
                <div class="num text-[var(--color-ink-10)]">{b.id}</div>
                {#if b.version}
                  <div class="text-[11px] text-[var(--color-ink-7)] num mt-0.5">
                    v{b.version}
                  </div>
                {/if}
              </td>
              <td><Tag>{b.connector}</Tag></td>
              <td class="num text-[var(--color-ink-9)]">
                <a href="/schemas/{b.namespace}.{b.schema}" class="link">
                  {b.namespace}.{b.schema}
                </a>
              </td>
              <td class="num text-[var(--color-ink-8)]">
                {b.interval_secs !== null ? `${b.interval_secs}s` : 'on-demand'}
              </td>
              <td>
                {#if err}
                  <Status kind="denied" label={err.slice(0, 30)} />
                {:else if outcome}
                  {#if outcome.kind === 'completed'}
                    <Status kind="healthy" />
                    <div class="num text-[11px] text-[var(--color-ink-8)] mt-0.5">
                      {outcome.resources_ingested}/{outcome.resources_seen} · {outcome.elapsed_ms}ms
                    </div>
                  {:else}
                    <Status kind="denied" label={outcome.kind} />
                  {/if}
                {:else}
                  <span class="num text-[var(--color-ink-7)]">—</span>
                {/if}
              </td>
              <td>
                <button
                  class="btn flex items-center gap-1.5"
                  onclick={() => sync(b.id)}
                  disabled={syncing[b.id]}
                >
                  {#if syncing[b.id]}
                    <Loader2 size={12} class="animate-spin" /> syncing
                  {:else}
                    <RefreshCw size={12} strokeWidth={2} /> sync
                  {/if}
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>
