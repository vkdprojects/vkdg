<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { SystemInfo, ConnectionSummary, RequestSummary, Account } from '$lib/api.js';
  import { Badge, StatusDot, Button, EmptyState, Spinner, AccountCredits, Stat } from '$lib/components/index.js';
  import { formatTime } from '$lib/format.js';
  import { m } from '$lib/paraglide/messages.js';
  import { PlusIcon, ArrowRight, Network, Gauge, GitCommitHorizontal } from 'lucide-svelte';

  let system = $state<SystemInfo | null>(null);
  let connections = $state<ConnectionSummary[]>([]);
  let accounts = $state<Account[]>([]);
  let requests = $state<RequestSummary[]>([]);
  let loading = $state(true);
  let error = $state('');
  const healthy = $derived(connections.filter((c) => c.status === 'healthy').length);
  const active = $derived(connections.reduce((n, c) => n + c.active_requests, 0));
  const capacity = $derived(connections.reduce((n, c) => n + c.max_concurrent, 0));
  const accountsById = $derived(new Map(accounts.map((a) => [a.id, a])));
  const statusLabels: Record<string, () => string> = { healthy: m.connection_status_healthy, degraded: m.connection_status_degraded, circuit_open: m.connection_status_circuit_open, cooldown: m.connection_status_cooldown, unknown: m.connection_status_unknown };
  const requestLabels: Record<string, () => string> = { completed: m.request_status_completed, failed: m.request_status_failed, partial: m.request_status_partial, cancelled: m.request_status_cancelled, pending: m.request_status_pending };

  function cooldownRemaining(iso: string): string {
    const seconds = Math.max(0, Math.round((new Date(iso).getTime() - Date.now()) / 1000));
    return seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
  }
  function duration(ms: number | null): string { return ms == null ? m.common_pending() : ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(2)}s`; }

  onMount(async () => {
    try {
      const [sys, conns, accts, recent] = await Promise.all([api.system(), api.listConnections(), api.listAccounts(), api.listRequests(6)]);
      system = sys; connections = conns.items; accounts = accts.items; requests = recent.items;
    } catch (e) { error = (e as Error).message; }
    finally { loading = false; }
  });
</script>

<div class="page dashboard">
  <div class="page-header">
    <div><h1>{m.nav_overview()}</h1><p>{m.overview_subtitle()}</p></div>
    {#if !loading && connections.length === 0}<div class="page-actions"><Button size="sm" onclick={() => (window.location.href = '/connections')}><PlusIcon size={14} />{m.connection_add()}</Button></div>{/if}
  </div>
  {#if loading}
    <div class="hero" aria-hidden="true">{#each [0,1,2,3] as i (i)}<div class="tile skeleton"></div>{/each}</div>
    <div class="loading"><Spinner size="sm" />{m.common_loading()}</div>
  {:else if error}<p class="error-msg" role="alert">{error}</p>
  {:else if system}
    <section class="hero" aria-label={m.overview_vitals()}>
      <div class="tile lead">
        <Stat label={m.system_status()} value={system.status} tone={system.status === 'ok' || system.status === 'healthy' ? 'success' : 'default'}>
          {#snippet icon()}<StatusDot status={system?.status ?? 'unknown'} />{/snippet}
        </Stat>
      </div>
      <div class="tile">
        <Stat label={m.overview_connection_health()} value={`${healthy} / ${connections.length}`} tone={connections.length > 0 && healthy < connections.length ? 'warning' : 'default'}>
          {#snippet icon()}<Network size={16} />{/snippet}
        </Stat>
      </div>
      <div class="tile">
        <Stat label={m.gateway_concurrency()} value={`${active} / ${capacity || '—'}`}>
          {#snippet icon()}<Gauge size={16} />{/snippet}
        </Stat>
      </div>
      <div class="tile">
        <Stat label={m.system_config_revision()} value={system.config_revision}>
          {#snippet icon()}<GitCommitHorizontal size={16} />{/snippet}
        </Stat>
      </div>
    </section>

    <div class="panels">
      <section class="panel connections" aria-labelledby="health-heading">
        <div class="panel-head"><h2 id="health-heading">{m.overview_connection_health()}</h2><a href="/connections">{m.overview_manage()} <ArrowRight size={13} /></a></div>
        {#if connections.length === 0}<EmptyState title={m.connection_empty()} description={m.connection_empty_desc()} />
        {:else}<div class="table-wrap"><table><thead><tr><th>{m.connection_provider()}</th><th>{m.connection_id()}</th><th>{m.connection_status()}</th><th>{m.connection_models()}</th><th>{m.gateway_concurrency()}</th><th>{m.acct_credits_column()}</th><th>{m.overview_condition()}</th></tr></thead><tbody>
          {#each connections as conn (conn.id)}{@const acct = conn.account_id ? accountsById.get(conn.account_id) : undefined}<tr><td class="provider">{conn.provider}</td><td class="mono">{conn.id}</td><td><div class="status"><Badge status={conn.status} label={(statusLabels[conn.status] ?? m.connection_status_unknown)()}/></div></td><td class="mono">{conn.model_count}</td><td class="mono">{conn.active_requests} / {conn.max_concurrent}</td><td>{#if acct}<AccountCredits account={acct} variant="compact" />{:else}<span class="no-account">{m.acct_credits_no_account()}</span>{/if}</td><td>{#if conn.cooldown_until}<span class="warning">{cooldownRemaining(conn.cooldown_until)}</span>{:else if conn.failure_count}<span class="warning">{m.connection_failures({n:conn.failure_count})}</span>{:else}—{/if}</td></tr>{/each}
        </tbody></table></div>{/if}
      </section>

      <section class="panel activity" aria-labelledby="activity-heading">
        <div class="panel-head"><h2 id="activity-heading">{m.request_recent()}</h2><a href="/requests">{m.overview_view_all()} <ArrowRight size={13}/></a></div>
        {#if requests.length === 0}<p class="empty">{m.request_empty()}</p>{:else}<ol>{#each requests as request (request.request_id)}<li><div><a href="/requests" class="model">{request.model}</a><span class="mono">{formatTime(request.started_at_ms)}</span></div><div><Badge status={request.status} label={(requestLabels[request.status] ?? (() => request.status))()}/><span class="mono">{duration(request.duration_ms)}</span></div></li>{/each}</ol>{/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .loading { display: flex; gap: var(--space-2); align-items: center; color: var(--text-3); margin-top: var(--space-4); }

  .hero { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-sm)), 1fr)); gap: var(--space-4); margin-bottom: var(--space-5); }
  .tile {
    background: var(--bg-surface); border: var(--border-w) solid var(--border); border-radius: var(--radius-lg);
    box-shadow: var(--shadow-1); padding: var(--space-4) var(--space-5); min-width: 0; min-height: var(--space-8);
  }
  .tile.skeleton { min-height: var(--space-8); border-color: transparent; }
  .tile :global(.stat-value) { text-transform: capitalize; font-size: var(--text-2xl); overflow-wrap: anywhere; }
  /* the one bold moment: gradient wash on the lead tile */
  .tile.lead {
    border-color: color-mix(in oklch, var(--accent) 35%, var(--border));
    background:
      radial-gradient(120% 140% at 0% 0%, color-mix(in oklch, var(--accent) 16%, transparent), transparent 60%),
      var(--bg-surface);
    box-shadow: var(--shadow-1), var(--glow);
  }

  .panels { display: grid; grid-template-columns: minmax(0, 2fr) minmax(min(100%, var(--col-lg)), 1fr); align-items: start; gap: var(--space-4); }
  .panel { min-width: 0; margin: 0; }
  .panel-head a {
    display: inline-flex; align-items: center; gap: var(--space-1); min-height: var(--control-h-sm); padding: 0 var(--space-2);
    border-radius: var(--radius-sm); color: var(--accent); text-decoration: none; font-size: var(--text-xs); font-weight: var(--weight-medium);
    transition: background var(--dur-1) var(--ease-out), gap var(--dur-1) var(--ease-out);
  }
  .panel-head a:hover { background: var(--accent-subtle); gap: var(--space-2); }

  th, td { white-space: nowrap; }
  .provider { color: var(--text-1); font-weight: var(--weight-medium); }
  .status { display: flex; align-items: center; gap: var(--space-2); }
  .warning { color: var(--warning); }
  .no-account { color: var(--text-3); }

  ol { list-style: none; padding: 0; margin: 0; }
  li { padding: var(--space-3) var(--space-4); border-bottom: var(--border-w) solid var(--border); display: grid; gap: var(--space-2); transition: background var(--dur-1) var(--ease-out); }
  li:hover { background: var(--bg-hover); }
  li:last-child { border: 0; }
  li > div { display: flex; justify-content: space-between; align-items: center; gap: var(--space-2); min-width: 0; }
  .model { color: var(--text-1); text-decoration: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: var(--weight-medium); }
  .model:hover { color: var(--accent); }
  .activity .mono { color: var(--text-3); font-size: var(--text-2xs); flex-shrink: 0; }
  .empty { padding: var(--space-5); margin: 0; }

  @media (max-width: 950px) { .panels { grid-template-columns: 1fr; } }
  @media (max-width: 560px) { .hero { gap: var(--space-3); } .tile { padding: var(--space-3) var(--space-4); } .page-header p { display: none; } }
</style>
