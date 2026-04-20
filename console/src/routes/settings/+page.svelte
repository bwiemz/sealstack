<script lang="ts">
  import { Client } from '$lib/api';
  import { loadSettings, saveSettings } from '$stores/settings';
  import SectionHeader from '$components/SectionHeader.svelte';
  import Status from '$components/Status.svelte';
  import { Save, CircleCheck } from 'lucide-svelte';

  let gatewayUrl = $state('');
  let user = $state('');
  let saved = $state(false);
  let reachable = $state<boolean | null>(null);
  let checking = $state(false);

  $effect(() => {
    const s = loadSettings();
    gatewayUrl = s.gatewayUrl;
    user = s.user;
  });

  async function checkHealth() {
    checking = true;
    try {
      const c = new Client({ gatewayUrl, user });
      reachable = await c.healthz();
    } finally {
      checking = false;
    }
  }

  function save() {
    saveSettings({ gatewayUrl: gatewayUrl.trim(), user: user.trim() });
    saved = true;
    setTimeout(() => (saved = false), 1500);
    checkHealth();
  }
</script>

<svelte:head>
  <title>Settings · ContextForge</title>
</svelte:head>

<div class="px-10 py-10 max-w-3xl">
  <header class="mb-8">
    <div class="label">settings</div>
    <h1 class="serif text-5xl text-[var(--color-ink-10)] mt-2 leading-none">preferences.</h1>
    <p class="serif italic text-[var(--color-ink-8)] text-lg mt-3 max-w-xl leading-snug">
      Stored in your browser. The gateway enforces its own authentication in
      production — these values are for dev convenience.
    </p>
  </header>

  <SectionHeader label="gateway" />
  <div class="panel p-6 space-y-5">
    <div>
      <div class="label mb-2">base url</div>
      <input
        type="text"
        class="field"
        placeholder="http://localhost:7070"
        bind:value={gatewayUrl}
      />
      <p class="serif italic text-[var(--color-ink-7)] text-[12px] mt-2 leading-relaxed">
        The gateway's public HTTP endpoint. The console makes every request
        from your browser directly to this URL — no proxy.
      </p>
    </div>

    <div>
      <div class="label mb-2">caller · x-cfg-user</div>
      <input type="text" class="field" placeholder="console" bind:value={user} />
      <p class="serif italic text-[var(--color-ink-7)] text-[12px] mt-2 leading-relaxed">
        Identity passed to the gateway on every request via the
        <code class="code inline text-[11px] px-1 py-0.5">X-Cfg-User</code> header.
        Receipts will record this as the caller id.
      </p>
    </div>

    <div class="flex items-center justify-between pt-3 border-t border-[var(--color-ink-3)]">
      <div>
        {#if checking}
          <Status kind="pending" label="checking…" />
        {:else if reachable === true}
          <Status kind="healthy" label="gateway reachable" />
        {:else if reachable === false}
          <Status kind="denied" label="gateway unreachable" />
        {:else}
          <Status kind="muted" label="not checked" />
        {/if}
      </div>
      <div class="flex items-center gap-3">
        <button class="btn" onclick={checkHealth} disabled={checking}>
          check reachability
        </button>
        <button class="btn btn-forge flex items-center gap-2" onclick={save}>
          {#if saved}
            <CircleCheck size={12} strokeWidth={2} /> saved
          {:else}
            <Save size={12} strokeWidth={2} /> save settings
          {/if}
        </button>
      </div>
    </div>
  </div>

  <SectionHeader label="about" />
  <div class="panel p-6 text-[13px] text-[var(--color-ink-9)] leading-relaxed space-y-2">
    <p>
      <span class="label mr-2">build</span>
      <span class="num">console · v0.1 · dev</span>
    </p>
    <p>
      <span class="label mr-2">source</span>
      <a href="https://github.com/bwiemz/contextforge" class="link" target="_blank" rel="noreferrer">
        github.com/bwiemz/contextforge
      </a>
    </p>
    <p>
      <span class="label mr-2">license</span>
      <span class="num">apache-2.0 · core · commercial eula for enterprise</span>
    </p>
  </div>
</div>
