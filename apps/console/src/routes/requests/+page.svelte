<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api.js';
  import type { RequestStatusFilter, RequestSummary } from '$lib/api.js';
  import { Badge, Button, EmptyState, Spinner } from '$lib/components/index.js';
  import { formatNumber, formatDateTime, formatTime } from '$lib/format.js';
  import { m } from '$lib/paraglide/messages.js';
  import { Dialog } from 'bits-ui';
  import { PauseIcon, PlayIcon, SearchIcon, XIcon } from 'lucide-svelte';
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

  /** Localized usage; each absent metric stays distinct from a reported numeric zero. */
  function fmtTokens(r: RequestSummary): string {
    return m.request_tokens_value({
      input: r.input_tokens == null ? m.request_metric_unavailable() : formatNumber(r.input_tokens),
      output: r.output_tokens == null ? m.request_metric_unavailable() : formatNumber(r.output_tokens),
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
    <div>
      <h1>{m.nav_requests()}</h1>
      <p class="refresh-note" aria-live="polite">
        <span class="live-dot" class:paused></span>
        {paused ? m.request_paused() : m.request_live()}
      </p>
    </div>
    <div class="page-actions">
      <Button
        variant="outline"
        size="md"
        onclick={() => (paused = !paused)}
        ariaLabel={paused ? m.request_resume_label() : m.request_pause_label()}
      >
        {#if paused}<PlayIcon size={14} aria-hidden="true" />{m.request_resume()}{:else}<PauseIcon size={14} aria-hidden="true" />{m.request_pause()}{/if}
      </Button>
    </div>
  </div>

  <div class="toolbar">
    <div class="segmented" role="group" aria-label={m.request_filter()}>
      {#each filterOptions as opt (opt.value)}
        <button
          type="button"
          aria-pressed={statusFilter === opt.value}
          onclick={() => onFilterChange(opt.value)}
        >
          {opt.label}
        </button>
      {/each}
    </div>

    <form onsubmit={search} class="search-form" aria-labelledby="search-heading">
      <h2 id="search-heading" class="sr-only">{m.request_search()}</h2>
      <label class="search-field">
        <span class="sr-only">{m.request_id_label()}</span>
        <span class="search-icon" aria-hidden="true"><SearchIcon size={14} /></span>
        <input type="text" bind:value={searchId} placeholder="req_..." aria-label={m.request_id_label()} />
      </label>
      <Button type="submit" size="md">{m.request_search_button()}</Button>
    </form>
  </div>
  {#if searchError}
    <p class="error-msg" role="alert">{searchError}</p>
  {/if}

  <section class="panel" aria-labelledby="recent-heading">
    <div class="panel-head">
      <h2 id="recent-heading">{m.request_recent()}</h2>
      <span class="count mono">{requests.length}</span>
    </div>
    {#if loading}
      <div class="panel-body">
        <div class="loading"><Spinner size="sm" /> {m.common_loading()}</div>
      </div>
    {:else if requests.length === 0}
      <div class="panel-body">
        <EmptyState title={m.request_empty()} description={m.request_empty_desc()} />
      </div>
    {:else}
      <div class="table-wrap">
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
                    {formatTime(r.started_at_ms)}
                  </button>
                </td>
                <td class="model-cell">{r.model}</td>
                <td class="mono">{r.api_type}</td>
                <td>
                  <div class="status-cell">
                    <Badge status={r.status} label={statusLabels[r.status]?.() ?? r.status} />
                    {#if r.stop_reason}
                      <span class="stop-badge stop-badge--{r.stop_reason}">{r.stop_reason}</span>
                    {/if}
                  </div>
                </td>
                <td class="mono">{r.connection_id ?? m.common_none()}</td>
                <td class="mono">{fmtDuration(r.duration_ms)}</td>
                <td class="mono">{fmtTokens(r)}</td>
                <td class="mono">{fmtCost(r.cost_microdollars)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
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
          <XIcon size={18} aria-hidden="true" />
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
            <dt>{m.request_started()}</dt><dd class="mono">{formatDateTime(d.started_at_ms)}</dd>
            <dt>{m.request_duration()}</dt><dd class="mono">{fmtDuration(d.duration_ms)}</dd>
            <dt>{m.request_tokens()}</dt><dd class="mono">{fmtTokens(d)}</dd>
            <dt>{m.request_cost()}</dt><dd class="mono">{fmtCost(d.cost_microdollars)}</dd>
          </dl>

          {#if d.stop_reason || d.error_message || d.thinking_requested != null || d.message_count != null || d.cache_read_tokens != null || d.cache_write_tokens != null || d.context_usage_pct != null}
            <h3 class="drawer-subtitle">{m.request_metadata()}</h3>
            <dl class="info-grid">
              {#if d.stop_reason}
                <dt>{m.request_stop_reason()}</dt>
                <dd><span class="stop-badge stop-badge--{d.stop_reason}">{d.stop_reason}</span></dd>
              {/if}
              {#if d.error_message}
                <dt>{m.request_error_message()}</dt>
                <dd class="mono error-text">{d.error_message}</dd>
              {/if}
              {#if d.thinking_requested != null}
                <dt>{m.request_thinking()}</dt>
                <dd>{d.thinking_requested ? 'yes' : 'no'}</dd>
              {/if}
              {#if d.message_count != null}
                <dt>{m.request_message_count()}</dt>
                <dd>{d.message_count}</dd>
              {/if}
              {#if d.cache_read_tokens != null}
                <dt>{m.request_cache_read()}</dt>
                <dd class="mono">{#if (d.cache_read_tokens ?? 0) > 0}<span class="cache-badge cache-badge--hit">{d.cache_read_tokens.toLocaleString()}</span>{:else}–{/if}</dd>
              {/if}
              {#if d.cache_write_tokens != null}
                <dt>{m.request_cache_write()}</dt>
                <dd class="mono">{d.cache_write_tokens.toLocaleString()}</dd>
              {/if}
              {#if d.context_usage_pct != null}
                {@const pct = d.context_usage_pct}
                <dt>Context usage</dt>
                <dd class="mono">
                  <span class="ctx-bar" title="{pct.toFixed(1)}% of context window">
                    <span class="ctx-bar__fill" style="width: {Math.min(pct, 100).toFixed(1)}%"></span>
                  </span>
                  {pct.toFixed(1)}%
                </dd>
              {/if}
            </dl>
          {/if}

          {#if d.state_transitions && d.state_transitions.length > 0}
            {@const transitions = d.state_transitions}
            <h3 class="drawer-subtitle">Pipeline timeline</h3>
            <ol class="timeline" aria-label="Pipeline phase timestamps">
              {#each transitions as [phase, ts], i (phase)}
                {@const prevTs = i === 0 ? null : transitions[i - 1][1]}
                {@const deltaMs = prevTs == null ? null : ts - prevTs}
                {@const isUpstreamOpen = phase === 'UpstreamOpen'}
                {@const isCommitted = phase === 'Committed'}
                {@const isTtfb = isCommitted && i > 0 && transitions.findIndex(([p]) => p === 'UpstreamOpen') !== -1}
                {@const upstreamOpenTs = isTtfb ? (transitions.find(([p]) => p === 'UpstreamOpen')?.[1] ?? null) : null}
                {@const ttfbMs = isTtfb && upstreamOpenTs != null ? ts - upstreamOpenTs : null}
                {@const isReceived = phase === 'Received'}
                {@const lastUpstreamOpenIdx = transitions.findLastIndex(([p]) => p === 'UpstreamOpen')}
                {@const gatewayMs = isUpstreamOpen && i > 0 ? ts - transitions[0][1] : null}
                <li class="timeline-item" class:timeline-ttfb={isTtfb} class:timeline-overhead={isUpstreamOpen}>
                  <span class="timeline-phase">{phase}</span>
                  <span class="timeline-ts mono">{new Date(ts).toISOString().slice(11, 23)}</span>
                  {#if deltaMs != null}
                    <span class="timeline-delta">+{deltaMs}ms</span>
                  {/if}
                  {#if gatewayMs != null}
                    <span class="timeline-label timeline-label--overhead">gateway overhead: {gatewayMs}ms</span>
                  {/if}
                  {#if ttfbMs != null}
                    <span class="timeline-label timeline-label--ttfb">TTFB: {ttfbMs}ms</span>
                  {/if}
                </li>
              {/each}
            </ol>
          {/if}


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
              <div class="table-wrap drawer-table">
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
              </div>
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
  .refresh-note {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-3);
    margin: var(--space-2) 0 0;
  }

  .live-dot {
    width: var(--dot-size);
    height: var(--dot-size);
    border-radius: var(--radius-full);
    background: var(--success);
    box-shadow: 0 0 0 var(--space-1) color-mix(in oklch, var(--success) 25%, transparent);
    animation: pulse var(--dur-pulse) ease-in-out infinite;
  }

  .live-dot.paused {
    background: var(--text-3);
    box-shadow: none;
    animation: none;
  }

  @keyframes pulse {
    50% { box-shadow: 0 0 0 var(--space-2) color-mix(in oklch, var(--success) 8%, transparent); }
  }

  @media (prefers-reduced-motion: reduce) {
    .live-dot { animation: none; }
  }

  /* Toolbar: segmented status filter + id search */
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  .search-form {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1 1 var(--dialog-w);
    justify-content: flex-end;
    min-width: 0;
  }

  .search-field {
    position: relative;
    flex: 1 1 auto;
    max-width: var(--dialog-w);
    min-width: 0;
  }

  .search-field input {
    padding-left: var(--space-6);
    min-height: var(--control-h-sm);
    border-radius: var(--radius-full);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .search-icon {
    position: absolute;
    left: var(--space-3);
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-3);
    display: flex;
    pointer-events: none;
  }

  .search-form :global(.btn) { min-height: var(--control-h-sm); }

  @media (max-width: 640px) {
    .toolbar { flex-direction: column; align-items: stretch; }
    .search-form { flex: 1 1 auto; justify-content: stretch; }
    .search-field { max-width: none; }
  }

  .count {
    font-size: var(--text-xs);
    color: var(--text-2);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius-full);
    padding: var(--space-0) var(--space-3);
  }

  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-sm);
    padding: var(--space-3) 0;
  }

  /* Table */
  .panel table { min-width: calc(var(--space-8) * 11); }
  .panel th { background: var(--bg-inset); }

  .clickable { cursor: pointer; }
  .model-cell { color: var(--text-1); font-weight: var(--weight-medium); white-space: nowrap; }

  .row-link {
    background: none;
    border: none;
    padding: var(--space-1) 0;
    min-height: var(--control-h-sm);
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    white-space: nowrap;
  }

  .row-link:hover { color: var(--accent-hover); text-decoration: underline; }

  .status-cell {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .stop-badge,
  .cache-badge,
  .timeline-label {
    display: inline-block;
    font-size: var(--text-2xs);
    font-weight: var(--weight-medium);
    padding: var(--space-0) var(--space-2);
    border-radius: var(--radius-full);
    white-space: nowrap;
  }

  .stop-badge {
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    color: var(--text-2);
  }

  .stop-badge--end_turn,
  .cache-badge--hit {
    background: var(--success-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--success) 28%, transparent);
    color: var(--success);
  }

  .stop-badge--max_tokens,
  .timeline-label--overhead {
    background: var(--warning-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--warning) 28%, transparent);
    color: var(--warning);
  }

  .stop-badge--tool_use,
  .timeline-label--ttfb {
    background: var(--accent-subtle);
    border: var(--border-w) solid color-mix(in oklch, var(--accent) 28%, transparent);
    color: var(--accent);
  }

  /* Drawer content (chrome comes from global .drawer-*) */
  .drawer-subtitle {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text-1);
    margin: var(--space-6) 0 var(--space-3);
  }

  .drawer-subtitle.small {
    font-size: var(--text-xs);
    color: var(--text-2);
    margin-top: var(--space-4);
  }

  .info-grid {
    display: grid;
    grid-template-columns: minmax(calc(var(--space-7) * 2), max-content) minmax(0, 1fr);
    gap: 0;
    font-size: var(--text-sm);
    margin: 0;
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .info-grid dt,
  .info-grid dd {
    padding: var(--space-2) var(--space-3);
    border-bottom: var(--border-w) solid var(--border);
    display: flex;
    align-items: center;
    flex-wrap: wrap;
  }

  .info-grid dt:nth-last-of-type(1),
  .info-grid dd:nth-last-of-type(1) { border-bottom: 0; }

  dt {
    color: var(--text-3);
    font-weight: var(--weight-medium);
  }

  dd {
    margin: 0;
    color: var(--text-1);
    overflow-wrap: anywhere;
  }

  @media (max-width: 420px) {
    .info-grid { grid-template-columns: minmax(0, 1fr); }
    .info-grid dt { padding-bottom: 0; border-bottom: 0; font-size: var(--text-xs); }
    .info-grid dd { padding-top: var(--space-0); }
  }

  .hint {
    font-size: var(--text-sm);
    color: var(--text-3);
  }

  .drawer-table {
    border: var(--border-w) solid var(--border);
    border-radius: var(--radius);
  }

  .drawer-table table { min-width: 0; }

  .ctx-bar {
    display: inline-block;
    vertical-align: middle;
    width: calc(var(--space-8) * 1.5);
    height: var(--meter-h);
    background: var(--border);
    border-radius: var(--radius-full);
    overflow: hidden;
    margin-right: var(--space-2);
  }

  .ctx-bar__fill {
    display: block;
    height: 100%;
    background: var(--accent);
    border-radius: var(--radius-full);
    transition: width var(--dur-3) var(--ease-out);
  }

  .error-text {
    color: var(--danger);
    font-size: var(--text-xs);
    word-break: break-all;
  }

  .timeline {
    list-style: none;
    padding: 0 0 0 var(--space-4);
    margin: 0;
    border-left: var(--focus-w) solid var(--border-strong);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .timeline-item {
    position: relative;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-2);
  }

  .timeline-item::before {
    content: '';
    position: absolute;
    left: calc(-1 * var(--space-4) - var(--dot-size) / 2 - var(--focus-w) / 2);
    top: 50%;
    width: var(--dot-size);
    height: var(--dot-size);
    margin-top: calc(-0.5 * var(--dot-size));
    border-radius: var(--radius-full);
    background: var(--bg-surface);
    border: var(--focus-w) solid var(--border-strong);
  }

  .timeline-item.timeline-ttfb,
  .timeline-item.timeline-overhead {
    color: var(--text-1);
    font-weight: var(--weight-medium);
  }

  .timeline-item.timeline-ttfb::before,
  .timeline-item.timeline-overhead::before {
    border-color: var(--accent);
    background: var(--accent);
  }

  .timeline-phase {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-2);
    min-width: calc(var(--space-8) * 1.5);
  }

  .timeline-ts {
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  .timeline-delta {
    font-size: var(--text-xs);
    background: var(--bg-inset);
    border: var(--border-w) solid var(--border);
    color: var(--text-2);
    padding: 0 var(--space-1);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
  }

  .timeline-label { font-weight: var(--weight-semibold); }
</style>
