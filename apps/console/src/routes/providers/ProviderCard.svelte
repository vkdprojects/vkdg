<script lang="ts">
  import { ExternalLink, ChevronRight, TriangleAlert } from 'lucide-svelte';
  import type { OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Meter, ProviderLogo } from '$lib/components/index.js';

  /** Per-provider rollup of accounts and connections. */
  export interface Health {
    accounts: number;
    needsLogin: number;
    connections: number;
    connected: number;
    degraded: number;
    error: number;
    active: number;
    max: number;
    configured: boolean;
  }

  interface Props {
    provider: OAuthProvider;
    health: Health;
    categoryLabel: string;
    /** Called before navigating so the list can re-highlight this card on return. */
    onOpen: (id: string) => void;
  }

  let { provider: p, health: h, categoryLabel, onOpen }: Props = $props();

  const healthStates: { key: 'connected' | 'degraded' | 'error'; label: () => string }[] = [
    { key: 'connected', label: m.providers_health_connected },
    { key: 'degraded', label: m.providers_health_degraded },
    { key: 'error', label: m.providers_health_error },
  ];
</script>

<article class="card" data-provider-id={p.id} data-state={h.error ? 'error' : h.degraded ? 'degraded' : h.connected ? 'healthy' : 'idle'}>
  <div class="card-top">
    <ProviderLogo id={p.id} name={p.display_name} fallbackChar={p.icon_char} fallbackColor={p.icon_color} />
    <div class="card-id">
      <a class="provider-link" href="/providers/{encodeURIComponent(p.id)}" onclick={() => onOpen(p.id)} aria-label={m.providers_open_detail({ name: p.display_name })}>{p.display_name}</a>
      <span class="category"><span class="cat-dot" data-cat={p.category} aria-hidden="true"></span>{categoryLabel}</span>
    </div>
    {#if p.site_url}
      <a class="site" href={p.site_url} target="_blank" rel="noopener noreferrer" aria-label={m.providers_open_site({ name: p.display_name })}><ExternalLink size={14} /></a>
    {/if}
    <ChevronRight class="chev" size={16} aria-hidden="true" />
  </div>

  <p class="desc">{p.description ?? p.id}</p>

  <div class="foot">
    <div class="health" aria-live="off">
      {#if h.connections === 0}
        <span class="hchip none">{m.providers_health_none()}</span>
      {:else}
        {#each healthStates as st (st.key)}
          {@const n = h[st.key]}
          {#if n > 0}
            <span class="hchip" data-tone={st.key}><span class="hdot" aria-hidden="true"></span>{#key n}<b class="tick">{n}</b>{/key}{st.label()}</span>
          {/if}
        {/each}
      {/if}
      {#if h.needsLogin > 0}
        <span class="hchip" data-tone="warn"><TriangleAlert size={12} aria-hidden="true" />{m.providers_accounts_need_login({ n: h.needsLogin })}</span>
      {/if}
    </div>

    {#if h.connections > 0}
      <div class="flight">
        <span class="flight-label">{m.providers_in_flight()}</span>
        <span class="flight-num mono">{#key h.active}<b class="tick">{h.active}</b>{/key}/{h.max || '—'}</span>
        <div class="flight-bar">
          <Meter bare value={h.active} limit={h.max || null} ariaLabel={m.providers_in_flight_aria({ active: h.active, max: h.max })} />
        </div>
      </div>
    {/if}
  </div>
</article>

<style>
  :global(.cat-dot) {
    --cat: var(--text-3);
    display: inline-block; flex-shrink: 0; width: var(--space-2); height: var(--space-2);
    border-radius: var(--radius-full); background: var(--cat);
  }
  :global(.cat-dot[data-cat='oauth_ide']) { --cat: var(--accent); }
  :global(.cat-dot[data-cat='llm_api']) { --cat: var(--success); }
  :global(.cat-dot[data-cat='compatible']) { --cat: var(--cooldown); }

  .card {
    --tone: var(--border-strong);
    position: relative; display: flex; flex-direction: column; gap: var(--space-3); min-width: 0;
    padding: var(--space-5); background: var(--bg-surface);
    border: var(--border-w) solid var(--border); border-radius: var(--radius-lg); box-shadow: var(--shadow-1);
    transition: border-color var(--dur-2) var(--ease-out), box-shadow var(--dur-2) var(--ease-out), transform var(--dur-2) var(--ease-out);
  }
  .card::before {
    content: ''; position: absolute; left: 0; top: var(--space-4); bottom: var(--space-4);
    width: var(--indicator-w); border-radius: var(--radius-full); background: var(--tone);
    opacity: 0; transition: opacity var(--dur-2) var(--ease-out), background var(--dur-2) var(--ease-out);
  }
  .card[data-state='healthy'] { --tone: var(--success); }
  .card[data-state='degraded'] { --tone: var(--warning); }
  .card[data-state='error'] { --tone: var(--danger); }
  .card:not([data-state='idle'])::before { opacity: 1; }
  .card:hover { border-color: color-mix(in oklch, var(--accent) 45%, var(--border)); box-shadow: var(--shadow-2); transform: var(--lift); }
  .card:focus-within { border-color: var(--accent); box-shadow: var(--ring); }

  .card-top { display: flex; align-items: center; gap: var(--space-3); min-width: 0; }
  .card-id { flex: 1 1 0; min-width: 0; display: flex; flex-direction: column; gap: var(--space-0); }

  .provider-link {
    color: var(--text-1); text-decoration: none; font-weight: var(--weight-semibold); font-size: var(--text-md);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; outline: none;
  }
  .provider-link:hover { color: var(--accent); }
  /* stretch the primary link over the whole card; the site link stays above it */
  .provider-link::after { content: ''; position: absolute; inset: 0; border-radius: var(--radius-lg); }
  .site {
    position: relative; z-index: var(--z-raised); flex-shrink: 0;
    display: inline-grid; place-items: center; width: var(--control-h-sm); height: var(--control-h-sm);
    border-radius: var(--radius-full); color: var(--text-3); text-decoration: none;
  }
  .site:hover { color: var(--accent); background: var(--bg-hover); }
  .card-top :global(.chev) {
    flex-shrink: 0; color: var(--accent); opacity: 0; transform: translateX(calc(var(--space-2) * -1));
    transition: opacity var(--dur-2) var(--ease-out), transform var(--dur-2) var(--ease-spring);
  }
  .card:hover :global(.chev), .card:focus-within :global(.chev) { opacity: 1; transform: none; }

  .category { display: inline-flex; align-items: center; gap: var(--space-2); font-size: var(--text-xs); color: var(--text-3); }

  .desc {
    margin: 0; font-size: var(--text-sm); color: var(--text-2); line-height: var(--leading);
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
    min-height: var(--space-7);
  }

  /* ── Live footer ── */
  .foot {
    display: flex; flex-direction: column; gap: var(--space-3); margin-top: auto;
    padding-top: var(--space-3); border-top: var(--border-w) solid var(--border);
  }
  .health { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); min-height: var(--space-5); }
  .hchip {
    --t: var(--text-3);
    display: inline-flex; align-items: center; gap: var(--space-1);
    padding: var(--space-0) var(--space-2); border-radius: var(--radius-full);
    font-size: var(--text-xs); font-weight: var(--weight-medium); color: var(--t);
    background: color-mix(in oklch, var(--t) 14%, transparent);
  }
  .hchip[data-tone='connected'] { --t: var(--success); }
  .hchip[data-tone='degraded'] { --t: var(--warning); }
  .hchip[data-tone='error'] { --t: var(--danger); }
  .hchip[data-tone='warn'] { --t: var(--warning); }
  .hchip.none { background: transparent; padding-left: 0; }
  .hdot { width: var(--space-1); height: var(--space-1); border-radius: var(--radius-full); background: var(--t); }
  .hchip b { font-family: var(--font-mono); font-weight: var(--weight-semibold); }

  .flight { display: grid; grid-template-columns: auto 1fr auto; align-items: center; column-gap: var(--space-3); }
  .flight-label { font-size: var(--text-xs); color: var(--text-3); }
  .flight-num { font-size: var(--text-xs); color: var(--text-2); text-align: right; grid-column: 3; grid-row: 1; }
  .flight-bar { grid-column: 2; grid-row: 1; min-width: 0; }

  /* One-shot tick whenever a keyed counter remounts with a new value. */
  :global(.tick) { display: inline-block; font-weight: inherit; animation: pd-tick var(--dur-3) var(--ease-spring); }
  @keyframes -global-pd-tick {
    from { opacity: 0.2; transform: scale(1.35); color: var(--accent); }
  }

  @media (max-width: 480px) {
    .card { padding: var(--space-4); }
  }
  @media (prefers-reduced-motion: reduce) {
    .card, .card:hover { transition: none; transform: none; }
    .card::before, .card-top :global(.chev) { transition: none; }
    :global(.tick) { animation: none; }
  }
</style>
