<script lang="ts">
  import { tick } from 'svelte';
  import { Search, XIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionSummary, OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Button, EmptyState, Spinner } from '$lib/components/index.js';
  import { poll } from '$lib/live.svelte.js';
  import ProviderCard from './ProviderCard.svelte';
  import type { Health } from './ProviderCard.svelte';

  type Category = 'oauth_ide' | 'llm_api' | 'compatible';
  type Filter = 'all' | Category;
  type Mode = 'all' | 'configured';

  const MODE_KEY = 'vkdg.providers.mode';
  const FLASH_KEY = 'vkdg.providers.flash';

  let providers = $state<OAuthProvider[]>([]);
  let accounts = $state<Account[]>([]);
  let connections = $state<ConnectionSummary[]>([]);
  let loading = $state(true);
  let filter = $state<Filter>('all');
  let query = $state('');
  let mode = $state<Mode>(
    typeof localStorage !== 'undefined' && localStorage.getItem(MODE_KEY) === 'configured' ? 'configured' : 'all',
  );
  let refreshFailed = false;

  const categoryOrder: Category[] = ['oauth_ide', 'llm_api', 'compatible'];
  const categoryLabels: Record<Category, () => string> = {
    oauth_ide: m.providers_filter_oauth_ide,
    llm_api: m.providers_filter_llm_api,
    compatible: m.providers_filter_compatible,
  };
  const modes: { value: Mode; label: () => string }[] = [
    { value: 'all', label: m.providers_mode_all },
    { value: 'configured', label: m.providers_mode_configured },
  ];

  /** Per-provider rollup, rebuilt once per poll instead of per card per render. */
  const health = $derived.by(() => {
    const map = new Map<string, Health>();
    const get = (id: string) => {
      let h = map.get(id);
      if (!h) map.set(id, (h = { accounts: 0, needsLogin: 0, connections: 0, connected: 0, degraded: 0, error: 0, active: 0, max: 0, configured: false }));
      return h;
    };
    for (const a of accounts) {
      const h = get(a.provider);
      h.accounts++;
      if (a.status === 'needs_login') h.needsLogin++;
      h.configured = true;
    }
    for (const c of connections) {
      const h = get(c.provider);
      h.connections++;
      h.configured = true;
      h.active += c.active_requests;
      h.max += c.max_concurrent;
      if (c.status === 'healthy') h.connected++;
      else if (c.status === 'degraded' || c.status === 'cooldown') h.degraded++;
      else if (c.status === 'circuit_open') h.error++;
    }
    return map;
  });
  const empty: Health = { accounts: 0, needsLogin: 0, connections: 0, connected: 0, degraded: 0, error: 0, active: 0, max: 0, configured: false };
  const healthOf = (id: string) => health.get(id) ?? empty;

  const needle = $derived(query.trim().toLowerCase());
  const searched = $derived(needle ? providers.filter((p) => p.display_name.toLowerCase().includes(needle)) : providers);

  /** Pill badges follow the search, so they always say what the grid would show. */
  const pills = $derived.by(() => {
    const count = (list: OAuthProvider[]) => ({ configured: list.filter((p) => healthOf(p.id).configured).length, total: list.length });
    return [
      { value: 'all' as Filter, label: m.providers_filter_all, ...count(searched) },
      ...categoryOrder.map((c) => ({ value: c as Filter, label: categoryLabels[c], ...count(searched.filter((p) => p.category === c)) })),
    ];
  });

  const visible = $derived(
    searched.filter((p) => (filter === 'all' || p.category === filter) && (mode === 'all' || healthOf(p.id).configured)),
  );
  const sections = $derived(
    categoryOrder
      .map((c) => ({ category: c, items: visible.filter((p) => p.category === c) }))
      .filter((s) => s.items.length > 0),
  );

  function setMode(next: Mode) {
    mode = next;
    try {
      localStorage.setItem(MODE_KEY, next);
    } catch {
      // Private mode may deny storage; the choice still applies for this session.
    }
  }
  function clearSearch() {
    query = '';
  }

  async function load(initial = false) {
    try {
      if (initial) {
        const [p, a, c] = await Promise.all([api.oauthProviders(), api.listAccounts(), api.listConnections()]);
        providers = p.items;
        accounts = a.items;
        connections = c.items;
      } else {
        // Provider catalog is static; only live state is refreshed.
        const [a, c] = await Promise.all([api.listAccounts(), api.listConnections()]);
        accounts = a.items;
        connections = c.items;
      }
      refreshFailed = false;
    } catch (e) {
      // One toast per failure streak: a down gateway must not spam every 5s.
      if (initial || !refreshFailed) toast.error(initial ? (e as Error).message : m.providers_refresh_failed());
      refreshFailed = true;
    } finally {
      loading = false;
    }
  }

  /** Remember which card was opened so the list can find it again on back-navigation. */
  function remember(id: string) {
    try {
      sessionStorage.setItem(FLASH_KEY, id);
    } catch {
      // Non-critical: only the return highlight is lost.
    }
  }

  async function flashReturned() {
    let id: string | null = null;
    try {
      id = sessionStorage.getItem(FLASH_KEY);
      sessionStorage.removeItem(FLASH_KEY);
    } catch {
      return;
    }
    if (!id) return;
    await tick();
    const el = document.querySelector<HTMLElement>(`[data-provider-id="${CSS.escape(id)}"]`);
    if (!el) return;
    const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
    el.scrollIntoView({ block: 'center', behavior: reduce ? 'auto' : 'smooth' });
    if (reduce) return;
    el.animate(
      [
        { boxShadow: 'var(--glow)', transform: 'scale(1.015)' },
        { boxShadow: 'var(--shadow-1)', transform: 'scale(1)' },
      ],
      { duration: 900, easing: 'cubic-bezier(0.22, 1, 0.36, 1)' },
    );
  }

  // Provider catalog is static: the first run loads it and highlights the card we came back from.
  let first = true;
  poll(async () => {
    const initial = first;
    first = false;
    await load(initial);
    if (initial) await flashReturned();
  });
