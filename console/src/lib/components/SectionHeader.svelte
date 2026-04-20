<script lang="ts">
  /**
   * The recurring section-divider device. A short italic serif label in small
   * caps, with a horizontal rule flowing out of it. Used to break the UI into
   * named zones: "schema", "connectors", "verdicts", etc.
   */
  interface Props {
    label: string;
    count?: number | null;
    /** Optional slot-like trailing content via children snippet. */
    children?: import('svelte').Snippet;
  }

  let { label, count = null, children }: Props = $props();
</script>

<div class="rule my-5">
  <span class="label">
    {label}
    {#if count !== null}
      <span class="num text-[var(--color-ink-8)] ml-1">{count}</span>
    {/if}
  </span>
  {#if children}
    <span class="ml-auto pl-3">{@render children()}</span>
  {/if}
</div>
