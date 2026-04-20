<script lang="ts">
  import '../app.css';
  import { page } from '$app/stores';
  import { Database, Plug, Search, ScrollText, Settings, Activity } from 'lucide-svelte';

  interface Props {
    children: import('svelte').Snippet;
  }

  let { children }: Props = $props();

  const nav = [
    { href: '/',           label: 'overview',    icon: Activity,   match: (p: string) => p === '/' },
    { href: '/schemas',    label: 'schemas',     icon: Database,   match: (p: string) => p.startsWith('/schemas') },
    { href: '/connectors', label: 'connectors',  icon: Plug,       match: (p: string) => p.startsWith('/connectors') },
    { href: '/query',      label: 'query',       icon: Search,     match: (p: string) => p.startsWith('/query') },
    { href: '/receipts',   label: 'receipts',    icon: ScrollText, match: (p: string) => p.startsWith('/receipts') },
    { href: '/settings',   label: 'settings',    icon: Settings,   match: (p: string) => p.startsWith('/settings') }
  ];
</script>

<div class="min-h-screen grid" style="grid-template-columns: 14rem 1fr;">
  <!-- Sidebar -->
  <aside
    class="border-r border-[var(--color-ink-3)] bg-[var(--color-ink-1)] flex flex-col"
    style="height: 100vh; position: sticky; top: 0;"
  >
    <!-- Brand -->
    <div class="px-5 pt-6 pb-5 border-b border-[var(--color-ink-3)]">
      <div class="flex items-baseline gap-2">
        <span class="serif text-2xl text-[var(--color-forge)] leading-none">cf</span>
        <span class="text-[var(--color-ink-10)] text-[15px] font-medium tracking-tight">
          SealStack
        </span>
      </div>
      <div class="label mt-2">console · v0.1</div>
    </div>

    <!-- Nav -->
    <nav class="flex-1 py-3">
      {#each nav as item}
        {@const active = item.match($page.url.pathname)}
        <a
          href={item.href}
          class="flex items-center gap-3 px-5 py-2.5 text-[13px] transition-colors
                 {active
                   ? 'text-[var(--color-ink-10)] border-l-2 border-[var(--color-forge)] bg-[var(--color-ink-2)]'
                   : 'text-[var(--color-ink-8)] border-l-2 border-transparent hover:text-[var(--color-ink-10)] hover:bg-[var(--color-ink-2)]'}"
        >
          <item.icon size={14} strokeWidth={1.5} />
          <span>{item.label}</span>
        </a>
      {/each}
    </nav>

    <!-- Footer / legend -->
    <div class="px-5 py-4 border-t border-[var(--color-ink-3)] text-[11px] text-[var(--color-ink-7)] space-y-1.5">
      <div class="label mb-2">legend</div>
      <div class="flex items-center gap-2">
        <span class="dot dot-healthy"></span><span>healthy</span>
      </div>
      <div class="flex items-center gap-2">
        <span class="dot dot-pending"></span><span>pending</span>
      </div>
      <div class="flex items-center gap-2">
        <span class="dot dot-denied"></span><span>denied</span>
      </div>
    </div>
  </aside>

  <!-- Main -->
  <main class="min-w-0">
    {@render children()}
  </main>
</div>
