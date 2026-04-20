<script lang="ts">
  import { goto } from '$app/navigation';
  import SectionHeader from '$components/SectionHeader.svelte';

  let receiptId = $state('');

  function view() {
    const id = receiptId.trim();
    if (id) goto(`/receipts/${encodeURIComponent(id)}`);
  }
</script>

<svelte:head>
  <title>Receipts · Signet</title>
</svelte:head>

<div class="px-10 py-10 max-w-3xl">
  <header class="mb-8">
    <div class="label">receipts</div>
    <h1 class="serif text-5xl text-[var(--color-ink-10)] mt-2 leading-none">provenance.</h1>
    <p class="serif italic text-[var(--color-ink-8)] text-lg mt-3 max-w-xl leading-snug">
      Every query issues a receipt: sources read, policies enforced, stage
      timings recorded. Receipts are immutable and signed.
    </p>
  </header>

  <SectionHeader label="find a receipt" />
  <div class="panel p-6">
    <div class="label mb-2">receipt id</div>
    <div class="flex gap-3">
      <input
        type="text"
        class="field num"
        placeholder="01JD5..."
        bind:value={receiptId}
        onkeydown={(e) => e.key === 'Enter' && view()}
      />
      <button class="btn btn-forge" onclick={view} disabled={!receiptId.trim()}>
        view
      </button>
    </div>
    <p class="serif italic text-[var(--color-ink-7)] text-[12px] mt-4 leading-relaxed">
      Receipt ids ride every query response. Run a search in the
      <a href="/query" class="link">playground</a> to generate one.
    </p>
  </div>
</div>
