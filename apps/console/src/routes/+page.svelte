<script lang="ts">
  import type { PageData } from './$types';
  import { Badge, StatusDot, Card, Button, EmptyState } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { PlusIcon } from 'lucide-svelte';

  let { data }: { data: PageData } = $props();

  function formatUptime(secs: number): string {
    const h = Math.floor(secs / 3600);
    const min = Math.floor((secs % 3600) / 60);
    return h > 0 ? `${h}h ${min}m` : `${min}m`;
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_overview()}</h1>
    {#if data.connections.length === 0}
      <Button variant="primary" size="sm" onclick={() => (window.location.href = '/connections')}>
        <PlusIcon size={14} />
        {m.connection_add()}
      </Button>
    {/if}
  </div>

  <div class="stats-bar">
    <Card>
      <div class="stat-card">
        <span class="stat-label">{m.system_version()}</span>
        <span class="stat-value">{data.system.version}</span>
      </div>
    </Card>
    <Card>
      <div class="stat-card">
        <span class="stat-label">{m.system_status()}</span>
        <Badge status={data.system.status} />
      </div>
    </Card>
    <Card>
      <div class="stat-card">
        <span class="stat-label">{m.system_uptime()}</span>
        <span class="stat-value">{formatUptime(data.system.uptime_secs)}</span>
      </div>
    </Card>
    <Card>
      <div class="stat-card">
        <span class="stat-label">{m.system_active_requests()}</span>
        <span class="stat-value">{data.system.active_requests} <span class="stat-unit">requests</span></span>
      </div>
    </Card>
  </div>

  <section aria-labelledby="connections-heading">
    <h2 id="connections-heading" class="section-title">{m.nav_connections()}</h2>
    {#if data.connections.length === 0}
      <EmptyState
        title={m.connection_empty()}
        description="Add a provider connection to start routing requests."
      />
    {:else}
      <div class="conn-grid">
        {#each data.connections as conn (conn.id)}
          <Card>
            <div class="conn-card">
              <div class="conn-header">
                <div class="conn-name">
                  <StatusDot status={conn.status} />
                  <span class="provider">{conn.provider}</span>
                </div>
                <Badge status={conn.status} />
              </div>
              <div class="conn-meta">
                {conn.model_count} models · {conn.active_requests} active
              </div>
              <div class="conn-id">{conn.id}</div>
            </div>
          </Card>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .stats-bar {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
    margin-bottom: 32px;
  }

  .stat-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .stat-label {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .stat-value {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-1);
  }

  .stat-unit {
    font-size: 0.8125rem;
    font-weight: 400;
    color: var(--text-3);
  }

  .section-title {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--text-1);
    margin: 0 0 16px;
  }

  .conn-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
  }

  .conn-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
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
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--text-1);
  }

  .conn-meta {
    font-size: 0.8125rem;
    color: var(--text-2);
  }

  .conn-id {
    font-size: 0.75rem;
    color: var(--text-3);
    font-family: monospace;
  }

  @media (max-width: 768px) {
    .stats-bar {
      grid-template-columns: repeat(2, 1fr);
    }
  }
</style>
