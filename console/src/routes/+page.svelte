<script lang="ts">
  import { Client } from '$lib/api';
  import { loadSettings } from '$stores/settings';
  import type { ConnectorBinding, SchemaSummary } from '$lib/types';
  import Metric from '$components/Metric.svelte';
  import SectionHeader from '$components/SectionHeader.svelte';
  import Status from '$components/Status.svelte';
  import Tag from '$components/Tag.svelte';
  import { ArrowUpRight } from 'lucide-svelte';

  let healthy = $state<boolean | null>(null);
  let schemas = $state<SchemaSummary[]>([]);
  let connectors = $state<ConnectorBinding[]>([]);
  let loading = $state(true);

  $effect(() => {
    const s = loadSettings();
    const c = new Client({ gatewayUrl: s.gatewayUrl, user: s.user });
    Promise.all([
      c.healthz().then((h) => (healthy = h)),
      c.listSchemas().then((ss) => (schemas = ss)).catch(() => {}),
      c.listConnectors().then((cs) => (connectors = cs)).catch(() => {})
    ]).finally(() => (loading = false));
  });

  const healthStatus = $derived(
    healthy === null ? 'muted' : healthy ? 'healthy' : 'denied'
  );
  const healthLabel = $derived(
    healthy === null ? 'checking…' : healthy ? 'gateway ready' : 'gateway unreachable'
  );
</script>

<svelte:head>
  <title>Overview · SealStack</title>
</svelte:head>

