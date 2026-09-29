<script lang="ts">
  import { onMount } from 'svelte';
  import { ExternalLink } from 'lucide-svelte';
  import { api } from '$lib/api.js';
  import type { Account, ConnectionStatus, ConnectionSummary, OAuthProvider } from '$lib/api.js';
  import { m } from '$lib/paraglide/messages.js';
  import { EmptyState, Meter, Spinner, Stat, StatusDot } from '$lib/components/index.js';

  type Filter = 'all' | 'oauth_ide' | 'llm_api' | 'compatible';

  let providers = $state<OAuthProvider[]>([]);
  let accounts = $state<Account[]>([]);
  let connections = $state<ConnectionSummary[]>([]);
  let loading = $state(true);
  let filter = $state<Filter>('all');

  const filtered = $derived(
    filter === 'all' ? providers : providers.filter((p) => p.category === filter)
  );
  const connectionsByProvider = $derived.by(() => {
    const grouped = new Map<string, ConnectionSummary[]>();
    for (const connection of connections) {
      const group = grouped.get(connection.provider);
      if (group) group.push(connection);
      else grouped.set(connection.provider, [connection]);
    }
    return grouped;
  });

  const statusLabels: Record<ConnectionStatus, () => string> = {
    healthy: m.connection_status_healthy,
    degraded: m.connection_status_degraded,
    circuit_open: m.connection_status_circuit_open,
    cooldown: m.connection_status_cooldown,
    unknown: m.connection_status_unknown,
  };

  const relativeTimeFmt = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });

  function humanizeCooldown(value: string) {
    const timestamp = new Date(value).getTime();
    if (!Number.isFinite(timestamp)) return m.common_none();

    const seconds = Math.round((timestamp - Date.now()) / 1000);
    if (Math.abs(seconds) < 60) return relativeTimeFmt.format(seconds, 'second');
    const minutes = Math.round(seconds / 60);
    if (Math.abs(minutes) < 60) return relativeTimeFmt.format(minutes, 'minute');
    const hours = Math.round(minutes / 60);
    if (Math.abs(hours) < 24) return relativeTimeFmt.format(hours, 'hour');
    return relativeTimeFmt.format(Math.round(hours / 24), 'day');
  }
  function connectionsFor(id: string) {
    return connectionsByProvider.get(id) ?? [];
  }


  /** How many active accounts each provider has */
  function countFor(id: string) {
    return accounts.filter((a) => a.provider === id).length;
  }

  /** Whether any account for this provider needs login */
  function hasAlert(id: string) {
    return accounts.some((a) => a.provider === id && a.status === 'needs_login');
  }

  onMount(async () => {
    try {
      const [pRes, aRes, cRes] = await Promise.all([
        api.oauthProviders(),
        api.listAccounts(),
        api.listConnections(),
      ]);
      providers = pRes.items;
      accounts = aRes.items;
      connections = cRes.items;
    } finally {
      loading = false;
    }
  });

  const FILTERS: { value: Filter; label: () => string }[] = [
    { value: 'all', label: () => m.providers_filter_all() },
    { value: 'oauth_ide', label: () => m.providers_filter_oauth_ide() },
    { value: 'llm_api', label: () => m.providers_filter_llm_api() },
    { value: 'compatible', label: () => m.providers_filter_compatible() },
  ];
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.providers_title()}</h1>
  </div>

  <div class="filters" role="tablist">
    {#each FILTERS as f (f.value)}
      <button
        role="tab"
        aria-selected={filter === f.value}
        class:active={filter === f.value}
        onclick={() => (filter = f.value)}
      >{f.label()}</button>
    {/each}
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if filtered.length === 0}
    <EmptyState title={m.providers_empty()} description={m.providers_connect_first()} />
  {:else}
    <div class="grid">
      {#each filtered as p (p.id)}
        <article class="card">
          <a class="card-link" href="/providers/{encodeURIComponent(p.id)}" aria-label={p.display_name}></a>
          <div class="card-head">
            <span class="icon" style:background={p.icon_color}>{p.icon_char}</span>
            <div class="card-name">
              <span class="name">{p.display_name}</span>
              {#if p.site_url}
                <a
                  class="site-link"
                  href={p.site_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  aria-label="Open {p.display_name} website"
                  onclick={(e) => e.stopPropagation()}
                >
                  <ExternalLink size={12} aria-hidden="true" />
                </a>
              {/if}
            </div>
          </div>

          {#if p.description}
            <p class="desc">{p.description}</p>
          {/if}
          {#if connectionsFor(p.id).length > 0}
            <div class="connection-summary">
              <Stat
                label={m.connection_active_requests()}
                value={connectionsFor(p.id).reduce((sum, connection) => sum + connection.active_requests, 0)}
                unit={`/ ${connectionsFor(p.id).reduce((sum, connection) => sum + connection.max_concurrent, 0)}`}
              />
              {#each connectionsFor(p.id) as connection (connection.id)}
                <div class="connection-row">
                  <div class="connection-head">
                    <span class="connection-name">
                      <StatusDot status={connection.status} size={7} />
                      <span class="connection-id">{connection.id}</span>
                    </span>
                    <span class="connection-status">{(statusLabels[connection.status] ?? m.connection_status_unknown)()}</span>
                  </div>
                  <Meter
                    value={connection.active_requests}
                    limit={connection.max_concurrent}
                    valueText={`${connection.active_requests} / ${connection.max_concurrent}`}
                  />
                  {#if connection.cooldown_until || connection.failure_count != null}
                    <div class="connection-detail">
                      {#if connection.cooldown_until}
                        <span>{m.connection_cooldown_until({ time: humanizeCooldown(connection.cooldown_until) })}</span>
                      {/if}
                      {#if connection.failure_count != null}
                        <span>{m.connection_failures({ n: connection.failure_count })}</span>
                      {/if}
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}


          <div class="card-foot">
            {#if countFor(p.id) > 0}
              <span class="pill connected">{m.providers_connected_count({ n: countFor(p.id) })}</span>
            {:else}
              <span class="pill empty">{m.providers_connect_first()}</span>
            {/if}

            {#if hasAlert(p.id)}
              <span class="pill warn" aria-label="Authentication issue">{m.acct_status_needs_login()}</span>
            {/if}
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>

<style>
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }

  .filters {
    display: flex;
    gap: 8px;
    margin-bottom: 1.5rem;
    flex-wrap: wrap;
  }

  .filters button {
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 9999px;
    color: var(--text-2);
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    padding: 4px 14px;
    transition: background 0.12s, color 0.12s;
  }

  .filters button:hover {
    background: var(--bg-hover);
    color: var(--text-1);
  }

  .filters button.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 0.875rem;
    padding: 16px 0;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 12px;
  }

  .card {
    position: relative;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px;
    text-decoration: none;
    transition: border-color 0.12s, box-shadow 0.12s;
  }
  .card-link {
    position: absolute;
    inset: 0;
    border-radius: inherit;
  }

  .card-link:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .site-link {
    position: relative;
    z-index: 1;
  }


  .card:hover {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .card-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .icon {
    align-items: center;
    border-radius: 8px;
    color: #fff;
    display: flex;
    flex-shrink: 0;
    font-size: 1rem;
    font-weight: 700;
    height: 36px;
    justify-content: center;
    width: 36px;
  }

  .card-name {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }

  .name {
    color: var(--text-1);
    font-size: 0.9375rem;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .site-link {
    color: var(--text-3);
    display: flex;
    flex-shrink: 0;
  }

  .site-link:hover {
    color: var(--accent);
  }

  .desc {
    color: var(--text-3);
    font-size: 0.8125rem;
    line-height: 1.4;
    margin: 0;
    overflow: hidden;
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .connection-summary {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding-top: 0.75rem;
    border-top: 1px solid var(--border);
  }

  .connection-row {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .connection-head,
  .connection-name,
  .connection-detail {
    display: flex;
    align-items: center;
  }

  .connection-head {
    justify-content: space-between;
    gap: 0.75rem;
  }

  .connection-name {
    gap: 0.4rem;
    min-width: 0;
  }

  .connection-id,
  .connection-status,
  .connection-detail {
    font-size: 0.6875rem;
  }

  .connection-id {
    overflow: hidden;
    color: var(--text-2);
    font-family: var(--font-mono);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .connection-status {
    flex-shrink: 0;
    color: var(--text-3);
  }

  .connection-detail {
    flex-wrap: wrap;
    gap: 0.25rem 0.75rem;
    color: var(--text-3);
    font-family: var(--font-mono);
  }

  .card-foot {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: auto;
  }

  .pill {
    border-radius: 9999px;
    font-size: 0.6875rem;
    font-weight: 500;
    padding: 2px 8px;
  }

  .pill.connected {
    background: color-mix(in oklch, var(--success) 15%, transparent);
    color: var(--success);
  }

  .pill.empty {
    background: var(--bg-base);
    color: var(--text-3);
  }

  .pill.warn {
    background: color-mix(in oklch, var(--warning) 15%, transparent);
    color: var(--warning);
  }
</style>
