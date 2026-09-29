<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { RequestStatusFilter, RequestSummary } from '$lib/api.js';
  import { Badge, Button, EmptyState, Select, Spinner } from '$lib/components/index.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Dialog } from 'bits-ui';
  import { PauseIcon, PlayIcon, XIcon } from 'lucide-svelte';
  import { toast } from 'svelte-sonner';

  let requests = $state<RequestSummary[]>([]);
  let loading = $state(true);
  let paused = $state(false);
  let statusFilter = $state<RequestStatusFilter>('all');

  let searchId = $state('');
  let searchError = $state('');
  let detailOpen = $state(false);
  let detailLoading = $state(false);
  let detailError = $state('');
  let detail = $state<RequestSummary | null>(null);

  const filterOptions = $derived([
    { value: 'all', label: m.request_filter_all() },
    { value: 'completed', label: m.request_status_completed() },
    { value: 'cancelled', label: m.request_status_cancelled() },
    { value: 'failed', label: m.request_status_failed() },
  ]);

  const statusLabels: Record<string, () => string> = {
    completed: m.request_status_completed,
    failed: m.request_status_failed,
    partial: m.request_status_partial,
    cancelled: m.request_status_cancelled,
    pending: m.request_status_pending,
  };

  const timeFmt = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });

  /** Localized usage; each absent metric stays distinct from a reported numeric zero. */
  function fmtTokens(r: RequestSummary): string {
    return m.request_tokens_value({
      input: r.input_tokens == null ? m.request_metric_unavailable() : r.input_tokens.toLocaleString(),
      output: r.output_tokens == null ? m.request_metric_unavailable() : r.output_tokens.toLocaleString(),
    });
  }

  /** USD from microdollars; an explicit localized empty value when no price was reported. */
  function fmtCost(micro: number | null | undefined): string {
    if (micro == null) return m.request_metric_unavailable();
    const usd = micro / 1_000_000;
    return `$${usd < 0.01 ? usd.toFixed(6) : usd.toFixed(4)}`;
  }

  function fmtDuration(ms: number | null | undefined): string {
    if (ms == null) return m.common_pending();
    return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(2)}s`;
  }

  async function load() {
    try {
      requests = (await api.listRequests(50, statusFilter)).items;
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  // Poll every 3s while the tab is visible and not paused; re-runs when `paused` flips.
  $effect(() => {
    const isPaused = paused;
    let timer: ReturnType<typeof setInterval> | undefined;
    const sync = () => {
      clearInterval(timer);
      timer = undefined;
      if (document.visibilityState === 'visible' && !isPaused) timer = setInterval(load, 3000);
    };
    sync();
    document.addEventListener('visibilitychange', sync);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', sync);
    };
  });

  function onFilterChange(v: string) {
    statusFilter = v as RequestStatusFilter;
    loading = true;
    load();
  }

  async function openDetail(id: string) {
    detailOpen = true;
    detailLoading = true;
    detailError = '';
    detail = null;
    try {
      detail = await api.getRequest(id);
    } catch (err) {
      detailError = (err as Error).message;
    } finally {
      detailLoading = false;
    }
  }

  function search(e: Event) {
    e.preventDefault();
    if (!searchId.trim()) { searchError = m.request_id_required(); return; }
    searchError = '';
    openDetail(searchId.trim());
  }
</script>

<div class="page">
  <div class="page-header">
    <h1 class="page-title">{m.nav_requests()}</h1>
    <div class="header-actions">
      <div class="filter">
        <Select label={m.request_filter()} options={filterOptions} value={statusFilter} onchange={onFilterChange} />
      </div>
      <Button
        variant="outline"
        size="sm"
        onclick={() => (paused = !paused)}
        ariaLabel={paused ? m.request_resume_label() : m.request_pause_label()}
      >
        {#if paused}<PlayIcon size={14} aria-hidden="true" />{m.request_resume()}{:else}<PauseIcon size={14} aria-hidden="true" />{m.request_pause()}{/if}
      </Button>
    </div>
  </div>

  <p class="refresh-note" aria-live="polite">{paused ? m.request_paused() : m.request_live()}</p>

  <section aria-labelledby="search-heading">
    <h2 id="search-heading">{m.request_search()}</h2>
    <form onsubmit={search} class="search-form">
      <label>
        {m.request_id_label()}
        <input type="text" bind:value={searchId} placeholder="req_..." />
      </label>
      <Button type="submit" size="md">{m.request_search_button()}</Button>
    </form>
    {#if searchError}
      <p class="error-msg" role="alert">{searchError}</p>
    {/if}
  </section>

  <section aria-labelledby="recent-heading">
    <h2 id="recent-heading">{m.request_recent()} ({requests.length})</h2>
    {#if loading}
      <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
    {:else if requests.length === 0}
      <EmptyState title={m.request_empty()} description={m.request_empty_desc()} />
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">{m.request_time()}</th>
            <th scope="col">{m.request_model()}</th>
            <th scope="col">{m.request_api_type()}</th>
            <th scope="col">{m.request_status()}</th>
            <th scope="col">{m.request_connection()}</th>
            <th scope="col">{m.request_duration()}</th>
            <th scope="col">{m.request_tokens()}</th>
            <th scope="col">{m.request_cost()}</th>
          </tr>
        </thead>
        <tbody>
          {#each requests as r (r.request_id)}
            <tr class="clickable" onclick={() => openDetail(r.request_id)}>
              <td class="mono">
                <!-- The button makes the row reachable by keyboard; the row click is a mouse shortcut. -->
                <button
                  type="button"
                  class="row-link"
                  aria-label={m.request_open_detail({ id: r.request_id })}
                  onclick={(e) => { e.stopPropagation(); openDetail(r.request_id); }}
                >
                  {timeFmt.format(new Date(r.started_at_ms))}
                </button>
              </td>
              <td>{r.model}</td>
              <td class="mono">{r.api_type}</td>
              <td><Badge status={r.status} label={statusLabels[r.status]?.() ?? r.status} /></td>
              <td class="mono">{r.connection_id ?? m.common_none()}</td>
              <td class="mono">{fmtDuration(r.duration_ms)}</td>
              <td class="mono">{fmtTokens(r)}</td>
              <td class="mono">{fmtCost(r.cost_microdollars)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<Dialog.Root bind:open={detailOpen}>
  <Dialog.Portal>
    <Dialog.Overlay class="drawer-overlay" />
    <Dialog.Content class="drawer-content" aria-describedby={undefined}>
      <div class="drawer-header">
        <Dialog.Title class="drawer-title">{m.request_detail_title()}</Dialog.Title>
        <Dialog.Close class="drawer-close" aria-label={m.common_close()}>
          <XIcon size={16} aria-hidden="true" />
        </Dialog.Close>
      </div>

      <div class="drawer-body" aria-live="polite">
        {#if detailLoading}
          <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
        {:else if detailError}
          <p class="error-msg" role="alert">{detailError}</p>
        {:else if detail}
          {@const d = detail}
          <dl class="info-grid">
            <dt>{m.request_id_label()}</dt><dd class="mono">{d.request_id}</dd>
            <dt>{m.request_model()}</dt><dd>{d.model}</dd>
            <dt>{m.request_api_type()}</dt><dd class="mono">{d.api_type}</dd>
            <dt>{m.request_status()}</dt><dd><Badge status={d.status} label={statusLabels[d.status]?.() ?? d.status} /></dd>
            <dt>{m.request_connection()}</dt><dd class="mono">{d.connection_id ?? m.common_none()}</dd>
            <dt>{m.request_started()}</dt><dd class="mono">{new Date(d.started_at_ms).toLocaleString()}</dd>
            <dt>{m.request_duration()}</dt><dd class="mono">{fmtDuration(d.duration_ms)}</dd>
            <dt>{m.request_tokens()}</dt><dd class="mono">{fmtTokens(d)}</dd>
            <dt>{m.request_cost()}</dt><dd class="mono">{fmtCost(d.cost_microdollars)}</dd>
          </dl>

          <h3 class="drawer-subtitle">{m.request_decision()}</h3>
          {#if d.decision}
            <dl class="info-grid">
              <dt>{m.request_route()}</dt><dd class="mono">{d.decision.route_id ?? m.common_none()}</dd>
              <dt>{m.request_attempts()}</dt><dd>{d.decision.attempt_count}</dd>
            </dl>
            <h4 class="drawer-subtitle small">{m.request_excluded()}</h4>
            {#if d.decision.candidates_excluded.length === 0}
              <p class="hint">{m.request_excluded_none()}</p>
            {:else}
              <table>
                <thead>
                  <tr>
                    <th scope="col">{m.request_connection()}</th>
                    <th scope="col">{m.request_reason()}</th>
                  </tr>
                </thead>
                <tbody>
                  {#each d.decision.candidates_excluded as ex (ex.id)}
                    <tr><td class="mono">{ex.id}</td><td>{ex.reason}</td></tr>
                  {/each}
                </tbody>
              </table>
            {/if}
          {:else}
            <p class="hint">{m.request_no_decision()}</p>
          {/if}
        {/if}
      </div>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  .page-header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
    margin-bottom: 0.5rem;
  }

  .page-header h1 {
    margin: 0;
  }

  .header-actions {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }

  .filter {
    min-width: 160px;
  }

  .refresh-note {
    font-size: 0.75rem;
    color: var(--text-3);
    margin-bottom: 1.5rem;
  }

  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 0.875rem;
    padding: 16px 0;
  }

  .search-form {
    display: flex;
    gap: 0.75rem;
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .search-form input {
    min-width: 280px;
  }

  .clickable {
    cursor: pointer;
  }

  .row-link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    white-space: nowrap;
  }

  .row-link:hover {
    text-decoration: underline;
  }

  :global(.drawer-overlay) {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 50;
  }

  :global(.drawer-content) {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 51;
    width: min(520px, 100vw);
    background: var(--bg-surface);
    border-left: 1px solid var(--border);
    box-shadow: -8px 0 32px rgba(0, 0, 0, 0.25);
    overflow-y: auto;
  }

  :global(.drawer-title) {
    font-size: 1rem;
    font-weight: 600;
    color: var(--text-1);
    margin: 0;
  }

  :global(.drawer-close) {
    display: flex;
    background: transparent;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm);
  }

  :global(.drawer-close:hover) {
    color: var(--text-1);
    background: var(--bg-hover);
  }

  .drawer-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px;
    border-bottom: 1px solid var(--border);
  }

  .drawer-body {
    padding: 20px;
  }

  .drawer-subtitle {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--text-1);
    margin: 1.5rem 0 0.75rem;
  }

  .drawer-subtitle.small {
    font-size: 0.8125rem;
    margin-top: 1rem;
  }

  .info-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.375rem 1.25rem;
    font-size: 0.875rem;
    margin: 0;
  }

  dt {
    color: var(--text-3);
    font-weight: 500;
  }

  dd {
    margin: 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }

  .hint {
    font-size: 0.8125rem;
    color: var(--text-3);
  }
</style>
