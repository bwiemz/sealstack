<script lang="ts">
  import { Client, ApiError } from '$lib/api';
  import { loadSettings } from '$stores/settings';
  import type { SchemaSummary, SearchResponse } from '$lib/types';
  import SectionHeader from '$components/SectionHeader.svelte';
  import EmptyState from '$components/EmptyState.svelte';
  import { Send, Loader2, MessageSquare, FileText, Trash2 } from 'lucide-svelte';
  import { tick } from 'svelte';

  // --- State --------------------------------------------------------------

  type Turn = {
    id: string;
    question: string;
    schema: string;
    response: SearchResponse | null;
    error: string | null;
    pending: boolean;
  };

  let schemas = $state<SchemaSummary[]>([]);
  let loadingSchemas = $state(true);
  let selectedSchema = $state('');
  let topK = $state<number>(8);

  let composer = $state('');
  let turns = $state<Turn[]>([]);
  let scrollAnchor: HTMLDivElement | undefined = $state();
  let sending = $state(false);

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

  async function send() {
    if (!composer.trim() || sending) return;
    if (!selectedSchema) return;
    const question = composer.trim();
    const turn: Turn = {
      // Date.now is fine here — the page is ephemeral and the id only
      // needs to be unique within the current session.
      id: `t-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`,
      question,
      schema: selectedSchema,
      response: null,
      error: null,
      pending: true
    };
    turns = [...turns, turn];
    composer = '';
    sending = true;
    await tick();
    scrollAnchor?.scrollIntoView({ behavior: 'smooth', block: 'end' });

    try {
      const resp = await clientFor().query({
        schema: selectedSchema,
        query: question,
        top_k: topK
      });
      turns = turns.map((t) => (t.id === turn.id ? { ...t, response: resp, pending: false } : t));
    } catch (e) {
      const msg = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
      turns = turns.map((t) => (t.id === turn.id ? { ...t, error: msg, pending: false } : t));
    } finally {
      sending = false;
      await tick();
      scrollAnchor?.scrollIntoView({ behavior: 'smooth', block: 'end' });
    }
  }

  function onKey(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      send();
    } else if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      send();
    }
  }

  function clearTranscript() {
    turns = [];
  }
</script>

<svelte:head>
  <title>Chat · SealStack</title>
</svelte:head>

