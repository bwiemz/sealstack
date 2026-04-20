<script lang="ts">
  /** Monospace code block with optional copy affordance. */
  interface Props {
    value: string;
    label?: string;
    maxHeight?: string;
  }

  let { value, label, maxHeight = '28rem' }: Props = $props();
  let copied = $state(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      copied = true;
      setTimeout(() => (copied = false), 1200);
    } catch {
      /* no-op */
    }
  }
</script>

<div class="relative">
  {#if label}
    <div class="flex items-center justify-between mb-2">
      <span class="label">{label}</span>
      <button class="btn text-[11px] px-2 py-1" onclick={copy}>
        {copied ? 'copied' : 'copy'}
      </button>
    </div>
  {:else}
    <button
      class="absolute top-2 right-2 btn text-[11px] px-2 py-1 z-10"
      onclick={copy}
    >
      {copied ? 'copied' : 'copy'}
    </button>
  {/if}
  <pre class="code" style="max-height: {maxHeight}; overflow: auto;">{value}</pre>
</div>
