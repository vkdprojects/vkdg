<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { SystemInfo, ConnectionSummary } from '$lib/api.js';
  import { Badge, StatusDot, Card, Button, EmptyState, Spinner, Stat, Meter } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { PlusIcon } from 'lucide-svelte';

  let system = $state<SystemInfo | null>(null);
  let connections = $state<ConnectionSummary[]>([]);
  let loading = $state(true);
  let error = $state('');

  function formatUptime(secs: number): string {
    const h = Math.floor(secs / 3600);
    const min = Math.floor((secs % 3600) / 60);
    return h > 0 ? `${h}h ${min}m` : `${min}m`;
  }

  function cooldownRemaining(iso: string): string {
    const ms = new Date(iso).getTime() - Date.now();
    if (ms <= 0) return '0s';
    const s = Math.round(ms / 1000);
    return s >= 60 ? `${Math.floor(s / 60)}m ${s % 60}s` : `${s}s`;
  }

  onMount(async () => {
    try {
      const [sys, conns] = await Promise.all([
        api.system(),
        api.listConnections(),
      ]);
      system = sys;
      connections = conns.items;
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  });
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_overview()}</h1>
    {#if !loading && connections.length === 0}
      <Button variant="primary" size="sm" onclick={() => (window.location.href = '/connections')}>
        <PlusIcon size={14} />
        {m.connection_add()}
      </Button>
    {/if}
  </div>

  {#if loading}
    <div class="loading"><Spinner size="sm" /> Loading…</div>
  {:else if error}
    <p class="error-msg" role="alert">{error}</p>
  {:else if system}
    {@const status = system.status}
    <div class="stats-bar">
      <Card>
        <Stat label={m.system_status()} value={status}>
          {#snippet icon()}<StatusDot {status} />{/snippet}
        </Stat>
      </Card>
      <Card>
        <Stat label={m.system_version()} value={system.version} />
      </Card>
      <Card>
        <Stat label={m.system_uptime()} value={formatUptime(system.uptime_secs)} />
      </Card>
      <Card>
        <Stat
          label={m.system_active_requests()}
          value={system.active_requests}
          tone={system.active_requests > 0 ? 'success' : 'default'}
        />
      </Card>
    </div>

    <section aria-labelledby="connections-heading">
      <div class="section-head">
        <h2 id="connections-heading" class="section-title">{m.nav_connections()}</h2>
        <span class="section-count mono">{connections.length}</span>
      </div>
      {#if connections.length === 0}
        <EmptyState
          title={m.connection_empty()}
          description={m.connection_empty_desc()}
        />
      {:else}
        <div class="conn-grid">
          {#each connections as conn (conn.id)}
            <Card padding="0">
              <div class="conn-card">
                <div class="conn-header">
                  <div class="conn-name">
                    <StatusDot status={conn.status} />
                    <span class="provider">{conn.provider}</span>
                  </div>
                  <Badge status={conn.status} />
                </div>

                <!-- Concurrency is the one real, tangible "limit" the data plane
                     exposes today (active_requests / max_concurrent slots) —
                     see REDESIGN-NOTES.md §1.1. Credits/quota are not shown
                     because upstream does not report them. -->
                <Meter
                  label={m.connection_active_requests()}
                  value={conn.active_requests}
                  limit={conn.max_concurrent}
                  valueText="{conn.active_requests} / {conn.max_concurrent}"
                  unlimitedText="—"
                />

                <div class="conn-foot">
                  <span class="conn-models mono">{conn.model_count} {m.connection_models().toLowerCase()}</span>
                  {#if conn.status === 'cooldown' && conn.cooldown_until}
                    <span class="conn-cooldown mono">{cooldownRemaining(conn.cooldown_until)}</span>
                  {:else if conn.status === 'circuit_open' && conn.failure_count}
                    <span class="conn-cooldown mono">{m.connection_failures({ n: conn.failure_count })}</span>
                  {/if}
                </div>

                <div class="conn-id mono">{conn.id}</div>
              </div>
            </Card>
          {/each}
        </div>
      {/if}
    </section>
  {/if}
</div>

<style>
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }

  .page-title {
    margin: 0;
  }

  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: var(--text-sm);
    padding: 32px 0;
  }

  .stats-bar {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 1px;
    margin-bottom: 2rem;
    background: var(--border);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .stats-bar > :global(.card) {
    border: none;
    border-radius: 0;
    background-image: none;
    background: var(--bg-surface);
    padding: 1rem 1.25rem;
  }

  .section-head {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    border-bottom: 1px solid var(--border);
    padding-bottom: 0.5rem;
    margin-bottom: 1rem;
  }

  .section-title {
    margin: 0;
    border: none;
    padding: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-1);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .section-count {
    color: var(--text-3);
    font-size: var(--text-xs);
  }

  .conn-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }

  .conn-card {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1rem;
  }

  .conn-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .conn-name {
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .provider {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text-1);
  }

  .conn-foot {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    font-size: var(--text-xs);
  }

  .conn-models {
    color: var(--text-3);
  }

  .conn-cooldown {
    color: var(--warning);
  }

  .conn-id {
    font-size: var(--text-2xs);
    color: var(--text-3);
    border-top: 1px solid var(--border);
    padding-top: 0.5rem;
  }

  @media (max-width: 768px) {
    .stats-bar {
      grid-template-columns: repeat(2, 1fr);
    }
  }
</style>