<div class="flex flex-col h-screen">
  <!-- Header -->
  <header class="px-10 pt-8 pb-4 border-b border-[var(--color-ink-3)] bg-[var(--color-ink-1)]">
    <div class="flex items-baseline gap-6">
      <div>
        <div class="label">chat</div>
        <h1 class="serif text-4xl text-[var(--color-ink-10)] mt-1 leading-none">conversation.</h1>
      </div>
      <p class="serif italic text-[var(--color-ink-7)] text-base flex-1 max-w-xl">
        Ask, see receipts inline. Each turn is one retrieval call against the
        selected schema — no model in the loop.
      </p>

      <div class="flex items-end gap-3">
        <div>
          <div class="label mb-1">schema</div>
          <select
            class="field"
            bind:value={selectedSchema}
            disabled={loadingSchemas || schemas.length === 0}
          >
            {#if loadingSchemas}
              <option>loading…</option>
            {:else if schemas.length === 0}
              <option>no schemas</option>
            {:else}
              {#each schemas as s}
                {@const qualified = `${s.namespace}.${s.name}`}
                <option value={qualified}>{qualified}</option>
              {/each}
            {/if}
          </select>
        </div>
        <div>
          <div class="label mb-1">top_k</div>
          <input
            type="number"
            min="1"
            max="32"
            class="field num"
            style="width: 4.5rem;"
            bind:value={topK}
          />
        </div>
        {#if turns.length > 0}
          <button class="btn" onclick={clearTranscript} title="Clear transcript">
            <Trash2 size={13} strokeWidth={1.5} />
            <span class="ml-2">clear</span>
          </button>
        {/if}
      </div>
    </div>
  </header>

  <!-- Transcript -->
  <div class="flex-1 overflow-y-auto px-10 py-8">
    {#if turns.length === 0}
      <div class="max-w-2xl mx-auto mt-16">
        <EmptyState
          title="Empty transcript."
          body="Type a question below. Each turn fires a /v1/query against the selected schema and renders the top results inline. Use ⌘↵ or ↵ to send."
        />
      </div>
    {:else}
      <ol class="space-y-8 max-w-3xl mx-auto">
        {#each turns as turn (turn.id)}
          <li class="space-y-4">
            <!-- User turn -->
            <div class="flex gap-3">
              <div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-ink-3)] flex items-center justify-center mt-1">
                <MessageSquare size={14} strokeWidth={1.5} class="text-[var(--color-ink-9)]" />
              </div>
              <div class="flex-1">
                <div class="label mb-1">you · <span class="num">{turn.schema}</span></div>
                <p class="serif italic text-[var(--color-ink-10)] text-lg leading-snug">
                  {turn.question}
                </p>
              </div>
            </div>

            <!-- Assistant turn -->
            <div class="flex gap-3 pl-11">
              {#if turn.pending}
                <div class="flex items-center gap-2 text-[var(--color-ink-7)] text-[13px]">
                  <Loader2 size={14} class="animate-spin" />
                  <span>retrieving…</span>
                </div>
              {:else if turn.error}
                <div class="panel p-4 border-[var(--color-denied-dim)] bg-[var(--color-denied-bg)] text-[var(--color-denied)] text-[13px] w-full">
                  {turn.error}
                </div>
              {:else if turn.response}
                <div class="w-full">
                  <SectionHeader
                    label="sources"
                    count={turn.response.results.length}
                  >
                    <span class="label">
                      <span class="num text-[var(--color-ink-10)]">{turn.response.elapsed_ms}</span>
                      ms · receipt
                      <a
                        href="/receipts/{turn.response.receipt_id}"
                        class="link num"
                      >
                        {turn.response.receipt_id.slice(0, 8)}…
                      </a>
                    </span>
                  </SectionHeader>

                  {#if turn.response.results.length === 0}
                    <p class="serif italic text-[var(--color-ink-7)] text-[15px]">
                      No matches were returned after policy evaluation.
                    </p>
                  {:else}
                    <ol class="space-y-3 mt-2">
                      {#each turn.response.results.slice(0, topK) as hit, i}
                        <li class="panel p-4 hover:border-[var(--color-ink-4)] transition-colors">
                          <div class="flex items-baseline gap-3 mb-2">
                            <span class="num text-[var(--color-ink-7)] text-[11px]">
                              #{(i + 1).toString().padStart(2, '0')}
                            </span>
                            <span class="num text-[var(--color-forge)] text-[12px] font-medium">
                              {hit.score.toFixed(4)}
                            </span>
                            <FileText size={11} strokeWidth={1.5} class="text-[var(--color-ink-7)] ml-auto" />
                            <span class="num text-[10px] text-[var(--color-ink-7)]">
                              {hit.id.slice(0, 12)}…
                            </span>
                          </div>
                          <p class="serif text-[var(--color-ink-10)] text-[15px] leading-snug">
                            {hit.excerpt || '(no excerpt)'}
                          </p>
                        </li>
                      {/each}
                    </ol>
                  {/if}
                </div>
              {/if}
            </div>
          </li>
        {/each}
      </ol>
      <div bind:this={scrollAnchor}></div>
    {/if}
  </div>

  <!-- Composer -->
  <div class="border-t border-[var(--color-ink-3)] bg-[var(--color-ink-1)] px-10 py-4">
    <div class="max-w-3xl mx-auto flex gap-3 items-end">
      <textarea
        class="field serif italic text-base flex-1"
        style="font-size: 16px; min-height: 3rem; max-height: 12rem; resize: vertical;"
        placeholder={selectedSchema
          ? `Ask ${selectedSchema} a question…`
          : 'Choose a schema first.'}
        bind:value={composer}
        onkeydown={onKey}
        disabled={!selectedSchema}
      ></textarea>
      <button
        class="btn btn-forge flex items-center justify-center gap-2 py-2.5 px-4 h-fit"
        onclick={send}
        disabled={sending || !composer.trim() || !selectedSchema}
        title="Send · ↵ or ⌘↵"
      >
        {#if sending}
          <Loader2 size={14} class="animate-spin" />
        {:else}
          <Send size={13} strokeWidth={2} />
        {/if}
        <span>send</span>
      </button>
    </div>
    <p class="text-[11px] text-[var(--color-ink-7)] text-center mt-2">
      <kbd class="num">↵</kbd> send · <kbd class="num">shift+↵</kbd> newline · <kbd class="num">⌘↵</kbd> send
    </p>
  </div>
</div>