</script>

<div class="page inventory">
  <div class="page-header">
    <div>
      <h1>{m.providers_title()}</h1>
      <p>{m.providers_inventory_subtitle()}</p>
    </div>
    <span class="live" title={m.providers_live_hint()}><span class="live-dot" aria-hidden="true"></span>{m.providers_live()}</span>
  </div>

  <div class="toolbar glass">
    <label class="search">
      <Search size={14} aria-hidden="true" />
      <span class="sr-only">{m.providers_search_label()}</span>
      <input type="search" bind:value={query} placeholder={m.providers_search_placeholder()} autocomplete="off" spellcheck="false" />
      {#if query}
        <button type="button" class="clear" onclick={clearSearch} aria-label={m.providers_clear_search()}><XIcon size={14} /></button>
      {/if}
    </label>

    <div class="segmented pills" role="group" aria-label={m.providers_filter_label()}>
      {#each pills as f (f.value)}
        <button
          type="button"
          aria-pressed={filter === f.value}
          aria-label="{f.label()} · {m.providers_configured_total({ configured: f.configured, total: f.total })}"
          onclick={() => (filter = f.value)}
        >
          {f.label()}
          <span class="pill-count mono" aria-hidden="true">{#key f.configured}<b class="tick">{f.configured}</b>{/key}/{f.total}</span>
        </button>
      {/each}
    </div>

    <div class="segmented" role="group" aria-label={m.providers_mode_label()}>
      {#each modes as md (md.value)}
        <button type="button" aria-pressed={mode === md.value} onclick={() => setMode(md.value)}>{md.label()}</button>
      {/each}
    </div>
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" />{m.common_loading()}</div>
  {:else if providers.length === 0}
    <EmptyState title={m.providers_empty()} description={m.providers_connect_first()} />
  {:else if sections.length === 0}
    {#if needle}
      <EmptyState title={m.providers_no_match_title({ query: query.trim() })} description={m.providers_no_match_hint()}>
        {#snippet icon()}<Search size={20} />{/snippet}
        {#snippet action()}<Button variant="outline" onclick={clearSearch}>{m.providers_clear_search()}</Button>{/snippet}
      </EmptyState>
    {:else}
      <EmptyState title={m.providers_none_configured_title()} description={m.providers_none_configured_hint()}>
        {#snippet action()}<Button variant="outline" onclick={() => setMode('all')}>{m.providers_show_all()}</Button>{/snippet}
      </EmptyState>
    {/if}
  {:else}
    {#each sections as s (s.category)}
      <section class="group" aria-labelledby="cat-{s.category}">
        <header class="group-head">
          <span class="cat-dot" data-cat={s.category} aria-hidden="true"></span>
          <h2 id="cat-{s.category}">{categoryLabels[s.category]()}</h2>
          <span class="chip">{#key s.items.length}<b class="tick">{s.items.length}</b>{/key}</span>
        </header>

        <div class="grid-auto" style="--grid-min: var(--col-lg)">
          {#each s.items as p (p.id)}
            <ProviderCard provider={p} health={healthOf(p.id)} categoryLabel={categoryLabels[p.category]?.() ?? p.category} onOpen={remember} />
          {/each}
        </div>
      </section>
    {/each}
  {/if}
</div>

<style>
  .page-header p { font-size: var(--text-sm); }

  .live {
    display: inline-flex; align-items: center; gap: var(--space-2);
    font-size: var(--text-xs); color: var(--text-3); min-height: var(--control-h-sm);
  }
  .live-dot {
    width: var(--space-2); height: var(--space-2); border-radius: var(--radius-full); background: var(--success);
    animation: live-pulse var(--dur-pulse) ease-in-out infinite;
  }
  @keyframes live-pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in oklch, var(--success) 50%, transparent); }
    50% { box-shadow: 0 0 0 var(--space-1) color-mix(in oklch, var(--success) 0%, transparent); }
  }

  /* ── Sticky glass toolbar ───────────────────────────────────────────── */
  .toolbar {
    position: sticky; top: var(--topbar-h); z-index: var(--z-sticky);
    display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-3);
    padding: var(--space-2) var(--space-3); margin-bottom: var(--space-5);
    border-radius: var(--radius-lg);
  }
  .search {
    position: relative; display: flex; flex-direction: row; align-items: center; gap: 0; flex: 1 1 var(--col-md); min-width: 0;
    color: var(--text-3);
  }
  .search > :global(svg) { position: absolute; left: var(--control-px); top: 50%; translate: 0 -50%; pointer-events: none; z-index: var(--z-raised); }
  .search input { padding-left: var(--space-6); padding-right: var(--space-6); min-height: var(--control-h); }
  .search input::-webkit-search-cancel-button { display: none; }
  .clear {
    position: absolute; right: var(--space-1); display: grid; place-items: center;
    width: var(--control-h-sm); height: var(--control-h-sm); border: 0; border-radius: var(--radius-full);
    background: transparent; color: var(--text-3); cursor: pointer;
  }
  .clear:hover { color: var(--text-1); background: var(--bg-hover); }

  /* Segmented groups must scroll inside themselves, never widen the page. */
  .pills { max-width: 100%; overflow-x: auto; scrollbar-width: none; }
  .pills::-webkit-scrollbar { display: none; }
  .segmented > button { display: inline-flex; align-items: center; gap: var(--space-2); white-space: nowrap; cursor: pointer; }
  .pill-count { font-size: var(--text-2xs); color: var(--text-3); }
  .segmented > button[aria-pressed='true'] .pill-count { color: var(--accent); }
  .pill-count :global(b), .chip :global(b) { font-weight: inherit; }

  .loading { display: flex; gap: var(--space-2); align-items: center; color: var(--text-3); padding: var(--space-6) 0; }

  /* ── Category sections ──────────────────────────────────────────────── */
  .group { margin-bottom: var(--space-6); }
  .group-head { display: flex; align-items: center; gap: var(--space-3); margin-bottom: var(--space-4); min-width: 0; }
  .group-head h2 { margin: 0; font-size: var(--text-md); }

  @media (max-width: 480px) {
    .toolbar { top: var(--topbar-h); }
    .toolbar > .segmented { max-width: 100%; }
  }
  @media (prefers-reduced-motion: reduce) {
    .live-dot { animation: none; }
  }
</style>
