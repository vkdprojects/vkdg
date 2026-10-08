<script lang="ts">
  import { api } from '$lib/api.js';
  import type { ConnectionSummary } from '$lib/api.js';
  import { AddConnectionDialog, Badge, Button, ConfirmDeleteDialog, ConnectionModelsDialog, EmptyState, Meter, RefreshButton, Spinner, Stat } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { formatTime, formatRelativeFrom } from '$lib/format.js';
  import { deleteConnection, syncConnectionModels } from '$lib/connection-actions.js';
  import { connectionStatusLabel, isCooling } from '$lib/status.js';
  import { poll } from '$lib/live.svelte.js';
  import { PlusIcon, RefreshCwIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';

  let connections = $state<ConnectionSummary[]>([]);
  let loading = $state(true);
  let refreshing = $state(false);
  let updatedAt = $state<Date | null>(null);
  let dialogOpen = $state(false);

  const totalActive = $derived(connections.reduce((sum, conn) => sum + conn.active_requests, 0));
  const totalCapacity = $derived(connections.reduce((sum, conn) => sum + conn.max_concurrent, 0));

  async function load() {
    refreshing = true;
    try {
      connections = (await api.listConnections()).items;
      updatedAt = new Date();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  // Every 5s while the tab is visible; refresh at once when it comes back.
  poll(load);

  let pendingDelete = $state<ConnectionSummary | null>(null);
  let deleting = $state(false);

  async function confirmDelete() {
    if (!pendingDelete) return;
    deleting = true;
    const ok = await deleteConnection(pendingDelete.id);
    deleting = false;
    if (!ok) return;
    pendingDelete = null;
    await load();
  }

  // Models chips: show a few entries; "+N more" opens a dialog with the full list.
  const MODEL_CHIPS = 3;
  let modelsDialogOpen = $state(false);
  let modelsDialogConn = $state<ConnectionSummary | null>(null);

  function showAllModels(conn: ConnectionSummary) {
    modelsDialogConn = conn;
    modelsDialogOpen = true;
  }
  const isPattern = (id: string) => /[*?]/.test(id);

  let syncingId = $state<string | null>(null);

  async function syncModels(conn: ConnectionSummary) {
    syncingId = conn.id;
    const ok = await syncConnectionModels(conn.id);
    syncingId = null;
    if (ok) await load();
  }

  let resettingId = $state<string | null>(null);

  async function resetCooldown(conn: ConnectionSummary) {
    resettingId = conn.id;
    try {
      await api.resetConnectionCooldown(conn.id);
      toast.success(m.connection_cooldown_reset({ id: conn.id }));
      await load();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      resettingId = null;
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1 class="page-title">{m.nav_connections()}</h1>
      <p class="refresh-note" aria-live="polite">
        {m.connection_auto_refresh()}
        {#if updatedAt}{m.common_updated_at({ time: formatTime(updatedAt) })}{/if}
      </p>
    </div>
    <div class="page-actions">
      <RefreshButton busy={refreshing} onRefresh={load} />
      <Button variant="primary" size="sm" onclick={() => (dialogOpen = true)}>
        <PlusIcon size={14} />
        {m.connection_add()}
      </Button>
    </div>
  </div>

  {#if !loading && connections.length > 0}
    <section class="summary panel">
      <Stat label={m.connection_active_requests()} value={totalActive} unit={`/ ${totalCapacity}`} />
      <Meter bare value={totalActive} limit={totalCapacity} ariaLabel={m.connection_active_requests()} />
    </section>
  {/if}

  {#if loading}
    <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
  {:else if connections.length === 0}
    <EmptyState title={m.connection_empty()} description={m.connection_empty_desc()} />
  {:else}
    <div class="conn-grid">
      {#each connections as conn (conn.id)}
        <article class="conn-card" data-status={conn.status}>
          <header class="conn-head">
            <div class="conn-title">
              <span class="mono conn-id" title={conn.id}>{conn.id}</span>
              <span class="conn-provider">{conn.provider}</span>
            </div>
            <Badge status={conn.status} label={connectionStatusLabel(conn.status)} />
          </header>

          <div class="conn-section">
            <span class="conn-label">{m.connection_active_requests()}</span>
            <Meter
              value={conn.active_requests}
              limit={conn.max_concurrent}
              valueText={`${conn.active_requests} / ${conn.max_concurrent}`}
            />
          </div>

          <div class="conn-section">
            <span class="conn-label">{m.connection_models()}</span>
            {#if conn.models.length === 0}
              <span class="hint">{m.common_none()}</span>
            {:else}
              <div class="model-chips" title={conn.models.join('\n')}>
                {#each conn.models.slice(0, MODEL_CHIPS) as id (id)}
                  <span class="model-chip" class:pattern={isPattern(id)}>{id}</span>
                {/each}
                {#if conn.models.length > MODEL_CHIPS}
                  <button type="button" class="models-more" onclick={() => showAllModels(conn)}>
                    {m.connection_models_more({ n: conn.models.length - MODEL_CHIPS })}
                  </button>
                {/if}
              </div>
            {/if}
          </div>

          <div class="conn-section">
            <span class="conn-label">{m.connection_cooldown()}</span>
            <div class="cooldown-info">
              {#if conn.cooldown_until}
                <div class="cooldown-until">{m.connection_cooldown_until({ time: formatRelativeFrom(conn.cooldown_until, m.common_none()) })}</div>
              {/if}
              {#if conn.failure_count != null}
                <div class="hint">{m.connection_failures({ n: conn.failure_count })}</div>
              {/if}
              {#if !conn.cooldown_until && conn.failure_count == null}
                <span class="hint">{m.common_none()}</span>
              {/if}
            </div>
          </div>

          <footer class="conn-actions">
            <span class="sr-only">{m.connection_actions()}</span>
            <Button
              variant="outline"
              size="sm"
              onclick={() => syncModels(conn)}
              disabled={syncingId === conn.id}
              ariaLabel={`${m.connection_sync_models()} ${conn.id}`}
            >
              <RefreshCwIcon size={14} aria-hidden="true" />
              {m.connection_sync_models()}
            </Button>
            {#if isCooling(conn)}
              <Button
                variant="outline"
                size="sm"
                onclick={() => resetCooldown(conn)}
                disabled={resettingId === conn.id}
                ariaLabel={`${m.connection_reset_cooldown()} ${conn.id}`}
              >
                <RefreshCwIcon size={14} aria-hidden="true" />
                {m.connection_reset_cooldown()}
              </Button>
            {/if}
            <Button
              variant="danger"
              size="sm"
              onclick={() => (pendingDelete = conn)}
              ariaLabel={`${m.connection_delete()} ${conn.id}`}
            >{m.connection_delete()}</Button>
          </footer>
        </article>
      {/each}
    </div>
  {/if}
</div>

<ConfirmDeleteDialog
  open={pendingDelete !== null}
  onClose={() => (pendingDelete = null)}
  title={m.connection_delete_title()}
  description={m.connection_delete_confirm({ id: pendingDelete?.id ?? '' })}
  confirmLabel={m.connection_delete()}
  busy={deleting}
  onConfirm={confirmDelete}
/>

<AddConnectionDialog bind:open={dialogOpen} />
<ConnectionModelsDialog
  bind:open={modelsDialogOpen}
  connectionId={modelsDialogConn?.id ?? ''}
  models={modelsDialogConn?.models ?? []}
/>


<style>
  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-sm);
    padding: var(--space-6) 0;
  }


  .refresh-note {
    margin: var(--space-1) 0 0;
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  /* Fleet summary: hero metric + meter */
  .summary {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(var(--col-md), 1fr));
    align-items: center;
    gap: var(--space-5);
    padding: var(--space-5);
    margin-bottom: var(--space-5);
  }
  .summary :global(.stat-value) {
    color: var(--accent);
  }

  /* Connection cards */
  .conn-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--col-xl)), 1fr));
    gap: var(--space-4);
  }

  .conn-card {
    --tone: var(--border-strong);
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
    padding: var(--space-5);
    background:
      linear-gradient(180deg, color-mix(in oklch, var(--tone) 7%, transparent), transparent 5rem), /* token-ok */
      var(--bg-surface);
    border: var(--border-w) solid color-mix(in oklch, var(--tone) 35%, var(--border));
    border-left: var(--indicator-w) solid var(--tone); /* token-ok */
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-1);
  }
  .conn-card[data-status='healthy'] { --tone: var(--success); }
  .conn-card[data-status='degraded'] { --tone: var(--warning); }
  .conn-card[data-status='circuit_open'] { --tone: var(--danger); }
  .conn-card[data-status='cooldown'] { --tone: var(--cooldown); }

  .conn-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .conn-title {
    display: flex;
    flex-direction: column;
    gap: var(--space-0);
    min-width: 0;
    flex: 1 1 var(--col-sm);
  }
  .conn-id {
    color: var(--text-1);
    font-weight: var(--weight-semibold);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .conn-provider {
    color: var(--text-3);
    font-size: var(--text-sm);
  }

  .conn-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .conn-label {
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    color: var(--text-3);
  }

  .cooldown-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-0);
    font-size: var(--text-sm);
    color: var(--text-1);
  }
  .cooldown-until { color: var(--tone); font-weight: var(--weight-medium); }
  .conn-card[data-status='healthy'] .cooldown-until { color: var(--text-1); }

  .hint {
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  .models-more {
    font: inherit;
    font-size: var(--text-xs);
    color: var(--accent);
    background: none;
    border: 0;
    padding: var(--space-0) var(--space-1);
    border-radius: var(--radius);
    cursor: pointer;
  }
  .models-more:hover { text-decoration: underline; background: var(--bg-hover); }

  .model-chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }
  .model-chip {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    padding: var(--space-0) var(--space-2);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-full);
    background: var(--bg-inset);
    color: var(--text-1);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .model-chip.pattern {
    border-style: dashed;
    color: var(--text-2);
  }

  .conn-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: auto;
    padding-top: var(--space-4);
    border-top: var(--border-w) solid var(--border);
  }
  .conn-actions :global(button) { min-height: var(--control-h-sm); }
  .conn-actions :global(button:last-child) { margin-left: auto; }

</style>
