<script lang="ts">
  /** Spare empty-state block. Italic serif title, mono body, optional CTA. */
  interface Props {
    title: string;
    body?: string;
    children?: import('svelte').Snippet;
  }

  let { title, body, children }: Props = $props();
</script>

<div class="panel p-10 text-center">
  <div class="serif text-2xl text-[var(--color-ink-9)] mb-2">{title}</div>
  {#if body}
    <p class="text-[var(--color-ink-7)] text-[13px] max-w-md mx-auto leading-relaxed">
      {body}
    </p>
  {/if}
  {#if children}
    <div class="mt-5">{@render children()}</div>
  {/if}
</div>