<div class="px-10 py-10">
  <!-- Masthead -->
  <header class="mb-10">
    <div class="flex items-baseline justify-between gap-6">
      <div>
        <div class="label">sealstack · overview</div>
        <h1 class="serif text-6xl text-[var(--color-ink-10)] mt-2 leading-none">
          <span class="text-[var(--color-forge)]">context</span>, engineered.
        </h1>
        <p class="serif italic text-[var(--color-ink-8)] text-lg mt-4 max-w-2xl leading-snug">
          The platform knows what your tools know. Every result carries a receipt —
          the sources it read, the policies it enforced, and the time each stage cost.
        </p>
      </div>
      <div class="text-right">
        <Status kind={healthStatus} label={healthLabel} />
      </div>
    </div>
  </header>

  <!-- Metrics -->
  <SectionHeader label="at a glance" />
  <div class="grid grid-cols-4 gap-4">
    <Metric
      value={loading ? '—' : schemas.length}
      label="schemas"
      note="Registered with the gateway. Compiled from CSL, backed by Postgres tables."
    />
    <Metric
      value={loading ? '—' : connectors.length}
      label="connectors"
      note="Bindings to data sources. Each pulls into one schema."
      hot={!loading && connectors.length > 0}
    />
    <Metric
      value={loading ? '—' : connectors.filter((c) => c.interval_secs !== null).length}
      label="scheduled syncs"
      note="Connectors with a non-zero interval. The rest run on demand."
    />
    <Metric
      value={healthy === true ? 'ready' : healthy === false ? 'down' : '—'}
      label="gateway"
      note="Availability of the configured SealStack gateway endpoint."
      hot={healthy === false}
    />
  </div>

  <!-- Activity rows -->
  <div class="grid grid-cols-2 gap-8 mt-12">
    <!-- Recent schemas -->
    <section>
      <SectionHeader label="schemas">
        <a href="/schemas" class="label hover:text-[var(--color-ink-10)] transition-colors">
          all schemas ↗
        </a>
      </SectionHeader>
      {#if loading}
        <p class="serif italic text-[var(--color-ink-7)]">loading…</p>
      {:else if schemas.length === 0}
        <div class="panel p-6">
          <p class="serif italic text-[var(--color-ink-7)] mb-3">
            No schemas registered yet.
          </p>
          <div class="code text-[12px]">sealstack schema apply schemas/doc.csl</div>
        </div>
      {:else}
        <div class="panel overflow-hidden">
          <table class="data">
            <thead>
              <tr>
                <th>qualified</th>
                <th style="width: 4rem">ver</th>
                <th style="width: 6rem">facets</th>
              </tr>
            </thead>
            <tbody>
              {#each schemas.slice(0, 5) as s}
                <tr>
                  <td>
                    <a href="/schemas/{s.namespace}.{s.name}" class="link">
                      {s.namespace}.{s.name}
                    </a>
                  </td>
                  <td class="num text-[var(--color-ink-8)]">v{s.version}</td>
                  <td class="num text-[var(--color-ink-8)]">
                    {s.facets?.length ?? 0}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>

    <!-- Recent connectors -->
    <section>
      <SectionHeader label="connectors">
        <a href="/connectors" class="label hover:text-[var(--color-ink-10)] transition-colors">
          all connectors ↗
        </a>
      </SectionHeader>
      {#if loading}
        <p class="serif italic text-[var(--color-ink-7)]">loading…</p>
      {:else if connectors.length === 0}
        <div class="panel p-6">
          <p class="serif italic text-[var(--color-ink-7)] mb-3">
            No connectors bound yet.
          </p>
          <div class="code text-[12px]">
            sealstack connector add local-files \
              --schema examples.Doc \
              --root ./sample-docs
          </div>
        </div>
      {:else}
        <div class="panel overflow-hidden">
          <table class="data">
            <thead>
              <tr>
                <th>binding</th>
                <th style="width: 6rem">kind</th>
              </tr>
            </thead>
            <tbody>
              {#each connectors.slice(0, 5) as c}
                <tr>
                  <td>
                    <div class="num text-[var(--color-ink-10)]">{c.id}</div>
                    <div class="label mt-1">{c.namespace}.{c.schema}</div>
                  </td>
                  <td><Tag>{c.connector}</Tag></td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>
  </div>

  <!-- Quickstart — surfaces on empty state but always available -->
  <section class="mt-12">
    <SectionHeader label="quickstart" />
    <div class="grid grid-cols-3 gap-4">
      <a href="/schemas" class="panel p-5 hover:border-[var(--color-ink-5)] transition-colors group block">
        <div class="flex items-start justify-between mb-3">
          <div class="serif text-2xl text-[var(--color-ink-10)]">schemas</div>
          <ArrowUpRight size={14} class="text-[var(--color-ink-7)] group-hover:text-[var(--color-forge)] transition-colors" />
        </div>
        <p class="text-[13px] text-[var(--color-ink-7)] leading-relaxed">
          Inspect compiled CSL schemas. Every registered schema emits a Postgres
          table, a vector collection, and an MCP tool.
        </p>
      </a>

      <a href="/connectors" class="panel p-5 hover:border-[var(--color-ink-5)] transition-colors group block">
        <div class="flex items-start justify-between mb-3">
          <div class="serif text-2xl text-[var(--color-ink-10)]">connectors</div>
          <ArrowUpRight size={14} class="text-[var(--color-ink-7)] group-hover:text-[var(--color-forge)] transition-colors" />
        </div>
        <p class="text-[13px] text-[var(--color-ink-7)] leading-relaxed">
          Bind data sources to schemas. Trigger one-shot or scheduled syncs.
          Watch resources flow into the engine in real time.
        </p>
      </a>

      <a href="/query" class="panel p-5 hover:border-[var(--color-ink-5)] transition-colors group block">
        <div class="flex items-start justify-between mb-3">
          <div class="serif text-2xl text-[var(--color-forge)]">query</div>
          <ArrowUpRight size={14} class="text-[var(--color-forge)]" />
        </div>
        <p class="text-[13px] text-[var(--color-ink-7)] leading-relaxed">
          Run hybrid search against any schema. Each result links to a full
          receipt — provenance, policy verdicts, stage timings.
        </p>
      </a>
    </div>
  </section>
</div>
