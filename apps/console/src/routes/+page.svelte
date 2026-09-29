<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { SystemInfo, ConnectionSummary, RequestSummary } from '$lib/api.js';
  import { Badge, StatusDot, Button, EmptyState, Spinner } from '$lib/components/index.js';
  import { formatTime } from '$lib/format.js';
  import { m } from '$lib/paraglide/messages.js';
  import { PlusIcon, ArrowRight } from 'lucide-svelte';

  let system = $state<SystemInfo | null>(null);
  let connections = $state<ConnectionSummary[]>([]);
  let requests = $state<RequestSummary[]>([]);
  let loading = $state(true);
  let error = $state('');
  const healthy = $derived(connections.filter((c) => c.status === 'healthy').length);
  const active = $derived(connections.reduce((n, c) => n + c.active_requests, 0));
  const capacity = $derived(connections.reduce((n, c) => n + c.max_concurrent, 0));
  const statusLabels: Record<string, () => string> = { healthy: m.connection_status_healthy, degraded: m.connection_status_degraded, circuit_open: m.connection_status_circuit_open, cooldown: m.connection_status_cooldown, unknown: m.connection_status_unknown };
  const requestLabels: Record<string, () => string> = { completed: m.request_status_completed, failed: m.request_status_failed, partial: m.request_status_partial, cancelled: m.request_status_cancelled, pending: m.request_status_pending };

  function cooldownRemaining(iso: string): string {
    const seconds = Math.max(0, Math.round((new Date(iso).getTime() - Date.now()) / 1000));
    return seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
  }
  function duration(ms: number | null): string { return ms == null ? m.common_pending() : ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(2)}s`; }

  onMount(async () => {
    try {
      const [sys, conns, recent] = await Promise.all([api.system(), api.listConnections(), api.listRequests(6)]);
      system = sys; connections = conns.items; requests = recent.items;
    } catch (e) { error = (e as Error).message; }
    finally { loading = false; }
  });
</script>

<div class="page dashboard">
  <div class="page-header">
    <div><h1>{m.nav_overview()}</h1><p>{m.overview_subtitle()}</p></div>
    {#if !loading && connections.length === 0}<Button size="sm" onclick={() => (window.location.href = '/connections')}><PlusIcon size={14} />{m.connection_add()}</Button>{/if}
  </div>
  {#if loading}<div class="loading"><Spinner size="sm" />{m.common_loading()}</div>
  {:else if error}<p class="error-msg" role="alert">{error}</p>
  {:else if system}
    <section class="vitals" aria-label={m.overview_vitals()}>
      <div><span>{m.system_status()}</span><strong><StatusDot status={system.status} />{system.status}</strong></div>
      <div><span>{m.overview_connection_health()}</span><strong class="mono">{healthy} / {connections.length}</strong></div>
      <div><span>{m.gateway_concurrency()}</span><strong class="mono">{active} / {capacity || '—'}</strong></div>
      <div><span>{m.system_config_revision()}</span><strong class="mono">{system.config_revision}</strong></div>
    </section>

    <div class="panels">
      <section class="panel connections" aria-labelledby="health-heading">
        <div class="panel-head"><h2 id="health-heading">{m.overview_connection_health()}</h2><a href="/connections">{m.overview_manage()} <ArrowRight size={13} /></a></div>
        {#if connections.length === 0}<EmptyState title={m.connection_empty()} description={m.connection_empty_desc()} />
        {:else}<div class="table-wrap"><table><thead><tr><th>{m.connection_provider()}</th><th>{m.connection_id()}</th><th>{m.connection_status()}</th><th>{m.connection_models()}</th><th>{m.gateway_concurrency()}</th><th>{m.overview_condition()}</th></tr></thead><tbody>
          {#each connections as conn (conn.id)}<tr><td class="provider">{conn.provider}</td><td class="mono">{conn.id}</td><td><div class="status"><StatusDot status={conn.status}/><Badge status={conn.status} label={(statusLabels[conn.status] ?? m.connection_status_unknown)()}/></div></td><td class="mono">{conn.model_count}</td><td class="mono">{conn.active_requests} / {conn.max_concurrent}</td><td>{#if conn.cooldown_until}<span class="warning">{cooldownRemaining(conn.cooldown_until)}</span>{:else if conn.failure_count}<span class="warning">{m.connection_failures({n:conn.failure_count})}</span>{:else}—{/if}</td></tr>{/each}
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
  .dashboard { max-width: 1600px; }
  .page-header { display:flex; align-items:flex-start; justify-content:space-between; margin-bottom:18px; }
  .page-header h1 { margin:0 0 3px; }.page-header p{font-size:var(--text-sm);margin:0}.loading{display:flex;gap:8px;align-items:center;color:var(--text-3)}
  .vitals{display:grid;grid-template-columns:repeat(4,1fr);background:var(--border);border:1px solid var(--border);gap:1px;margin-bottom:16px}
  .vitals>div{background:var(--bg-surface);padding:14px 16px;display:flex;flex-direction:column;gap:4px}.vitals span{font-size:var(--text-2xs);color:var(--text-3);text-transform:uppercase;letter-spacing:.06em}.vitals strong{display:flex;align-items:center;gap:8px;font-size:var(--text-md);font-weight:550;text-transform:capitalize}
  .panels{display:grid;grid-template-columns:minmax(0,2fr) minmax(270px,.75fr);align-items:start;gap:16px}.panel{border:1px solid var(--border);background:var(--bg-surface);margin:0;min-width:0}.panel-head{display:flex;align-items:center;justify-content:space-between;padding:11px 13px;border-bottom:1px solid var(--border)}.panel-head h2{border:0;padding:0;margin:0}.panel-head a{display:flex;align-items:center;gap:4px;color:var(--accent);text-decoration:none;font-size:var(--text-xs)}.table-wrap{overflow-x:auto}th,td{white-space:nowrap}.provider{color:var(--text-1);font-weight:550}.status{display:flex;align-items:center;gap:6px}.warning{color:var(--warning)}
  ol{list-style:none;padding:0;margin:0}li{padding:10px 13px;border-bottom:1px solid var(--border);display:grid;gap:6px}li:last-child{border:0}li>div{display:flex;justify-content:space-between;align-items:center;gap:8px}.model{color:var(--text-1);text-decoration:none;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.activity .mono{color:var(--text-3);font-size:var(--text-2xs)}.empty{padding:18px;margin:0}
  @media(max-width:950px){.panels{grid-template-columns:1fr}.vitals{grid-template-columns:repeat(2,1fr)}}@media(max-width:560px){.vitals{grid-template-columns:1fr 1fr}.dashboard{padding:1rem}.page-header p{display:none}}
</style>
