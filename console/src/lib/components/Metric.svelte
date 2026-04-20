<script lang="ts">
  /**
   * A single metric tile. Giant tabular number in mono, small italic serif
   * label below. Used on the dashboard; reads like a ticker in a trading
   * terminal — exact, not decorative.
   */
  interface Props {
    value: string | number;
    label: string;
    /** Optional qualifier shown small to the right of the value. */
    unit?: string | null;
    /** Optional footnote, italic serif, below the label. */
    note?: string | null;
    /** If true, apply the forge-amber accent to the value (active state). */
    hot?: boolean;
  }

  let { value, label, unit = null, note = null, hot = false }: Props = $props();
</script>

<div class="panel p-5">
  <div class="flex items-baseline gap-2">
    <span
      class="num text-4xl font-medium tracking-tight"
      class:text-[var(--color-forge)]={hot}
      class:text-[var(--color-ink-10)]={!hot}
    >
      {value}
    </span>
    {#if unit}
      <span class="text-xs text-[var(--color-ink-7)] num uppercase tracking-wider">
        {unit}
      </span>
    {/if}
  </div>
  <div class="label mt-3">{label}</div>
  {#if note}
    <div class="serif text-[var(--color-ink-7)] text-[13px] mt-2 leading-snug">
      {note}
    </div>
  {/if}
</div>
