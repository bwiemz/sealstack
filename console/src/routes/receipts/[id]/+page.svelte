<script lang="ts">
  import { page } from '$app/stores';
  import { client } from '$lib/api';
  import { loadSettings } from '$stores/settings';
  import { Client, ApiError } from '$lib/api';
  import type { Receipt } from '$lib/types';
  import SectionHeader from '$components/SectionHeader.svelte';
  import Tag from '$components/Tag.svelte';
  import EmptyState from '$components/EmptyState.svelte';
  import CodeBlock from '$components/CodeBlock.svelte';
  import { Check, X, Download } from 'lucide-svelte';

  let receipt = $state<Receipt | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);

  $effect(() => {
    const id = $page.params.id;
    if (!id) {
      error = 'missing receipt id';
      loading = false;
      return;
    }
    loading = true;
    error = null;
    receipt = null;
    const settings = loadSettings();
    const c = new Client({ gatewayUrl: settings.gatewayUrl, user: settings.user });
    c.getReceipt(id)
      .then((r) => {
        receipt = r;
      })
      .catch((e) => {
        error = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
      })
      .finally(() => {
        loading = false;
      });
  });

  /** Stage timings as percentages for the stage bar. */
  const timingStages = $derived.by(() => {
    if (!receipt) return [];
    const t = receipt.timings;
    const total = t.total_ms || 1;
    const stages = [
      { name: 'embed',   ms: t.embed_ms,  color: 'bg-[var(--color-ink-5)]' },
      { name: 'vector',  ms: t.vector_ms, color: 'bg-[var(--color-ink-6)]' },
      { name: 'bm25',    ms: t.bm25_ms,   color: 'bg-[var(--color-ink-7)]' },
      { name: 'fuse',    ms: t.fuse_ms,   color: 'bg-[var(--color-forge-dim)]' },
      { name: 'rerank',  ms: t.rerank_ms, color: 'bg-[var(--color-forge)]' },
      { name: 'policy',  ms: t.policy_ms, color: 'bg-[var(--color-ink-8)]' }
    ];
    return stages.map((s) => ({ ...s, pct: (s.ms / total) * 100 }));
  });

  function downloadJson() {
    if (!receipt) return;
    const blob = new Blob([JSON.stringify(receipt, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `receipt-${receipt.id}.json`;
    a.click();
    URL.revokeObjectURL(url);
  }

  function formatTime(iso: string): string {
    try {
      return new Date(iso).toISOString().replace('T', ' ').replace('Z', ' UTC');
    } catch {
      return iso;
    }
  }

  function formatScore(n: number): string {
    return n.toFixed(4);
  }
</script>

<svelte:head>
  <title>Receipt {($page.params.id ?? '').slice(0, 8)}… · SealStack</title>
</svelte:head>

{#if loading}
  <div class="px-10 py-16">
    <div class="shimmer serif text-[var(--color-ink-7)] text-lg">loading receipt…</div>
  </div>
{:else if error}
  <div class="px-10 py-16">
    <EmptyState title="Receipt unavailable" body={error} />
  </div>
{:else if receipt}
  <!-- Document layout — wide margins, centered column. Intentionally reads
       like a tax document or legal receipt rather than a web card. -->
  <div class="max-w-4xl mx-auto px-10 py-12">
    <!-- Masthead -->
    <header class="pb-8 border-b border-[var(--color-ink-4)]">
      <div class="flex items-start justify-between">
        <div>
          <div class="label">sealstack · receipt</div>
          <h1 class="serif text-5xl mt-3 text-[var(--color-ink-10)] leading-none">
            verdict &amp; provenance
          </h1>
          <div class="num text-[13px] text-[var(--color-ink-7)] mt-4">{receipt.id}</div>
        </div>
        <button class="btn" onclick={downloadJson}>
          <Download size={12} strokeWidth={1.75} class="inline mr-1.5 -mt-0.5" />
          json
        </button>
      </div>
    </header>

    <!-- Bibliographic block — reads like a document header -->
    <section class="grid grid-cols-2 gap-x-10 gap-y-4 mt-8 text-[13px]">
      <div>
        <div class="label">issued</div>
        <div class="num mt-1.5 text-[var(--color-ink-10)]">{formatTime(receipt.issued_at)}</div>
      </div>
      <div>
        <div class="label">caller</div>
        <div class="num mt-1.5 text-[var(--color-ink-10)]">
          {receipt.caller.id}
          {#if receipt.caller.tenant}
            <span class="text-[var(--color-ink-7)]"> @ {receipt.caller.tenant}</span>
          {/if}
        </div>
        {#if receipt.caller.roles.length > 0}
          <div class="flex flex-wrap gap-1 mt-2">
            {#each receipt.caller.roles as role}
              <Tag>{role}</Tag>
            {/each}
          </div>
        {/if}
      </div>
      <div>
        <div class="label">schema</div>
        <div class="num mt-1.5 text-[var(--color-ink-10)]">
          {receipt.schema.namespace}.{receipt.schema.name}
          <span class="text-[var(--color-ink-7)]"> v{receipt.schema.version}</span>
        </div>
      </div>
      <div>
        <div class="label">elapsed</div>
        <div class="num mt-1.5 text-[var(--color-ink-10)]">
          {receipt.timings.total_ms}<span class="text-[var(--color-ink-7)]"> ms</span>
        </div>
      </div>
    </section>

    <!-- Query block — the question being audited -->
    <section class="mt-10">
      <SectionHeader label="query" />
      <blockquote class="serif text-2xl text-[var(--color-ink-10)] leading-snug italic pl-4 border-l-2 border-[var(--color-forge)]">
        “{receipt.query}”
      </blockquote>
      {#if receipt.filters && Object.keys(receipt.filters).length > 0}
        <div class="mt-4">
          <div class="label mb-2">filters</div>
          <pre class="code text-[12px]">{JSON.stringify(receipt.filters, null, 2)}</pre>
        </div>
      {/if}
    </section>

    <!-- Timings stage bar — a ticker-style breakdown -->
    <section class="mt-10">
      <SectionHeader label="latency breakdown" count={receipt.timings.total_ms}>
        <span class="label">ms · total</span>
      </SectionHeader>
      <!-- Stacked bar -->
      <div class="flex h-8 border border-[var(--color-ink-4)] overflow-hidden">
        {#each timingStages as stage}
          {#if stage.pct > 0.5}
            <div
              class="{stage.color} flex items-center justify-center text-[10px] text-[var(--color-ink-0)] font-medium uppercase tracking-wider"
              style="width: {stage.pct}%"
              title="{stage.name} · {stage.ms}ms"
            >
              {#if stage.pct > 8}
                {stage.name}
              {/if}
            </div>
          {/if}
        {/each}
      </div>
      <!-- Legend -->
      <div class="grid grid-cols-6 gap-2 mt-3 text-[11px]">
        {#each timingStages as stage}
          <div>
            <div class="flex items-center gap-1.5">
              <span class="w-2 h-2 {stage.color} inline-block"></span>
              <span class="label">{stage.name}</span>
            </div>
            <div class="num text-[var(--color-ink-9)] mt-0.5">{stage.ms}<span class="text-[var(--color-ink-7)]">ms</span></div>
          </div>
        {/each}
      </div>
    </section>

    <!-- Policy verdicts — diff-style green allow / red deny -->
    <section class="mt-10">
      <SectionHeader label="policy verdicts" count={receipt.verdicts.length} />
      {#if receipt.verdicts.length === 0}
        <p class="serif text-[var(--color-ink-7)]">No policy rules evaluated.</p>
      {:else}
        <ul class="font-mono text-[13px] border border-[var(--color-ink-3)]">
          {#each receipt.verdicts as v, i}
            <li
              class="flex items-start gap-3 px-4 py-2.5 border-b border-[var(--color-ink-3)] last:border-b-0"
              class:bg-[var(--color-healthy-bg)]={v.decision === 'allow'}
              class:bg-[var(--color-denied-bg)]={v.decision === 'deny'}
            >
              <span
                class="shrink-0 mt-0.5"
                class:text-[var(--color-healthy)]={v.decision === 'allow'}
                class:text-[var(--color-denied)]={v.decision === 'deny'}
              >
                {#if v.decision === 'allow'}
                  <Check size={14} strokeWidth={2} />
                {:else}
                  <X size={14} strokeWidth={2} />
                {/if}
              </span>
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2">
                  <span class="num text-[var(--color-ink-7)] text-[11px]">#{(i + 1).toString().padStart(2, '0')}</span>
                  <span
                    class="font-medium"
                    class:text-[var(--color-healthy)]={v.decision === 'allow'}
                    class:text-[var(--color-denied)]={v.decision === 'deny'}
                  >
                    {v.decision}
                  </span>
                  <span class="text-[var(--color-ink-10)]">{v.rule}</span>
                </div>
                {#if v.message}
                  <div class="serif italic text-[var(--color-ink-7)] text-[12px] mt-1 pl-6">
                    {v.message}
                  </div>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <!-- Sources — provenance table -->
    <section class="mt-10">
      <SectionHeader label="sources" count={receipt.sources.length}>
        <span class="label">provenance</span>
      </SectionHeader>
      {#if receipt.sources.length === 0}
        <p class="serif text-[var(--color-ink-7)]">No sources matched.</p>
      {:else}
        <div class="panel overflow-hidden">
          <table class="data">
            <thead>
              <tr>
                <th style="width: 3rem">#</th>
                <th style="width: 7rem">score</th>
                <th>excerpt</th>
                <th style="width: 9rem">record</th>
              </tr>
            </thead>
            <tbody>
              {#each receipt.sources as src, i}
                <tr>
                  <td class="num text-[var(--color-ink-7)]">{(i + 1).toString().padStart(2, '0')}</td>
                  <td>
                    <div class="num text-[var(--color-ink-10)]">{formatScore(src.score)}</div>
                    {#if src.rerank_score !== undefined}
                      <div class="num text-[10px] text-[var(--color-ink-7)] mt-0.5">
                        rr {formatScore(src.rerank_score)}
                      </div>
                    {/if}
                    {#if src.freshness_boost !== undefined}
                      <div class="num text-[10px] text-[var(--color-ink-7)]">
                        fr {src.freshness_boost.toFixed(3)}
                      </div>
                    {/if}
                  </td>
                  <td>
                    <div class="serif italic text-[var(--color-ink-10)] leading-snug">
                      {src.excerpt}
                    </div>
                  </td>
                  <td class="num text-[11px] text-[var(--color-ink-8)] break-all">
                    {src.record_id}
                    {#if src.chunk_id}
                      <div class="text-[var(--color-ink-7)] mt-0.5">chunk {src.chunk_id.slice(0, 8)}…</div>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>

    <!-- Signed footer -->
    <footer class="mt-12 pt-6 border-t border-[var(--color-ink-4)]">
      <div class="grid grid-cols-2 gap-8 items-end">
        <div>
          <div class="label">signature</div>
          {#if receipt.signature}
            <div class="num text-[11px] text-[var(--color-ink-8)] mt-2 break-all">
              {receipt.signature}
            </div>
          {:else}
            <div class="serif italic text-[var(--color-ink-7)] text-[13px] mt-2">
              Unsigned. This receipt was issued by a development instance.
            </div>
          {/if}
        </div>
        <div class="text-right">
          <div class="serif italic text-[var(--color-ink-7)] text-[13px]">
            Retained {formatTime(receipt.issued_at)}<br />
            under CSL policy in effect at issue.
          </div>
        </div>
      </div>
    </footer>
  </div>
{/if}
