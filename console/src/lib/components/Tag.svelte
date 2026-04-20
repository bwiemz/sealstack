<script lang="ts">
  type Tone = 'default' | 'forge' | 'healthy' | 'denied';

  interface Props {
    tone?: Tone;
    children: import('svelte').Snippet;
  }

  let { tone = 'default', children }: Props = $props();
</script>

<span class="tag" class:tag-forge={tone === 'forge'} class:tag-healthy={tone === 'healthy'} class:tag-denied={tone === 'denied'}>
  {@render children()}
</span>
