<script lang="ts">
  /**
   * Tiny status indicator — a tinted dot followed by a short label.
   * Used in tables, cards, and anywhere a state needs one-glance clarity.
   *
   * States:
   *   healthy — green. The thing is working right now.
   *   pending — amber, with glow. Something is actively happening.
   *   denied  — red. A request was refused.
   *   muted   — gray. Inert / unknown / not applicable.
   */
  type Kind = 'healthy' | 'pending' | 'denied' | 'muted';

  interface Props {
    kind: Kind;
    label?: string;
  }

  let { kind, label }: Props = $props();
</script>

<span class="inline-flex items-center whitespace-nowrap">
  <span class="dot dot-{kind}"></span>
  {#if label}
    <span class="text-[var(--color-ink-9)]">{label}</span>
  {/if}
</span>
